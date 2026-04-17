use std::fmt;
use std::future::Future;
use std::ops::Deref;

use crate::components::bus::error::JackfieldError;
use crate::components::message::Message;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProducerId(pub String);

impl ProducerId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ProducerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for ProducerId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProducerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub struct Envelope {
    pub origin: ProducerId,
    pub message: Box<dyn Message>,
}

pub struct ProducerHandle {
    origin: ProducerId,
    sender: mpsc::Sender<Envelope>,
}

impl ProducerHandle {
    pub(super) fn new(origin: ProducerId, sender: mpsc::Sender<Envelope>) -> Self {
        ProducerHandle { origin, sender }
    }

    /// Returns an owned future (clones sender + origin) so callers can return `impl Future + Send + 'static`.
    pub fn make_send(&self, msg: Box<dyn Message>) -> impl Future<Output = Result<(), JackfieldError>> + Send + 'static {
        let sender = self.sender.clone();
        let origin = self.origin.clone();
        async move {
            sender
                .send(Envelope { origin, message: msg })
                .await
                .map_err(|_| JackfieldError::ChannelClosed)
        }
    }
}
