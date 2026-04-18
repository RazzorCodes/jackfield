use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::components::bus::envelope::{Envelope, ProducerId, ProducerHandle};
use crate::components::bus::throttle::{Throttle, TokenBucket};
use crate::components::endpoint::{Consumer, Producer};

struct Registry {
    consumers: Vec<Box<dyn Consumer>>,
}

impl Registry {
    fn new() -> Self {
        Registry {
            consumers: Vec::new(),
        }
    }

    fn register(&mut self, consumer: Box<dyn Consumer>) {
        self.consumers.push(consumer);
    }

    async fn route(&mut self, envelope: Envelope) -> Option<Envelope> {
        if let Some(idx) = self.consumers.iter().position(|c| c.available() && c.validate(&envelope)) {
            self.consumers[idx].consume(envelope.message).await;
            None
        } else {
            Some(envelope)
        }
    }
}

pub struct Bus {
    tx: mpsc::Sender<Envelope>,
    rx: mpsc::Receiver<Envelope>,
    registry: Registry,
    pending: VecDeque<Envelope>,
    max_pending: usize,
}

impl Bus {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Bus {
            tx,
            rx,
            registry: Registry::new(),
            pending: VecDeque::new(),
            max_pending: usize::MAX,
        }
    }

    /// Cap the number of unroutable messages held between drain cycles.
    /// When full, the oldest unrouted message is evicted to make room for the newest.
    pub fn max_pending(mut self, cap: usize) -> Self {
        self.max_pending = cap;
        self
    }

    fn push_pending(&mut self, envelope: Envelope) {
        if self.pending.len() >= self.max_pending {
            self.pending.pop_front();
        }
        self.pending.push_back(envelope);
    }

    pub fn register_consumer(&mut self, consumer: Box<dyn Consumer>) {
        self.registry.register(consumer);
    }

    /// Creates a `ProducerHandle` directly without going through a `Producer` impl.
    /// Useful for embedding producers (e.g. Python integration) that manage the handle themselves.
    pub fn make_handle(&self, name: impl Into<String>) -> ProducerHandle {
        let id = ProducerId(name.into());
        ProducerHandle::new(id, self.tx.clone(), None)
    }

    pub fn register_producer<P: Producer>(&mut self, producer: &mut P) {
        let id = ProducerId(producer.name().to_string());
        producer.attach(ProducerHandle::new(id, self.tx.clone(), None));
    }

    pub fn register_producer_throttled<P: Producer>(&mut self, producer: &mut P, throttle: Throttle) {
        let id = ProducerId(producer.name().to_string());
        let bucket = Arc::new(tokio::sync::Mutex::new(TokenBucket::new(throttle.rate, throttle.burst)));
        producer.attach(ProducerHandle::new(id, self.tx.clone(), Some(bucket)));
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty() && self.pending.is_empty()
    }

    /// Drains all currently pending messages. Unhandled messages are retained
    /// in an internal buffer (capped at `max_pending`) and retried next call.
    pub async fn drain(&mut self) {
        // Re-attempt messages that were unhandled on a previous drain.
        for envelope in std::mem::take(&mut self.pending) {
            if let Some(envelope) = self.registry.route(envelope).await {
                self.push_pending(envelope);
            }
        }
        // Process newly arrived messages from the channel.
        while let Ok(envelope) = self.rx.try_recv() {
            if let Some(envelope) = self.registry.route(envelope).await {
                self.push_pending(envelope);
            }
        }
    }

    /// Continuous async dispatch loop. Runs until all senders are dropped.
    /// Messages that no consumer accepts are held in the internal pending buffer.
    pub async fn dispatch(&mut self) {
        while let Some(envelope) = self.rx.recv().await {
            if let Some(unhandled) = self.registry.route(envelope).await {
                self.push_pending(unhandled);
            }
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new(256).max_pending(1024)
    }
}
