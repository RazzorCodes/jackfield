// WsEndpoint: tokio-tungstenite server with self-registering connections.
// First binary frame = proto::RegisterData (role + dimensions_json).
// Subsequent frames = proto::BusableItem. Disconnect = implicit deregister.
use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::components::bus::bus::BusCmdHandle;
use crate::components::message::codec::proto;
use crate::components::endpoint::network::connection::{ChannelConsumer, parse_dimensions};
use crate::components::message::Message;

pub struct WsEndpoint {
    name: String,
    addr: SocketAddr,
    cmd_handle: BusCmdHandle,
}

impl WsEndpoint {
    pub fn new(name: impl Into<String>, addr: SocketAddr, cmd_handle: BusCmdHandle) -> Self {
        WsEndpoint { name: name.into(), addr, cmd_handle }
    }

    pub fn start(&self) -> JoinHandle<()> {
        let name = self.name.clone();
        let cmd_handle = self.cmd_handle.clone();
        let addr = self.addr;
        tokio::spawn(async move {
            let listener = TcpListener::bind(addr).await.expect("ws bind failed");
            loop {
                let Ok((stream, _peer)) = listener.accept().await else { continue };
                let name = name.clone();
                let cmd_handle = cmd_handle.clone();
                tokio::spawn(async move {
                    handle_connection(name, cmd_handle, stream).await;
                });
            }
        })
    }
}

async fn handle_connection(
    _name: String,
    cmd_handle: BusCmdHandle,
    stream: tokio::net::TcpStream,
) {
    let ws = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(_) => return,
    };
    let (mut sink, mut source) = ws.split();

    // First binary frame must be RegisterData.
    let reg = loop {
        match source.next().await {
            Some(Ok(WsMessage::Binary(b))) => {
                match proto::RegisterData::decode(b.as_ref()) {
                    Ok(r) => break r,
                    Err(_) => return,
                }
            }
            Some(Ok(WsMessage::Close(_))) | None => return,
            _ => continue,
        }
    };

    let is_consumer = reg.r#type.contains(&(proto::EndpointType::Consumer as i32));
    let is_producer = reg.r#type.contains(&(proto::EndpointType::Producer as i32));

    let dims = match parse_dimensions(&reg.dimensions_json) {
        Ok(d) => d,
        Err(_) => return,
    };

    // Register as consumer: allocate channel, register ChannelConsumer in bus,
    // spawn task forwarding bus items → WS sink.
    let consumer_reg_id = if is_consumer {
        let (tx, mut rx) = mpsc::channel::<proto::BusableItem>(256);
        let consumer = Box::new(ChannelConsumer { tx });
        let id = match cmd_handle.register(consumer, dims).await {
            Ok(id) => id,
            Err(_) => return,
        };
        let sink_tx = {
            let (bytes_tx, mut bytes_rx) = mpsc::channel::<Vec<u8>>(256);
            tokio::spawn(async move {
                while let Some(b) = bytes_rx.recv().await {
                    if sink.send(WsMessage::Binary(b.into())).await.is_err() { break; }
                }
            });
            bytes_tx
        };
        tokio::spawn(async move {
            while let Some(item) = rx.recv().await {
                if sink_tx.send(item.encode_to_vec()).await.is_err() { break; }
            }
        });
        Some(id)
    } else {
        None
    };

    // Producer handle for inbound frames.
    let producer_handle = if is_producer {
        Some(cmd_handle.make_producer_handle(
            format!("{}/{}", reg.name, uuid::Uuid::new_v4())
        ))
    } else {
        None
    };

    // Main inbound loop — runs until the connection closes (clean or crash).
    while let Some(Ok(ws_msg)) = source.next().await {
        match ws_msg {
            WsMessage::Binary(b) => {
                if let Some(handle) = &producer_handle {
                    if let Ok(item) = proto::BusableItem::decode(b.as_ref()) {
                        let msg: Box<dyn Message> = item.into();
                        let _ = handle.make_send(msg).await;
                    }
                }
            }
            WsMessage::Close(_) => break,
            _ => continue,
        }
    }

    // Implicit deregister on any disconnect.
    if let Some(id) = consumer_reg_id {
        cmd_handle.deregister(id).await;
    }
}
