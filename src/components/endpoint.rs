use crate::components::bus::error::JackfieldError;
use crate::components::message::*;
use std::sync::mpsc::Sender;

pub trait Handler: Send + Sync {
    fn handle(&mut self) -> Option<Box<dyn Message>>;
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

use bitflags::bitflags;

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
    consumer_handler: Option<Box<dyn Consumer>>,
    producer_handler: Option<Box<dyn Handler>>,
}

impl Endpoint {
    pub fn new(name: impl Into<String>, flags: EndpointType) -> Self {
        Endpoint {
            name: name.into(),
            flags,
            sender: None,
            consumer_handler: None,
            producer_handler: None,
        }
    }

    pub fn set_consumer(mut self, consumer: impl Consumer + 'static) -> Self {
        self.consumer_handler = Some(Box::new(consumer));
        self
    }

    pub fn set_producer(mut self, handler: impl Handler + 'static) -> Self {
        self.producer_handler = Some(Box::new(handler));
        self
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
