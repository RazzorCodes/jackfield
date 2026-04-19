use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::components::bus::dimension::{DimState, Dimension, DispatchEvent, EventMeta, Verdict};
use crate::components::bus::envelope::{Envelope, ProducerId, ProducerHandle};
use crate::components::bus::throttle::{Throttle, TokenBucket};
use crate::components::endpoint::{Consumer, Producer};

// ── Registration internals ────────────────────────────────────────────────────

struct DimEntry {
    dim: Box<dyn Dimension>,
    state: DimState,
    reject_on_miss: bool,
}

struct RegistrationEntry {
    id: u64,
    consumer: Box<dyn Consumer>,
    dims: Vec<DimEntry>,
    #[allow(dead_code)]
    discovery: bool,
}

pub struct RegistrationBuilder<'a> {
    entries: &'a mut Vec<RegistrationEntry>,
    idx: usize,
}

impl<'a> RegistrationBuilder<'a> {
    pub fn require(self, dim: impl Dimension + 'static) -> Self {
        let state = dim.new_state(1.0);
        self.entries[self.idx].dims.push(DimEntry { dim: Box::new(dim), state, reject_on_miss: true });
        self
    }

    pub fn prefer(self, dim: impl Dimension + 'static, weight: f32) -> Self {
        let state = dim.new_state(weight);
        self.entries[self.idx].dims.push(DimEntry { dim: Box::new(dim), state, reject_on_miss: false });
        self
    }

    pub fn with_discovery(self) -> Self {
        self.entries[self.idx].discovery = true;
        self
    }
}

// ── Registry ──────────────────────────────────────────────────────────────────

struct Registry {
    entries: Vec<RegistrationEntry>,
    next_id: u64,
}

struct RoutePlan {
    meta: Arc<EventMeta>,
    qualified: Vec<(f32, usize)>,
}

impl Registry {
    fn new() -> Self {
        Registry { entries: Vec::new(), next_id: 0 }
    }

    fn register(&mut self, consumer: Box<dyn Consumer>) -> RegistrationBuilder<'_> {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(RegistrationEntry { id, consumer, dims: vec![], discovery: false });
        let idx = self.entries.len() - 1;
        RegistrationBuilder { entries: &mut self.entries, idx }
    }

    /// Ranks consumers for the given envelope without blocking on consumption.
    fn plan(&self, envelope: &Envelope) -> RoutePlan {
        let meta = Arc::new(EventMeta::from_envelope(envelope));
        let mut qualified: Vec<(f32, usize)> = Vec::new();

        for (i, entry) in self.entries.iter().enumerate() {
            let mut total = 0.0f32;
            let mut rejected = false;
            for de in &entry.dims {
                match de.dim.evaluate(envelope, &de.state) {
                    Verdict::Reject => {
                        if de.reject_on_miss {
                            rejected = true;
                            break;
                        }
                    }
                    Verdict::Score(s) => {
                        total += de.state.weight * s;
                    }
                }
            }
            if !rejected {
                // Non-finite scores (NaN, ±inf from a misbehaving adaptive dim) are
                // clamped to 0.0 so they never hijack the sort order.
                let score = if total.is_finite() { total } else { 0.0 };
                qualified.push((score, i));
            }
        }

        // Sort by score descending, then registration ID ascending.
        qualified.sort_by(|(sa, ia), (sb, ib)| {
            sb.total_cmp(sa).then_with(|| self.entries[*ia].id.cmp(&self.entries[*ib].id))
        });

        RoutePlan { meta, qualified }
    }

    /// Observes the outcome of a dispatch attempt.
    fn observe(&mut self, plan: RoutePlan, winner_idx: Option<usize>, pre_winner: Vec<(usize, bool)>) {
        let meta = plan.meta;
        let mut events: Vec<(usize, DispatchEvent)> = pre_winner
            .into_iter()
            .map(|(idx, is_busy)| {
                let event = if is_busy {
                    DispatchEvent::Busy { meta: meta.clone() }
                } else {
                    DispatchEvent::Vetoed { meta: meta.clone() }
                };
                (idx, event)
            })
            .collect();

        if let Some(winner_idx) = winner_idx {
            if let Some(pos) = plan.qualified.iter().position(|(_, idx)| *idx == winner_idx) {
                for (_, idx) in &plan.qualified[pos + 1..] {
                    events.push((*idx, DispatchEvent::Skipped { meta: meta.clone() }));
                }
            }
        }

        for (idx, event) in events {
            for de in &mut self.entries[idx].dims {
                de.dim.observe(&event, &mut de.state);
            }
        }
    }

    fn observe_consumed(&mut self, idx: usize, meta: Arc<EventMeta>, latency: std::time::Duration) {
        let event = DispatchEvent::Consumed { meta, latency };
        for de in &mut self.entries[idx].dims {
            de.dim.observe(&event, &mut de.state);
        }
    }
}

// ── Bus ───────────────────────────────────────────────────────────────────────

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
        let plan = self.registry.plan(&envelope);
        let mut pre_winner = Vec::new();
        let mut winner_idx = None;

        let start = Instant::now();
        for (_, idx) in &plan.qualified {
            let entry = &mut self.registry.entries[*idx];
            if !entry.consumer.available() {
                pre_winner.push((*idx, true));
            } else if !entry.consumer.validate(&envelope) {
                pre_winner.push((*idx, false));
            } else {
                winner_idx = Some(*idx);
                break;
            }
        }

        if let Some(idx) = winner_idx {
            let meta = plan.meta.clone();
            let consumer = &mut self.registry.entries[idx].consumer;
            consumer.consume(envelope.message).await;
            let latency = start.elapsed();

            self.registry.observe(plan, Some(idx), pre_winner);
            self.registry.observe_consumed(idx, meta, latency);
            None
        } else {
            self.registry.observe(plan, None, pre_winner);
            Some(envelope)
        }
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

impl Default for Bus {
    fn default() -> Self {
        Self::new(256).max_pending(1024)
    }
}
