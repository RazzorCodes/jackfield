// Aggregates sub-modules and flattens the public API one level up.
pub mod bus;
pub mod endpoint;
pub mod message;
pub mod router;

pub use bus::{Bus, JackfieldError, RegistrationBuilder};
pub use endpoint::{Consumer, Endpoint, EndpointType, Producer, Throttle};
pub use message::{BaseMessage, Message};
pub use router::{
    AffinityRouter, BlindRouter, Router,
    Dimension, DimState, DispatchEvent, Envelope, EventMeta,
    LabelDim, ProducerDim, ProducerHandle, ProducerId, SizeDim, Verdict,
};
#[cfg(feature = "grpc")]
pub use endpoint::{GrpcConsumer, GrpcEndpoint};
#[cfg(feature = "websocket")]
pub use endpoint::{WsConsumer, WsEndpoint};
