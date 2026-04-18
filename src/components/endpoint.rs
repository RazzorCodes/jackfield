use std::future::Future;
use std::pin::Pin;

use crate::components::bus::envelope::{Envelope, ProducerHandle};
use crate::components::bus::error::JackfieldError;
use crate::components::message::Message;

pub trait Consumer: Sync + Send {
    fn available(&self) -> bool;
    fn validate(&self, envelope: &Envelope) -> bool;
    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
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

pub use crate::components::endpoints::direct::Endpoint;
