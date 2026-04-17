use tokio::sync::mpsc;

use crate::components::bus::envelope::{Envelope, ProducerId, ProducerHandle};
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

    fn route(&mut self, envelope: Envelope) -> Option<Envelope> {
        if let Some(idx) = self.consumers.iter().position(|c| c.validate(&envelope)) {
            self.consumers[idx].consume(envelope.message);
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
}

impl Bus {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Bus {
            tx,
            rx,
            registry: Registry::new(),
        }
    }

    pub fn register_consumer(&mut self, consumer: Box<dyn Consumer>) {
        self.registry.register(consumer);
    }

    pub fn register_producer<P: Producer>(&mut self, producer: &mut P) {
        let id = ProducerId(producer.name().to_string());
        producer.attach(ProducerHandle::new(id, self.tx.clone()));
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    /// Drains all currently pending messages synchronously. Unhandled messages are requeued.
    pub fn drain(&mut self) {
        let mut unhandled = Vec::new();
        while let Ok(envelope) = self.rx.try_recv() {
            if let Some(envelope) = self.registry.route(envelope) {
                unhandled.push(envelope);
            }
        }
        for envelope in unhandled {
            let _ = self.tx.try_send(envelope);
        }
    }

    /// Continuous async dispatch loop. Runs until all senders are dropped.
    pub async fn dispatch(&mut self) {
        while let Some(envelope) = self.rx.recv().await {
            self.registry.route(envelope);
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new(256)
    }
}
