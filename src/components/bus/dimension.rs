use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use crate::components::bus::envelope::{Envelope, ProducerId};

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
