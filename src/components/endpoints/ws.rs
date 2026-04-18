use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::components::bus::codec::proto;
use crate::components::bus::envelope::{Envelope, ProducerId, ProducerHandle};
use crate::components::bus::error::JackfieldError;
use crate::components::endpoint::{Consumer, Producer};
use crate::components::message::Message;

type ConnectionId = u64;
type Connections = Arc<Mutex<HashMap<ConnectionId, mpsc::Sender<Vec<u8>>>>>;

pub struct WsEndpoint {
    name: String,
    addr: SocketAddr,
    handle: Option<ProducerHandle>,
    connections: Connections,
    next_id: Arc<AtomicU64>,
}

impl WsEndpoint {
    pub fn new(name: impl Into<String>, addr: SocketAddr) -> Self {
        WsEndpoint {
            name: name.into(),
            addr,
            handle: None,
            connections: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn start(self) -> JoinHandle<()> {
        let name = self.name.clone();
        let handle = Arc::new(Mutex::new(self.handle));
        let connections = self.connections.clone();
        let next_id = self.next_id.clone();
        let addr = self.addr;

        tokio::spawn(async move {
            let listener = TcpListener::bind(addr).await.expect("ws bind failed");
            loop {
                let Ok((stream, _peer)) = listener.accept().await else { continue };
                let conn_id = next_id.fetch_add(1, Ordering::Relaxed);
                let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(256);
                connections.lock().await.insert(conn_id, outbound_tx);

                let name = name.clone();
                let handle = handle.clone();
                let connections = connections.clone();

                tokio::spawn(async move {
                    let ws = match accept_async(stream).await {
                        Ok(ws) => ws,
                        Err(_) => {
                            connections.lock().await.remove(&conn_id);
                            return;
                        }
                    };
                    let (mut sink, mut source) = ws.split();

                    let sink_task = tokio::spawn(async move {
                        while let Some(bytes) = outbound_rx.recv().await {
                            if sink.send(WsMessage::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    });

                    let producer_id = ProducerId(format!("{}/{}", name, conn_id));
                    while let Some(Ok(ws_msg)) = source.next().await {
                        let bytes = match ws_msg {
                            WsMessage::Binary(b) => b,
                            WsMessage::Close(_) => break,
                            _ => continue,
                        };
                        let Ok(proto_msg) = proto::BusMessage::decode(bytes.as_ref()) else {
                            continue;
                        };
                        let msg: Box<dyn Message> = proto_msg.into();
                        let guard = handle.lock().await;
                        if let Some(h) = guard.as_ref() {
                            let fut = h.make_send_with_origin(producer_id.clone(), msg);
                            drop(guard);
                            let _ = fut.await;
                        }
                    }

                    sink_task.abort();
                    connections.lock().await.remove(&conn_id);
                });
            }
        })
    }
}

impl Producer for WsEndpoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn attach(&mut self, handle: ProducerHandle) {
        self.handle = Some(handle);
    }

    fn send_bus(&mut self, _msg: Box<dyn Message>) -> impl Future<Output = Result<(), JackfieldError>> + Send {
        async move { Err(JackfieldError::NotRegistered) }
    }
}

impl Consumer for WsEndpoint {
    fn available(&self) -> bool {
        true
    }

    fn validate(&self, _envelope: &Envelope) -> bool {
        true
    }

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let bytes = {
            let proto_msg: proto::BusMessage = message.into();
            proto_msg.encode_to_vec()
        };
        let connections = self.connections.clone();
        Box::pin(async move {
            let mut conns = connections.lock().await;
            let mut dead = Vec::new();
            for (id, tx) in conns.iter() {
                if tx.send(bytes.clone()).await.is_err() {
                    dead.push(*id);
                }
            }
            for id in dead {
                conns.remove(&id);
            }
        })
    }
}
