pub mod components;

#[cfg(feature = "python")]
pub mod integrations;

#[cfg(test)]
mod tests {
    use super::*;

    struct MockEndpoint {
        accepted_labels: Vec<String>,
    }

    impl Consumer for MockEndpoint {
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
        let _bus = MessageBus::new();
    }
}
