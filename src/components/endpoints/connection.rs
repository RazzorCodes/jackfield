use std::collections::HashMap;
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
