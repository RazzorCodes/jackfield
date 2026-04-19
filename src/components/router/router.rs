// Router trait + AffinityRouter (score-based) + BlindRouter (in-order, hard-reject only).
use std::time::Instant;

use crate::components::endpoint::Consumer;
use super::dimensions::dimension::Verdict;
use super::envelope::Envelope;
use super::registry::{Registry, RegistrationBuilder};

pub trait Router: Send {
    fn register_consumer(&mut self, consumer: Box<dyn Consumer>) -> RegistrationBuilder<'_>;
    fn route(&mut self, envelope: Envelope) -> impl std::future::Future<Output = Option<Envelope>> + Send;
}

pub struct AffinityRouter {
    registry: Registry,
}

impl AffinityRouter {
    pub fn new() -> Self {
        AffinityRouter { registry: Registry::new() }
    }
}

impl Default for AffinityRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Router for AffinityRouter {
    fn register_consumer(&mut self, consumer: Box<dyn Consumer>) -> RegistrationBuilder<'_> {
        self.registry.register(consumer)
    }

    async fn route(&mut self, envelope: Envelope) -> Option<Envelope> {
        let plan = self.registry.plan(&envelope);
        let mut pre_winner = Vec::new();
        let mut winner_idx = None;

        for (_, _, idx) in &plan.qualified {
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
            let start = Instant::now();
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
}

pub struct BlindRouter {
    registry: Registry,
}

impl BlindRouter {
    pub fn new() -> Self {
        BlindRouter { registry: Registry::new() }
    }
}

impl Default for BlindRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Router for BlindRouter {
    fn register_consumer(&mut self, consumer: Box<dyn Consumer>) -> RegistrationBuilder<'_> {
        self.registry.register(consumer)
    }

    async fn route(&mut self, envelope: Envelope) -> Option<Envelope> {
        for entry in &mut self.registry.entries {
            let hard_rejected = entry.dims.iter()
                .filter(|de| de.reject_on_miss)
                .any(|de| matches!(de.dim.evaluate(&envelope, &de.state), Verdict::Reject));
            if hard_rejected {
                continue;
            }
            if !entry.consumer.available() {
                continue;
            }
            if !entry.consumer.validate(&envelope) {
                continue;
            }
            entry.consumer.consume(envelope.message).await;
            return None;
        }
        Some(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::future::Future;
    use crate::components::router::envelope::{Envelope, ProducerId};
    use crate::components::router::dimensions::dims::ProducerDim;
    use crate::components::message::BaseMessage;

    struct MockConsumer {
        received: usize,
    }

    impl MockConsumer {
        fn new() -> Self {
            MockConsumer { received: 0 }
        }
    }

    impl Consumer for MockConsumer {
        fn available(&self) -> bool {
            true
        }
        fn validate(&self, _: &Envelope) -> bool {
            true
        }
        fn consume(&mut self, _: Box<dyn crate::components::message::Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.received += 1;
            Box::pin(async {})
        }
    }

    #[tokio::test]
    async fn affinity_router_routes_to_matching_consumer() {
        let mut router = AffinityRouter::new();
        router.register_consumer(Box::new(MockConsumer::new()))
            .require(ProducerDim::only("producer_a"));

        let msg = Box::new(BaseMessage::new(None, None, None));
        let env = Envelope { origin: ProducerId("producer_a".into()), message: msg };
        assert!(router.route(env).await.is_none(), "should be consumed");
    }

    #[tokio::test]
    async fn affinity_router_returns_unhandled() {
        let mut router = AffinityRouter::new();
        router.register_consumer(Box::new(MockConsumer::new()))
            .require(ProducerDim::only("producer_a"));

        let msg = Box::new(BaseMessage::new(None, None, None));
        let env = Envelope { origin: ProducerId("other".into()), message: msg };
        assert!(router.route(env).await.is_some(), "should be unhandled");
    }

    #[tokio::test]
    async fn blind_router_routes_in_order() {
        let mut router = BlindRouter::new();
        router.register_consumer(Box::new(MockConsumer::new()));

        let msg = Box::new(BaseMessage::new(None, None, None));
        let env = Envelope { origin: ProducerId("any".into()), message: msg };
        assert!(router.route(env).await.is_none(), "should be consumed");
    }

    #[tokio::test]
    async fn blind_router_respects_hard_reject() {
        let mut router = BlindRouter::new();
        router.register_consumer(Box::new(MockConsumer::new()))
            .require(ProducerDim::only("producer_a"));

        let msg = Box::new(BaseMessage::new(None, None, None));
        let env = Envelope { origin: ProducerId("other".into()), message: msg };
        assert!(router.route(env).await.is_some(), "should be unhandled due to require filter");
    }
}
