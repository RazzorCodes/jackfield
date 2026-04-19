use std::fmt;
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

use crate::components::bus::error::JackfieldError;
use crate::components::bus::throttle::TokenBucket;
use crate::components::message::Message;
use tokio::sync::mpsc;
use tokio::time::sleep;

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
    throttle: Option<Arc<tokio::sync::Mutex<TokenBucket>>>,
}

impl ProducerHandle {
    pub(super) fn new(
        origin: ProducerId,
        sender: mpsc::Sender<Envelope>,
        throttle: Option<Arc<tokio::sync::Mutex<TokenBucket>>>,
    ) -> Self {
        ProducerHandle { origin, sender, throttle }
    }

    /// Non-blocking send: returns `Err(ChannelFull)` immediately if the channel has no space.
    /// Intended for synchronous callers (e.g. Python) that cannot await.
    pub fn try_make_send(&self, msg: Box<dyn Message>) -> Result<(), JackfieldError> {
        self.sender
            .try_send(Envelope { origin: self.origin.clone(), message: msg })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => JackfieldError::ChannelFull,
                mpsc::error::TrySendError::Closed(_) => JackfieldError::ChannelClosed,
            })
    }

    pub fn make_send(&self, msg: Box<dyn Message>) -> impl Future<Output = Result<(), JackfieldError>> + Send + 'static {
        let sender = self.sender.clone();
        let origin = self.origin.clone();
        let throttle = self.throttle.clone();
        async move {
            wait_for_token(&throttle).await;
            sender
                .send(Envelope { origin, message: msg })
                .await
                .map_err(|_| JackfieldError::ChannelClosed)
        }
    }

    /// Like `make_send` but uses an explicitly provided origin — for shared handles across multiple
    /// connections (e.g. gRPC) where each connection needs its own distinct `ProducerId`.
    pub fn make_send_with_origin(
        &self,
        origin: ProducerId,
        msg: Box<dyn Message>,
    ) -> impl Future<Output = Result<(), JackfieldError>> + Send + 'static {
        let sender = self.sender.clone();
        let throttle = self.throttle.clone();
        async move {
            wait_for_token(&throttle).await;
            sender
                .send(Envelope { origin, message: msg })
                .await
                .map_err(|_| JackfieldError::ChannelClosed)
        }
    }
}

async fn wait_for_token(throttle: &Option<Arc<tokio::sync::Mutex<TokenBucket>>>) {
    if let Some(bucket) = throttle {
        loop {
            // Lock, check, unlock — then sleep outside the lock if needed.
            let wait = bucket.lock().await.try_acquire();
            match wait {
                None => break,
                Some(d) => sleep(d).await,
            }
        }
    }
}
