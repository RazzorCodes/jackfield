// Bus: channel wrapper that dispatches messages through a pluggable Router.
use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::components::router::envelope::{Envelope, ProducerId, ProducerHandle};
use crate::components::router::registry::RegistrationBuilder;
use crate::components::router::router::{AffinityRouter, Router};
use crate::components::endpoint::{Consumer, Producer, Throttle};
use crate::components::endpoint::throttle::TokenBucket;

pub struct Bus<R: Router = AffinityRouter> {
    tx: mpsc::Sender<Envelope>,
    rx: mpsc::Receiver<Envelope>,
    router: R,
    pending: VecDeque<Envelope>,
    max_pending: usize,
}

impl<R: Router> Bus<R> {
    pub fn with_router(capacity: usize, router: R) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Bus { tx, rx, router, pending: VecDeque::new(), max_pending: usize::MAX }
    }

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

    pub fn register_consumer(&mut self, consumer: Box<dyn Consumer>) -> RegistrationBuilder<'_> {
        self.router.register_consumer(consumer)
    }

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

    async fn route(&mut self, envelope: Envelope) -> Option<Envelope> {
        self.router.route(envelope).await
    }

    pub async fn drain(&mut self) {
        for envelope in std::mem::take(&mut self.pending) {
            if let Some(envelope) = self.route(envelope).await {
                self.push_pending(envelope);
            }
        }
        while let Ok(envelope) = self.rx.try_recv() {
            if let Some(envelope) = self.route(envelope).await {
                self.push_pending(envelope);
            }
        }
    }

    pub async fn dispatch(&mut self) {
        while let Some(envelope) = self.rx.recv().await {
            if let Some(unhandled) = self.route(envelope).await {
                self.push_pending(unhandled);
            }
        }
    }
}

impl Bus<AffinityRouter> {
    pub fn new(capacity: usize) -> Self {
        Self::with_router(capacity, AffinityRouter::new())
    }
}

impl Default for Bus<AffinityRouter> {
    fn default() -> Self {
        Self::new(256).max_pending(1024)
    }
}
