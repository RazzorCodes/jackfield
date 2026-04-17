#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::future::Future;

    use crate::components::bus::bus::Bus;
    use crate::components::bus::envelope::Envelope;
    use crate::components::endpoint::{Consumer, Endpoint, EndpointType, Producer};
    use crate::components::message::BaseMessage;

    struct MockConsumer {
        accepted_labels: Vec<String>,
    }

    impl Consumer for MockConsumer {
        fn available(&self) -> bool {
            true
        }

        fn validate(&self, envelope: &Envelope) -> bool {
            envelope
                .message
                .get_labels()
                .iter()
                .all(|l| self.accepted_labels.contains(l))
        }

        fn consume(&mut self, _message: Box<dyn crate::components::message::Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            Box::pin(async {})
        }
    }

    #[test]
    fn bus_creation() {
        let _bus = Bus::default();
    }

    #[tokio::test]
    async fn routing() {
        let mut bus = Bus::default();

        bus.register_consumer(Box::new(MockConsumer {
            accepted_labels: vec!["label1".to_string(), "label2".to_string()],
        }));

        let mut endpoint = Endpoint::new("test_endpoint", EndpointType::PRODUCER);
        bus.register_producer(&mut endpoint);

        endpoint
            .send_bus(Box::new(BaseMessage::new(
                None,
                Some(vec!["label1".to_string()]),
                None,
            )))
            .await
            .unwrap();
        endpoint
            .send_bus(Box::new(BaseMessage::new(
                None,
                Some(vec!["label1".to_string(), "label2".to_string()]),
                None,
            )))
            .await
            .unwrap();
        endpoint
            .send_bus(Box::new(BaseMessage::new(
                None,
                Some(vec![
                    "label1".to_string(),
                    "label2".to_string(),
                    "label3".to_string(),
                ]),
                None,
            )))
            .await
            .unwrap();

        bus.drain().await;
        assert!(!bus.is_empty(), "Bus should have one unprocessed message");

        bus.register_consumer(Box::new(MockConsumer {
            accepted_labels: vec![
                "label1".to_string(),
                "label2".to_string(),
                "label3".to_string(),
            ],
        }));
        bus.drain().await;
        assert!(bus.is_empty(), "Bus should be empty now");
    }

    #[tokio::test]
    async fn producer_identity_preserved() {
        use std::sync::{Arc, Mutex};

        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let captured_clone = captured.clone();

        struct RecordingConsumer {
            origins: Arc<Mutex<Vec<String>>>,
        }

        impl Consumer for RecordingConsumer {
            fn available(&self) -> bool {
                true
            }
            fn validate(&self, envelope: &Envelope) -> bool {
                self.origins
                    .lock()
                    .unwrap()
                    .push(envelope.origin.as_str().to_string());
                true
            }
            fn consume(&mut self, _: Box<dyn crate::components::message::Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                Box::pin(async {})
            }
        }

        let mut bus = Bus::default();
        bus.register_consumer(Box::new(RecordingConsumer {
            origins: captured_clone,
        }));

        let mut ep_a = Endpoint::new("producer_a", EndpointType::PRODUCER);
        let mut ep_b = Endpoint::new("producer_b", EndpointType::PRODUCER);
        bus.register_producer(&mut ep_a);
        bus.register_producer(&mut ep_b);

        ep_a.send_bus(Box::new(BaseMessage::new(None, None, None)))
            .await
            .unwrap();
        ep_b.send_bus(Box::new(BaseMessage::new(None, None, None)))
            .await
            .unwrap();

        bus.drain().await;

        let seen = captured.lock().unwrap();
        assert!(seen.contains(&"producer_a".to_string()));
        assert!(seen.contains(&"producer_b".to_string()));
    }
}
