use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tonic::async_trait;
use tonic::transport::Server;
use tonic::{Request, Response, Status, Streaming};

use crate::components::bus::codec::proto::{
    self,
    bus_server::{Bus as BusTrait, BusServer},
};
use crate::components::bus::envelope::{Envelope, ProducerId, ProducerHandle};
use crate::components::bus::error::JackfieldError;
use crate::components::endpoint::{Consumer, Producer};
use crate::components::message::Message;

type ConnectionId = u64;
type Connections = Arc<Mutex<HashMap<ConnectionId, mpsc::Sender<proto::BusMessage>>>>;

pub struct GrpcEndpoint {
    name: String,
    addr: SocketAddr,
    handle: Option<ProducerHandle>,
    connections: Connections,
    next_id: Arc<AtomicU64>,
}

impl GrpcEndpoint {
    pub fn new(name: impl Into<String>, addr: SocketAddr) -> Self {
        GrpcEndpoint {
            name: name.into(),
            addr,
            handle: None,
            connections: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn start(self) -> JoinHandle<()> {
        let service = JackfieldGrpcService {
            name: self.name.clone(),
            handle: Arc::new(Mutex::new(self.handle)),
            connections: self.connections.clone(),
            next_id: self.next_id.clone(),
        };
        let addr = self.addr;
        tokio::spawn(async move {
            Server::builder()
                .add_service(BusServer::new(service))
                .serve(addr)
                .await
                .ok();
        })
    }
}

impl Producer for GrpcEndpoint {
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

impl Consumer for GrpcEndpoint {
    fn available(&self) -> bool {
        true
    }

    fn validate(&self, _envelope: &Envelope) -> bool {
        true
    }

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let bus_msg: proto::BusMessage = message.into();
        let connections = self.connections.clone();
        Box::pin(async move {
            let mut conns = connections.lock().await;
            let mut dead = Vec::new();
            for (id, tx) in conns.iter() {
                if tx.send(bus_msg.clone()).await.is_err() {
                    dead.push(*id);
                }
            }
            for id in dead {
                conns.remove(&id);
            }
        })
    }
}

struct JackfieldGrpcService {
    name: String,
    handle: Arc<Mutex<Option<ProducerHandle>>>,
    connections: Connections,
    next_id: Arc<AtomicU64>,
}

#[async_trait]
impl BusTrait for JackfieldGrpcService {
    type StreamStream = tokio_stream::wrappers::ReceiverStream<Result<proto::BusMessage, Status>>;

    async fn stream(
        &self,
        request: Request<Streaming<proto::BusMessage>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        let conn_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (outbound_tx, outbound_rx) = mpsc::channel::<Result<proto::BusMessage, Status>>(256);

        let (bus_msg_tx, mut bus_msg_rx) = mpsc::channel::<proto::BusMessage>(256);
        self.connections.lock().await.insert(conn_id, bus_msg_tx);

        let outbound_tx_clone = outbound_tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = bus_msg_rx.recv().await {
                if outbound_tx_clone.send(Ok(msg)).await.is_err() {
                    break;
                }
            }
        });

        let name = self.name.clone();
        let handle = self.handle.clone();
        let connections = self.connections.clone();
        let mut inbound = request.into_inner();
        tokio::spawn(async move {
            let producer_id = ProducerId(format!("{}/{}", name, conn_id));
            while let Ok(Some(proto_msg)) = inbound.message().await {
                let msg: Box<dyn Message> = proto_msg.into();
                let guard = handle.lock().await;
                if let Some(h) = guard.as_ref() {
                    let fut = h.make_send_with_origin(producer_id.clone(), msg);
                    drop(guard);
                    let _ = fut.await;
                }
            }
            connections.lock().await.remove(&conn_id);
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(outbound_rx)))
    }
}
