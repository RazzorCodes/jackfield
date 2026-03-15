use crate::components::message::*;
use bitflags::bitflags;
use std::error::Error;
use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use uuid::Uuid;

pub struct MessageBus {
    sender: Sender<Box<dyn Message>>,
    receiver: Receiver<Box<dyn Message>>,
    consumers: Vec<Box<dyn Consumer>>,
}

impl MessageBus {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        MessageBus {
            sender,
            receiver,
            consumers: Vec::new(),
        }
    }

    pub fn register_consumer(&mut self, consumer: Box<dyn Consumer>) {
        self.consumers.push(consumer);
    }

    pub fn register_producer(&mut self, producer: &mut dyn Producer) {
        producer.attach(self.sender.clone());
    }

    pub fn done(&self) -> bool {
        matches!(self.receiver.try_recv(), Err(TryRecvError::Empty))
    }

    pub fn route_sync(&mut self) {
        let messages: Vec<Box<dyn Message>> = self.receiver.try_iter().collect();

        for message in messages {
            let consumer_index = self.consumers.iter().position(|c| c.validate(&*message));

            match consumer_index {
                Some(idx) => self.consumers[idx].consume(message),
                None => {
                    // re-queue unprocessed — zero-copy, box is moved back in
                    let _ = self.sender.send(message);
                }
            }
        }
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}

pub trait Consumer: Sync + Send {
    fn available(&self) -> bool;
    fn validate(&self, message: &dyn Message) -> bool;
    fn consume(&mut self, message: Box<dyn Message>);
}

pub trait Producer {
    fn attach(&mut self, sender: Sender<Box<dyn Message>>);
    fn send_bus(&mut self, msg: Box<dyn Message>) -> Result<(), JackfieldError>;
}

#[derive(Debug)]
pub enum JackfieldError {
    NotRegistered,
    ChannelClosed,
    Custom(String),
}

impl fmt::Display for JackfieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegistered => write!(f, "Endpoint not registered with a bus"),
            Self::ChannelClosed => write!(f, "Bus channel has been closed"),
            Self::Custom(msg) => write!(f, "Bus error: {}", msg),
        }
    }
}

impl Error for JackfieldError {}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EndpointType: u8 {
        const CONSUMER = 1 << 0;
        const PRODUCER = 1 << 1;
    }
}

pub struct Endpoint {
    name: String,
    flags: EndpointType,
    sender: Option<Sender<Box<dyn Message>>>,
}

impl Endpoint {
    pub fn new(name: impl Into<String>, flags: EndpointType) -> Self {
        Endpoint {
            name: name.into(),
            flags,
            sender: None,
        }
    }
}

impl Producer for Endpoint {
    fn attach(&mut self, sender: Sender<Box<dyn Message>>) {
        self.sender = Some(sender);
    }

    fn send_bus(&mut self, msg: Box<dyn Message>) -> Result<(), JackfieldError> {
        if !self.flags.contains(EndpointType::PRODUCER) {
            return Err(JackfieldError::NotRegistered);
        }

        self.sender
            .as_ref()
            .ok_or(JackfieldError::NotRegistered)?
            .send(msg)
            .map_err(|_| JackfieldError::ChannelClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let _bus = MessageBus::new();
    }

    #[test]
    fn sync_routing() {
        let mut bus = MessageBus::new();

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
