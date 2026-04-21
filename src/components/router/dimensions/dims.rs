// Built-in routing dimensions: ProducerDim (by origin name), LabelDim (by message labels), SizeDim (by payload size).
use super::dimension::{adjust_weight, DimState, Dimension, DispatchEvent, Verdict};
use crate::components::router::envelope::Envelope;

#[derive(Clone)]
enum Matcher {
    AnyOf(Vec<String>),
    NoneOf(Vec<String>),
}

impl Matcher {
    fn matches(&self, value: &str) -> bool {
        match self {
            Matcher::AnyOf(list) => list.iter().any(|s| s == value),
            Matcher::NoneOf(list) => !list.iter().any(|s| s == value),
        }
    }
}

#[derive(Clone)]
pub struct ProducerDim(Matcher);

impl ProducerDim {
    pub fn only(name: impl Into<String>) -> Self {
        ProducerDim(Matcher::AnyOf(vec![name.into()]))
    }

    pub fn any_of(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        ProducerDim(Matcher::AnyOf(names.into_iter().map(|n| n.into()).collect()))
    }

    pub fn none_of(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        ProducerDim(Matcher::NoneOf(names.into_iter().map(|n| n.into()).collect()))
    }
}

impl Dimension for ProducerDim {
    fn evaluate(&self, env: &Envelope, _state: &DimState) -> Verdict {
        if self.0.matches(&env.origin.0) {
            Verdict::Score(1.0)
        } else {
            Verdict::Reject
        }
    }

    fn observe(&self, event: &DispatchEvent, state: &mut DimState) {
        adjust_weight(event, state);
    }
}

#[derive(Clone)]
enum LabelMatcher {
    AnyOf(Vec<String>),
    AllOf(Vec<String>),
    NoneOf(Vec<String>),
}

#[derive(Clone)]
pub struct LabelDim(LabelMatcher);

impl LabelDim {
    pub fn any_of(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        LabelDim(LabelMatcher::AnyOf(labels.into_iter().map(|l| l.into()).collect()))
    }

    pub fn all_of(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        LabelDim(LabelMatcher::AllOf(labels.into_iter().map(|l| l.into()).collect()))
    }

    pub fn none_of(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        LabelDim(LabelMatcher::NoneOf(labels.into_iter().map(|l| l.into()).collect()))
    }
}

impl Dimension for LabelDim {
    fn evaluate(&self, env: &Envelope, _state: &DimState) -> Verdict {
        let msg_labels = env.message.get_labels();
        let matched = match &self.0 {
            LabelMatcher::AnyOf(l) => l.iter().any(|label| msg_labels.contains(label)),
            LabelMatcher::AllOf(l) => l.iter().all(|label| msg_labels.contains(label)),
            LabelMatcher::NoneOf(l) => !l.iter().any(|label| msg_labels.contains(label)),
        };
        if matched { Verdict::Score(1.0) } else { Verdict::Reject }
    }

    fn observe(&self, event: &DispatchEvent, state: &mut DimState) {
        adjust_weight(event, state);
    }
}

#[derive(Clone)]
pub struct SizeDim {
    min: Option<usize>,
    max: Option<usize>,
}

impl SizeDim {
    pub fn at_most(max_bytes: usize) -> Self {
        SizeDim { min: None, max: Some(max_bytes) }
    }

    pub fn at_least(min_bytes: usize) -> Self {
        SizeDim { min: Some(min_bytes), max: None }
    }

    pub fn between(min: usize, max: usize) -> Self {
        SizeDim { min: Some(min), max: Some(max) }
    }
}

impl Dimension for SizeDim {
    fn evaluate(&self, env: &Envelope, _state: &DimState) -> Verdict {
        let size = env.message.get_bytes().len();
        let too_small = self.min.map(|m| size < m).unwrap_or(false);
        let too_large = self.max.map(|m| size > m).unwrap_or(false);

        if too_small || too_large {
            Verdict::Reject
        } else {
            Verdict::Score(1.0)
        }
    }

    fn observe(&self, event: &DispatchEvent, state: &mut DimState) {
        adjust_weight(event, state);
    }
}
