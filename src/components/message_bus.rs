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
        while let Some(message) = self.pending.pop_front() {
            for consumer in &self.consumers {
                if consumer.validate(&message) {
                    consumer.consume(message);
                    break;
                }
            }
        }
    }
}
