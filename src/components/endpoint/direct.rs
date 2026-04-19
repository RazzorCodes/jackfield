// In-process Endpoint: implements both Producer and Consumer for local use within the same binary.
use std::future::Future;
use std::pin::Pin;

use crate::components::router::envelope::{Envelope, ProducerHandle};
use crate::components::bus::error::JackfieldError;
use crate::components::message::Message;
use crate::components::endpoint::{Consumer, Producer, EndpointType};

pub struct Endpoint {
    name: String,
    flags: EndpointType,
    handle: Option<ProducerHandle>,
    consumer_handler: Option<Box<dyn Consumer>>,
}

impl Endpoint {
    pub fn new(name: impl Into<String>, flags: EndpointType) -> Self {
        Endpoint {
            name: name.into(),
            flags,
            handle: None,
            consumer_handler: None,
        }
    }

    pub fn set_consumer(mut self, consumer: impl Consumer + 'static) -> Self {
        self.consumer_handler = Some(Box::new(consumer));
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
            self.handle.as_ref().map(|h: &ProducerHandle| h.make_send(msg))
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

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        if let Some(handler) = &mut self.consumer_handler {
            handler.consume(message)
        } else {
            Box::pin(async {})
        }
    }
}
