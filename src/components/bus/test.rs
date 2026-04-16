#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use crate::components::bus::bus::*;
    use crate::components::endpoint::*;
    use crate::components::message::*;

    struct MockConsumer {
        accepted_labels: Vec<String>,
    }

    impl Consumer for MockConsumer {
        fn available(&self) -> bool {
            true
        }

        fn validate(&self, message: &dyn Message) -> bool {
            message
                .get_labels()
                .iter()
                .all(|l| self.accepted_labels.contains(l))
        }

        fn consume(&mut self, _message: Box<dyn Message>) {}
    }

    #[test]
    fn bus_creation() {
        let _bus = Bus::new();
    }

    #[test]
    fn sync_routing() {
        let mut bus = Bus::new();

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
            .unwrap();
        endpoint
            .send_bus(Box::new(BaseMessage::new(
                None,
                Some(vec!["label1".to_string(), "label2".to_string()]),
                None,
            )))
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
            .unwrap();

        bus.route_sync();

        assert!(!bus.done(), "Bus should have one unprocessed message");

        bus.register_consumer(Box::new(MockConsumer {
            accepted_labels: vec![
                "label1".to_string(),
                "label2".to_string(),
                "label3".to_string(),
            ],
        }));
        bus.route_sync();
        assert!(bus.done(), "Bus should be empty now");
    }
}
