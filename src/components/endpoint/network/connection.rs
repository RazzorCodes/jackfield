// ConnectionRegistry: shared connection table (connect/disconnect/broadcast) — kept for backwards compatibility.
// ChannelConsumer + parse_dimensions: shared between gRPC and WebSocket endpoints.
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};

#[derive(Clone)]
pub struct ConnectionRegistry<T> {
    connections: Arc<Mutex<HashMap<u64, mpsc::Sender<T>>>>,
    next_id: Arc<AtomicU64>,
    active: Arc<AtomicUsize>,
}

impl<T: Clone + Send + 'static> ConnectionRegistry<T> {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn available(&self) -> bool {
        self.active.load(Ordering::Relaxed) > 0
    }

    pub async fn connect(&self, tx: mpsc::Sender<T>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.connections.lock().await.insert(id, tx);
        self.active.fetch_add(1, Ordering::Relaxed);
        id
    }

    pub async fn disconnect(&self, id: u64) {
        if self.connections.lock().await.remove(&id).is_some() {
            self.active.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub async fn broadcast(&self, msg: T) {
        let senders: Vec<(u64, mpsc::Sender<T>)> = {
            let conns = self.connections.lock().await;
            conns.iter().map(|(id, tx)| (*id, tx.clone())).collect()
        };
        let mut dead = Vec::new();
        for (id, tx) in &senders {
            if tx.send(msg.clone()).await.is_err() {
                dead.push(*id);
            }
        }
        if !dead.is_empty() {
            let mut conns = self.connections.lock().await;
            for id in dead {
                if conns.remove(&id).is_some() {
                    self.active.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }
}

// ── Shared between gRPC and WebSocket ────────────────────────────────────────

#[cfg(any(feature = "grpc", feature = "websocket"))]
pub use network_shared::*;

#[cfg(any(feature = "grpc", feature = "websocket"))]
mod network_shared {
    use super::*;
    use crate::components::message::codec::proto;
    use crate::components::router::dimensions::{Dimension, LabelDim, ProducerDim, SizeDim};
    use crate::components::router::envelope::Envelope;
    use crate::components::message::Message;
    use tonic::Status;

    pub struct ChannelConsumer {
        pub tx: mpsc::Sender<proto::BusableItem>,
    }

    impl crate::components::endpoint::Consumer for ChannelConsumer {
        fn available(&self) -> bool {
            !self.tx.is_closed()
        }

        fn validate(&self, _: &Envelope) -> bool {
            true
        }

        fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            let tx = self.tx.clone();
            let item: proto::BusableItem = message.into();
            Box::pin(async move { tx.send(item).await.ok(); })
        }
    }

    pub fn parse_dimensions(json: &str) -> Result<Vec<(Box<dyn Dimension>, bool, f32)>, Status> {
        if json.is_empty() {
            return Ok(vec![]);
        }
        let v: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| Status::invalid_argument(format!("dimensions_json: {e}")))?;
        let mut dims: Vec<(Box<dyn Dimension>, bool, f32)> = Vec::new();
        if let Some(l) = v.get("labels") {
            let d = parse_label_dim(l)
                .ok_or_else(|| Status::invalid_argument("invalid labels dimension"))?;
            dims.push((Box::new(d), true, 1.0));
        }
        if let Some(p) = v.get("producer") {
            let d = parse_producer_dim(p)
                .ok_or_else(|| Status::invalid_argument("invalid producer dimension"))?;
            dims.push((Box::new(d), true, 1.0));
        }
        if let Some(s) = v.get("size") {
            let d = parse_size_dim(s)
                .ok_or_else(|| Status::invalid_argument("invalid size dimension"))?;
            dims.push((Box::new(d), true, 1.0));
        }
        Ok(dims)
    }

    fn str_vec(a: &serde_json::Value) -> Vec<String> {
        a.as_array()
            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    fn parse_label_dim(v: &serde_json::Value) -> Option<LabelDim> {
        if let Some(a) = v.get("all_of") { return Some(LabelDim::all_of(str_vec(a))); }
        if let Some(a) = v.get("any_of") { return Some(LabelDim::any_of(str_vec(a))); }
        if let Some(a) = v.get("none_of") { return Some(LabelDim::none_of(str_vec(a))); }
        None
    }

    fn parse_producer_dim(v: &serde_json::Value) -> Option<ProducerDim> {
        if let Some(s) = v.get("only").and_then(|s| s.as_str()) { return Some(ProducerDim::only(s)); }
        if let Some(a) = v.get("any_of") { return Some(ProducerDim::any_of(str_vec(a))); }
        if let Some(a) = v.get("none_of") { return Some(ProducerDim::none_of(str_vec(a))); }
        None
    }

    fn parse_size_dim(v: &serde_json::Value) -> Option<SizeDim> {
        if let Some(n) = v.get("at_most").and_then(|n| n.as_u64()) {
            return Some(SizeDim::at_most(n as usize));
        }
        if let Some(n) = v.get("at_least").and_then(|n| n.as_u64()) {
            return Some(SizeDim::at_least(n as usize));
        }
        if let Some(arr) = v.get("between").and_then(|a| a.as_array()) {
            if arr.len() == 2 {
                let min = arr[0].as_u64()? as usize;
                let max = arr[1].as_u64()? as usize;
                return Some(SizeDim::between(min, max));
            }
        }
        None
    }
}
