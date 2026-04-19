use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};

use crate::components::bus::envelope::Envelope;

/// Shared connection state for network endpoints (gRPC, WebSocket).
///
/// Tracks live connections, an active-connection count (used by `Consumer::available`),
/// and optional label-based routing. All internal state is reference-counted so the
/// registry can be cheaply cloned and shared between the endpoint struct (Consumer side)
/// and the background accept/serve task (connection management side).
#[derive(Clone)]
pub struct ConnectionRegistry<T> {
    connections: Arc<Mutex<HashMap<u64, mpsc::Sender<T>>>>,
    next_id: Arc<AtomicU64>,
    active: Arc<AtomicUsize>,
    accept_labels: Option<Vec<String>>,
}

impl<T: Clone + Send + 'static> ConnectionRegistry<T> {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            accept_labels: None,
        }
    }

    pub fn accept_labels(mut self, labels: Vec<String>) -> Self {
        self.accept_labels = Some(labels);
        self
    }

    pub fn available(&self) -> bool {
        self.active.load(Ordering::Relaxed) > 0
    }

    pub fn validates(&self, envelope: &Envelope) -> bool {
        match &self.accept_labels {
            None => true,
            Some(labels) => envelope.message.get_labels().iter().any(|l| labels.contains(l)),
        }
    }

    /// Register a new client connection. Returns the assigned connection ID.
    pub async fn connect(&self, tx: mpsc::Sender<T>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.connections.lock().await.insert(id, tx);
        self.active.fetch_add(1, Ordering::Relaxed);
        id
    }

    /// Unregister a client connection. No-ops if the connection was already removed
    /// (e.g., by `broadcast` pruning a dead sender before the task exit fires).
    pub async fn disconnect(&self, id: u64) {
        if self.connections.lock().await.remove(&id).is_some() {
            self.active.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Send `msg` to every live connection. Dead senders are pruned; subsequent
    /// `disconnect` calls for the same ID are safe no-ops.
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
