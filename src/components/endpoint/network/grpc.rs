// GrpcEndpoint: tonic-based server. Inbound connections → bus via ProducerHandle. consumer() → GrpcConsumer for outbound broadcast.
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tonic::async_trait;
use tonic::transport::Server;
use tonic::{Request, Response, Status, Streaming};

use crate::components::message::codec::proto::{
    self,
    bus_server::{Bus as BusTrait, BusServer},
};
use crate::components::router::envelope::{Envelope, ProducerId, ProducerHandle};
use crate::components::bus::error::JackfieldError;
use crate::components::endpoint::{Consumer, Producer};
use crate::components::endpoint::network::ConnectionRegistry;
use crate::components::message::Message;

pub struct GrpcEndpoint {
    name: String,
    addr: SocketAddr,
    handle: Arc<Mutex<Option<ProducerHandle>>>,
    registry: ConnectionRegistry<proto::BusMessage>,
}

impl GrpcEndpoint {
    pub fn new(name: impl Into<String>, addr: SocketAddr) -> Self {
        GrpcEndpoint {
            name: name.into(),
            addr,
            handle: Arc::new(Mutex::new(None)),
            registry: ConnectionRegistry::new(),
        }
    }

    pub fn start(&self) -> JoinHandle<()> {
        let service = JackfieldGrpcService {
            name: self.name.clone(),
            handle: self.handle.clone(),
            registry: self.registry.clone(),
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

    pub fn consumer(&self) -> GrpcConsumer {
        GrpcConsumer { registry: self.registry.clone() }
    }
}

impl Producer for GrpcEndpoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn attach(&mut self, handle: ProducerHandle) {
        *self.handle.lock().unwrap() = Some(handle);
    }

    fn send_bus(&mut self, _msg: Box<dyn Message>) -> impl Future<Output = Result<(), JackfieldError>> + Send {
        async move { Err(JackfieldError::NotRegistered) }
    }
}

pub struct GrpcConsumer {
    registry: ConnectionRegistry<proto::BusMessage>,
}

impl Consumer for GrpcConsumer {
    fn available(&self) -> bool {
        self.registry.available()
    }

    fn validate(&self, _: &Envelope) -> bool {
        true
    }

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let bus_msg: proto::BusMessage = message.into();
        let registry = self.registry.clone();
        Box::pin(async move { registry.broadcast(bus_msg).await })
    }
}

struct JackfieldGrpcService {
    name: String,
    handle: Arc<Mutex<Option<ProducerHandle>>>,
    registry: ConnectionRegistry<proto::BusMessage>,
}

#[async_trait]
impl BusTrait for JackfieldGrpcService {
    type StreamStream = tokio_stream::wrappers::ReceiverStream<Result<proto::BusMessage, Status>>;

    async fn stream(
        &self,
        request: Request<Streaming<proto::BusMessage>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        let (bus_msg_tx, mut bus_msg_rx) = mpsc::channel::<proto::BusMessage>(256);
        let (outbound_tx, outbound_rx) = mpsc::channel::<Result<proto::BusMessage, Status>>(256);

        let conn_id = self.registry.connect(bus_msg_tx).await;

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
        let registry = self.registry.clone();
        let mut inbound = request.into_inner();
        tokio::spawn(async move {
            let producer_id = ProducerId(format!("{}/{}", name, conn_id));
            while let Ok(Some(proto_msg)) = inbound.message().await {
                let msg: Box<dyn Message> = proto_msg.into();
                let fut = handle.lock().unwrap().as_ref().map(|h| h.make_send_with_origin(producer_id.clone(), msg));
                if let Some(fut) = fut {
                    let _ = fut.await;
                }
            }
            registry.disconnect(conn_id).await;
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(outbound_rx)))
    }
}
