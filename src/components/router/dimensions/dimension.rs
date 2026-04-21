// Routing dimension trait: evaluate (score/reject a message), observe (post-dispatch feedback), DimState.
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use crate::components::router::envelope::{Envelope, ProducerId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Reject,
    Score(f32),
}

pub struct DimState {
    pub weight: f32,
    pub inner: Box<dyn Any + Send + Sync>,
}

#[derive(Debug, Clone)]
pub struct EventMeta {
    pub origin: ProducerId,
    pub labels: Vec<String>,
    pub size: usize,
}

impl EventMeta {
    pub fn from_envelope(env: &Envelope) -> Self {
        EventMeta {
            origin: env.origin.clone(),
            labels: env.message.get_labels().to_vec(),
            size: env.message.get_bytes().len(),
        }
    }
}

#[non_exhaustive]
pub enum DispatchEvent {
    Vetoed { meta: Arc<EventMeta> },
    Consumed { meta: Arc<EventMeta>, latency: Duration },
    Busy { meta: Arc<EventMeta> },
    Skipped { meta: Arc<EventMeta> },
}

pub trait Dimension: Send + Sync {
    fn evaluate(&self, env: &Envelope, state: &DimState) -> Verdict;
    fn observe(&self, _event: &DispatchEvent, _state: &mut DimState) {}
    fn new_state(&self, initial_weight: f32) -> DimState {
        DimState { weight: initial_weight, inner: Box::new(()) }
    }
}

/// Exponential weight adjustment shared by all built-in dimensions.
///
/// Consumed  → reward   (×1.1, cap 100.0)
/// Busy      → penalise (×0.8, floor 0.001)
/// Vetoed    → penalise (×0.9, floor 0.001)
/// Skipped   → no change (outranked, not misbehaving)
pub(crate) fn adjust_weight(event: &DispatchEvent, state: &mut DimState) {
    match event {
        DispatchEvent::Consumed { .. } => {
            state.weight = (state.weight * 1.1).min(100.0);
        }
        DispatchEvent::Busy { .. } => {
            state.weight = (state.weight * 0.8).max(0.001);
        }
        DispatchEvent::Vetoed { .. } => {
            state.weight = (state.weight * 0.9).max(0.001);
        }
        _ => {}
    }
}
