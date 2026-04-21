// Bus: channel wrapper that dispatches messages through a pluggable Router.
use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::components::registry::registry::Registry;
use crate::components::registry::RegistrationBuilder;
use crate::components::router::dimensions::Dimension;
use crate::components::router::envelope::{Envelope, ProducerId, ProducerHandle};
use crate::components::router::router::{AffinityRouter, Router};
use crate::components::endpoint::{Consumer, Producer, Throttle};
use crate::components::endpoint::throttle::TokenBucket;
use crate::components::bus::error::JackfieldError;

pub enum BusCmd {
    Register {
        consumer: Box<dyn Consumer>,
        dims: Vec<(Box<dyn Dimension>, bool, f32)>,
        reply: oneshot::Sender<u64>,
    },
    Deregister {
        id: u64,
    },
}

#[derive(Clone)]
pub struct BusCmdHandle {
    cmd_tx: mpsc::Sender<BusCmd>,
    envelope_tx: mpsc::Sender<Envelope>,
}

impl BusCmdHandle {
    pub async fn register(
        &self,
        consumer: Box<dyn Consumer>,
        dims: Vec<(Box<dyn Dimension>, bool, f32)>,
    ) -> Result<u64, JackfieldError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(BusCmd::Register { consumer, dims, reply: reply_tx })
            .await
            .map_err(|_| JackfieldError::ChannelClosed)?;
        reply_rx.await.map_err(|_| JackfieldError::ChannelClosed)
    }

    pub async fn deregister(&self, id: u64) {
        let _ = self.cmd_tx.send(BusCmd::Deregister { id }).await;
    }

    pub fn make_producer_handle(&self, name: impl Into<String>) -> ProducerHandle {
        ProducerHandle::new(ProducerId(name.into()), self.envelope_tx.clone(), None)
    }
}

pub struct Bus<R: Router = AffinityRouter> {
    tx: mpsc::Sender<Envelope>,
    rx: mpsc::Receiver<Envelope>,
    cmd_tx: mpsc::Sender<BusCmd>,
    cmd_rx: mpsc::Receiver<BusCmd>,
    router: R,
    registry: Registry,
    pending: VecDeque<Envelope>,
    max_pending: usize,
}

impl<R: Router> Bus<R> {
    pub fn with_router(capacity: usize, router: R) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        Bus { tx, rx, cmd_tx, cmd_rx, router, registry: Registry::new(), pending: VecDeque::new(), max_pending: usize::MAX }
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
        self.registry.register(consumer)
    }

    pub fn deregister_consumer(&mut self, id: u64) -> bool {
        self.registry.deregister(id)
    }

    pub fn make_handle(&self, name: impl Into<String>) -> ProducerHandle {
        let id = ProducerId(name.into());
        ProducerHandle::new(id, self.tx.clone(), None)
    }

    pub fn make_cmd_handle(&self) -> BusCmdHandle {
        BusCmdHandle { cmd_tx: self.cmd_tx.clone(), envelope_tx: self.tx.clone() }
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

    fn apply_cmd(&mut self, cmd: BusCmd) {
        match cmd {
            BusCmd::Register { consumer, dims, reply } => {
                let mut b = self.registry.register(consumer);
                for (dim, required, weight) in dims {
                    b = if required { b.require_boxed(dim) } else { b.prefer_boxed(dim, weight) };
                }
                let _ = reply.send(b.id);
            }
            BusCmd::Deregister { id } => { self.registry.deregister(id); }
        }
    }

    async fn route(&mut self, envelope: Envelope) -> Option<Envelope> {
        self.router.route(&mut self.registry, envelope).await
    }

    pub async fn drain(&mut self) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            self.apply_cmd(cmd);
        }
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
        loop {
            tokio::select! {
                Some(envelope) = self.rx.recv() => {
                    if let Some(unhandled) = self.route(envelope).await {
                        self.push_pending(unhandled);
                    }
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    self.apply_cmd(cmd);
                }
                else => break,
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
