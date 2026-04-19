// Registry: consumer registration table with per-consumer dimension entries.
// RegistrationBuilder: fluent API for attaching routing dimensions to a consumer.
use std::sync::Arc;

use crate::components::endpoint::Consumer;
use super::dimensions::dimension::{DimState, Dimension, DispatchEvent, EventMeta, Verdict};
use super::envelope::Envelope;

pub(super) struct DimEntry {
    pub(super) dim: Box<dyn Dimension>,
    pub(super) state: DimState,
    pub(super) reject_on_miss: bool,
}

pub(super) struct RegistrationEntry {
    pub(super) id: u64,
    pub(super) consumer: Box<dyn Consumer>,
    pub(super) dims: Vec<DimEntry>,
}

pub struct RegistrationBuilder<'a> {
    entries: &'a mut Vec<RegistrationEntry>,
    idx: usize,
}

impl<'a> RegistrationBuilder<'a> {
    pub fn require(self, dim: impl Dimension + 'static) -> Self {
        self.require_boxed(Box::new(dim))
    }

    pub fn require_boxed(self, dim: Box<dyn Dimension>) -> Self {
        let state = dim.new_state(1.0);
        self.entries[self.idx].dims.push(DimEntry { dim, state, reject_on_miss: true });
        self
    }

    pub fn prefer(self, dim: impl Dimension + 'static, weight: f32) -> Self {
        self.prefer_boxed(Box::new(dim), weight)
    }

    pub fn prefer_boxed(self, dim: Box<dyn Dimension>, weight: f32) -> Self {
        let state = dim.new_state(weight);
        self.entries[self.idx].dims.push(DimEntry { dim, state, reject_on_miss: false });
        self
    }
}

pub(super) struct RoutePlan {
    pub(super) meta: Arc<EventMeta>,
    pub(super) qualified: Vec<(f32, u64, usize)>,
}

pub(super) struct Registry {
    pub(super) entries: Vec<RegistrationEntry>,
    next_id: u64,
}

impl Registry {
    pub(super) fn new() -> Self {
        Registry { entries: Vec::new(), next_id: 0 }
    }

    pub(super) fn register(&mut self, consumer: Box<dyn Consumer>) -> RegistrationBuilder<'_> {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(RegistrationEntry { id, consumer, dims: vec![] });
        let idx = self.entries.len() - 1;
        RegistrationBuilder { entries: &mut self.entries, idx }
    }

    /// Ranks consumers for the given envelope without blocking on consumption.
    pub(super) fn plan(&self, envelope: &Envelope) -> RoutePlan {
        let meta = Arc::new(EventMeta::from_envelope(envelope));
        let mut qualified: Vec<(f32, u64, usize)> = Vec::new();

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
                qualified.push((score, entry.id, i));
            }
        }

        qualified.sort_by(|(sa, id_a, _), (sb, id_b, _)| {
            sb.total_cmp(sa).then_with(|| id_a.cmp(id_b))
        });

        RoutePlan { meta, qualified }
    }

    pub(super) fn observe(
        &mut self,
        plan: RoutePlan,
        winner_idx: Option<usize>,
        pre_winner: Vec<(usize, bool)>,
    ) {
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
            if let Some(pos) = plan.qualified.iter().position(|(_, _, idx)| *idx == winner_idx) {
                for (_, _, idx) in &plan.qualified[pos + 1..] {
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

    pub(super) fn observe_consumed(
        &mut self,
        idx: usize,
        meta: Arc<EventMeta>,
        latency: std::time::Duration,
    ) {
        let event = DispatchEvent::Consumed { meta, latency };
        for de in &mut self.entries[idx].dims {
            de.dim.observe(&event, &mut de.state);
        }
    }
}
