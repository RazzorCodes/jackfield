// GrpcEndpoint: tonic server with Register/Stream/Deregister RPCs.
// Clients self-register declaring role (PRODUCER/CONSUMER) and optional dimension filters.
// TCP remote_addr is used to correlate Register → Stream → Deregister without overhead.
use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tonic::{async_trait, Request, Response, Status, Streaming};

use crate::components::bus::bus::BusCmdHandle;
use crate::components::message::codec::proto::{
    self,
    bus_server::{Bus as BusTrait, BusServer},
};
use crate::components::router::envelope::ProducerHandle;
use crate::components::endpoint::network::connection::{ChannelConsumer, parse_dimensions};
use crate::components::message::Message;

struct EndpointEntry {
    uuid: uuid::Uuid,
    consumer_reg_id: Option<u64>,
    consumer_rx: Option<mpsc::Receiver<proto::BusableItem>>,
    producer_handle: Option<ProducerHandle>,
}

type EndpointMap = std::sync::Arc<tokio::sync::Mutex<HashMap<SocketAddr, EndpointEntry>>>;

pub struct GrpcEndpoint {
    name: String,
    addr: SocketAddr,
    cmd_handle: BusCmdHandle,
}

impl GrpcEndpoint {
    pub fn new(name: impl Into<String>, addr: SocketAddr, cmd_handle: BusCmdHandle) -> Self {
        GrpcEndpoint { name: name.into(), addr, cmd_handle }
    }

    pub fn start(&self) -> JoinHandle<()> {
        let service = JackfieldGrpcService {
            name: self.name.clone(),
            cmd_handle: self.cmd_handle.clone(),
            endpoints: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        };
        let addr = self.addr;
        tokio::spawn(async move {
            let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
            health_reporter.set_serving::<BusServer<JackfieldGrpcService>>().await;
            tonic::transport::Server::builder()
                .add_service(health_service)
                .add_service(BusServer::new(service))
                .serve(addr)
                .await
                .ok();
        })
    }
}

struct JackfieldGrpcService {
    name: String,
    cmd_handle: BusCmdHandle,
    endpoints: EndpointMap,
}

#[async_trait]
impl BusTrait for JackfieldGrpcService {
    type StreamStream = tokio_stream::wrappers::ReceiverStream<Result<proto::BusableItem, Status>>;

    async fn register(
        &self,
        request: Request<proto::RegisterData>,
    ) -> Result<Response<proto::GenericResponse>, Status> {
        let addr = request.remote_addr()
            .ok_or_else(|| Status::internal("no remote address"))?;
        let data = request.into_inner();

        let endpoint_uuid = uuid::Uuid::new_v4();
        let is_consumer = data.r#type.contains(&(proto::EndpointType::Consumer as i32));
        let is_producer = data.r#type.contains(&(proto::EndpointType::Producer as i32));

        let dims = parse_dimensions(&data.dimensions_json)?;

        let mut entry = EndpointEntry {
            uuid: endpoint_uuid,
            consumer_reg_id: None,
            consumer_rx: None,
            producer_handle: None,
        };

        if is_consumer {
            let (tx, rx) = mpsc::channel::<proto::BusableItem>(256);
            let consumer = Box::new(ChannelConsumer { tx });
            let id = self.cmd_handle.register(consumer, dims).await
                .map_err(|_| Status::internal("bus registration failed"))?;
            entry.consumer_reg_id = Some(id);
            entry.consumer_rx = Some(rx);
        }

        if is_producer {
            entry.producer_handle = Some(
                self.cmd_handle.make_producer_handle(format!("{}/{}", self.name, endpoint_uuid))
            );
        }

        self.endpoints.lock().await.insert(addr, entry);

        Ok(Response::new(proto::GenericResponse {
            status: proto::GenericStatus::Success as i32,
            data: Some(proto::generic_response::Data::Uuid(endpoint_uuid.as_bytes().to_vec())),
        }))
    }

    async fn stream(
        &self,
        request: Request<Streaming<proto::BusableItem>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        let addr = request.remote_addr()
            .ok_or_else(|| Status::internal("no remote address"))?;

        let (consumer_rx, consumer_reg_id, producer_handle) = {
            let mut endpoints = self.endpoints.lock().await;
            let entry = endpoints.get_mut(&addr)
                .ok_or_else(|| Status::failed_precondition("not registered; call Register first"))?;
            (entry.consumer_rx.take(), entry.consumer_reg_id, entry.producer_handle.take())
        };

        let (outbound_tx, outbound_rx) = mpsc::channel::<Result<proto::BusableItem, Status>>(256);

        let has_consumer = consumer_rx.is_some();

        if let Some(mut rx) = consumer_rx {
            let tx = outbound_tx.clone();
            let cmd_handle = self.cmd_handle.clone();
            let endpoints = self.endpoints.clone();
            tokio::spawn(async move {
                while let Some(item) = rx.recv().await {
                    if tx.send(Ok(item)).await.is_err() { break; }
                }
                // Stream broke before Deregister was called — clean up.
                if let Some(id) = consumer_reg_id {
                    cmd_handle.deregister(id).await;
                }
                endpoints.lock().await.remove(&addr);
            });
        }

        if let Some(handle) = producer_handle {
            let mut inbound = request.into_inner();
            // Producer-only endpoints own their own cleanup; BOTH endpoints let the consumer task handle it.
            let cleanup = if has_consumer { None } else { Some(self.endpoints.clone()) };
            tokio::spawn(async move {
                while let Ok(Some(item)) = inbound.message().await {
                    let msg: Box<dyn Message> = item.into();
                    let _ = handle.make_send(msg).await;
                }
                if let Some(endpoints) = cleanup {
                    endpoints.lock().await.remove(&addr);
                }
            });
        }

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(outbound_rx)))
    }

    async fn deregister(
        &self,
        request: Request<proto::DeregisterNotification>,
    ) -> Result<Response<proto::Empty>, Status> {
        let addr = request.remote_addr()
            .ok_or_else(|| Status::internal("no remote address"))?;
        let data = request.into_inner();

        let uuid = uuid::Uuid::from_slice(&data.uuid)
            .map_err(|_| Status::invalid_argument("invalid uuid"))?;

        let consumer_reg_id = {
            let mut endpoints = self.endpoints.lock().await;
            match endpoints.entry(addr) {
                std::collections::hash_map::Entry::Occupied(e) if e.get().uuid == uuid => {
                    e.remove().consumer_reg_id
                }
                std::collections::hash_map::Entry::Occupied(_) => {
                    return Err(Status::not_found("uuid mismatch"));
                }
                std::collections::hash_map::Entry::Vacant(_) => {
                    return Err(Status::not_found("not registered"));
                }
            }
        };

        if let Some(id) = consumer_reg_id {
            self.cmd_handle.deregister(id).await;
        }

        Ok(Response::new(proto::Empty {}))
    }
}

