use std::future::Future;

use crate::components::bus::envelope::{Envelope, ProducerHandle};
use crate::components::bus::error::JackfieldError;
use crate::components::message::Message;

pub trait Handler: Send + Sync {
    fn handle(&mut self) -> Option<Box<dyn Message>>;
}

pub trait Consumer: Sync + Send {
    fn available(&self) -> bool;
    fn validate(&self, envelope: &Envelope) -> bool;
    fn consume(&mut self, message: Box<dyn Message>);
}

pub trait Producer {
    fn name(&self) -> &str;
    fn attach(&mut self, handle: ProducerHandle);
    fn send_bus(&mut self, msg: Box<dyn Message>) -> impl Future<Output = Result<(), JackfieldError>> + Send;
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
    handle: Option<ProducerHandle>,
    consumer_handler: Option<Box<dyn Consumer>>,
    producer_handler: Option<Box<dyn Handler>>,
}

impl Endpoint {
    pub fn new(name: impl Into<String>, flags: EndpointType) -> Self {
        Endpoint {
            name: name.into(),
            flags,
            handle: None,
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
    fn name(&self) -> &str {
        &self.name
    }

    fn attach(&mut self, handle: ProducerHandle) {
        self.handle = Some(handle);
    }

    fn send_bus(&mut self, msg: Box<dyn Message>) -> impl Future<Output = Result<(), JackfieldError>> + Send {
        let send_op = if self.flags.contains(EndpointType::PRODUCER) {
            self.handle.as_ref().map(|h| h.make_send(msg))
        } else {
            None
        };
        async move { send_op.ok_or(JackfieldError::NotRegistered)?.await }
    }
}

impl Consumer for Endpoint {
    fn available(&self) -> bool {
        self.consumer_handler.as_ref().is_some_and(|c| c.available())
    }

    fn validate(&self, envelope: &Envelope) -> bool {
        self.consumer_handler.as_ref().is_some_and(|c| c.validate(envelope))
    }

    fn consume(&mut self, message: Box<dyn Message>) {
        if let Some(handler) = &mut self.consumer_handler {
            handler.consume(message);
        }
    }
}
