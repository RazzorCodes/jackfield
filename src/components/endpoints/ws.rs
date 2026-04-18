use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
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
use crate::components::endpoints::connection::ConnectionRegistry;
use crate::components::message::Message;

pub struct WsEndpoint {
    name: String,
    addr: SocketAddr,
    handle: Option<ProducerHandle>,
    registry: ConnectionRegistry<Vec<u8>>,
}

impl WsEndpoint {
    pub fn new(name: impl Into<String>, addr: SocketAddr) -> Self {
        WsEndpoint {
            name: name.into(),
            addr,
            handle: None,
            registry: ConnectionRegistry::new(),
        }
    }

    pub fn accept_labels(mut self, labels: Vec<String>) -> Self {
        self.registry = self.registry.accept_labels(labels);
        self
    }

    pub fn start(self) -> JoinHandle<()> {
        let name = self.name;
        let handle = Arc::new(Mutex::new(self.handle));
        let registry = self.registry;
        let addr = self.addr;

        tokio::spawn(async move {
            let listener = TcpListener::bind(addr).await.expect("ws bind failed");
            loop {
                let Ok((stream, _peer)) = listener.accept().await else { continue };

                let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(256);
                let conn_id = registry.connect(outbound_tx).await;

                let name = name.clone();
                let handle = handle.clone();
                let registry = registry.clone();

                tokio::spawn(async move {
                    let ws = match accept_async(stream).await {
                        Ok(ws) => ws,
                        Err(_) => {
                            registry.disconnect(conn_id).await;
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
                    registry.disconnect(conn_id).await;
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
        self.registry.available()
    }

    fn validate(&self, envelope: &Envelope) -> bool {
        self.registry.validates(envelope)
    }

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let bytes = {
            let proto_msg: proto::BusMessage = message.into();
            proto_msg.encode_to_vec()
        };
        let registry = self.registry.clone();
        Box::pin(async move { registry.broadcast(bytes).await })
    }
}
