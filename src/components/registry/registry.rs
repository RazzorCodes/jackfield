use std::sync::Arc;

use crate::components::endpoint::Consumer;
use crate::components::router::dimensions::dimension::{DimState, Dimension, DispatchEvent, EventMeta, Verdict};
use crate::components::router::envelope::Envelope;

pub(crate) struct DimEntry {
    pub(crate) dim: Box<dyn Dimension>,
    pub(crate) state: DimState,
    pub(crate) reject_on_miss: bool,
}

pub(crate) struct RegistrationEntry {
    pub(crate) id: u64,
    pub(crate) consumer: Box<dyn Consumer>,
    pub(crate) dims: Vec<DimEntry>,
}

pub struct RegistrationBuilder<'a> {
    pub id: u64,
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

pub(crate) struct RoutePlan {
    pub(crate) meta: Arc<EventMeta>,
    pub(crate) qualified: Vec<(f32, u64, usize)>,
}

pub struct Registry {
    pub(crate) entries: Vec<RegistrationEntry>,
    next_id: u64,
}

impl Registry {
    pub fn new() -> Self {
        Registry { entries: Vec::new(), next_id: 0 }
    }

    pub fn register(&mut self, consumer: Box<dyn Consumer>) -> RegistrationBuilder<'_> {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(RegistrationEntry { id, consumer, dims: vec![] });
        let idx = self.entries.len() - 1;
        RegistrationBuilder { id, entries: &mut self.entries, idx }
    }

    pub fn deregister(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() < before
    }

    pub(crate) fn plan(&self, envelope: &Envelope) -> RoutePlan {
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
                // Non-finite scores from misbehaving adaptive dims are clamped to 0.0.
                let score = if total.is_finite() { total } else { 0.0 };
                qualified.push((score, entry.id, i));
            }
        }

        qualified.sort_by(|(sa, id_a, _), (sb, id_b, _)| {
            sb.total_cmp(sa).then_with(|| id_a.cmp(id_b))
        });

        RoutePlan { meta, qualified }
    }

    pub(crate) fn observe(
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

    pub(crate) fn observe_consumed(
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
