use crate::components::message::*;
use std::collections::VecDeque;
use uuid::Uuid;

pub trait Consumer: Sync + Send {
    fn validate(&self, message: &Box<dyn Message>) -> bool;
    fn consume(&self, message: Box<dyn Message>);
}

pub struct MessageBus {
    pending: VecDeque<Box<dyn Message>>,
    // ---
    consumers: Vec<Box<dyn Consumer>>,
}

impl MessageBus {
    pub fn new() -> Self {
        return MessageBus {
            pending: VecDeque::new(),
            consumers: Vec::new(),
        };
    }
    pub fn add_consumer(&mut self, consumer: Box<dyn Consumer>) {
        self.consumers.push(consumer);
    }
    pub fn add_message(&mut self, mut message: Box<dyn Message>) -> Uuid {
        if message.get_uuid() == Uuid::nil() {
            message.aquire_uuid();
        }

        let id = message.get_uuid();
        self.pending.push_back(message);

        return id;
    }

    fn next(&mut self) -> Option<Box<dyn Message>> {
        return self.pending.pop_front();
    }

    fn done(&self) -> bool {
        return self.pending.is_empty();
    }

    pub fn route_sync(&mut self) {
        let mut unprocessed: Vec<Box<dyn Message>> = vec![];

        while let Some(message) = self.pending.pop_front() {
            // 1. Find the index of the consumer that wants this message.
            // We only borrow the message here (&message).
            let consumer_index = self.consumers.iter().position(|c| c.validate(&message));

            match consumer_index {
                // 2. If we found one, give the message to that specific consumer.
                Some(idx) => {
                    self.consumers[idx].consume(message);
                }
                // 3. If none found, message is still owned by this scope, so push to unprocessed.
                None => {
                    unprocessed.push(message);
                }
            }
        }

        self.pending.extend(unprocessed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockConsumer {
        accepted_labels: Vec<String>,
    }
    impl Consumer for MockConsumer {
        fn validate(&self, message: &Box<dyn Message>) -> bool {
            let labels = message.get_labels();
            for label in labels.iter() {
                if !self.accepted_labels.contains(&label) {
                    return false;
                }
            }

            return true;
        }
        fn consume(&self, message: Box<dyn Message>) {}
    }

    #[test]
    fn bus_creation() {
        let bus = MessageBus::new();

        assert!(true)
    }

    #[test]
    fn sync_routing() {
        let mut bus = MessageBus::new();

        let mock_consumer = Box::new(MockConsumer {
            accepted_labels: vec!["label1".to_string(), "label2".to_string()],
        });
        bus.add_consumer(mock_consumer);

        let msg_ok1 = Box::new(BaseMessage::new(
            None,
            Some(vec!["label1".to_string()]),
            None,
        ));
        let msg_ok2 = Box::new(BaseMessage::new(
            None,
            Some(vec!["label1".to_string(), "label2".to_string()]),
            None,
        ));
        let msg_bad = Box::new(BaseMessage::new(
            None,
            Some(vec![
                "label1".to_string(),
                "label2".to_string(),
                "label3".to_string(),
            ]),
            None,
        ));
        bus.add_message(msg_ok1);
        bus.add_message(msg_ok2);
        bus.add_message(msg_bad);

        bus.route_sync();

        if bus.done() {
            assert!(false);
        }

        let mock_consumer_new = Box::new(MockConsumer {
            accepted_labels: vec![
                "label1".to_string(),
                "label2".to_string(),
                "label3".to_string(),
            ],
        });
        bus.add_consumer(mock_consumer_new);
        bus.route_sync();

        if !bus.done() {
            assert!(false);
        }
    }
}
