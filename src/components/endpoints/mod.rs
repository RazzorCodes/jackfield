pub mod connection;
pub mod direct;
#[cfg(feature = "grpc")]
pub mod grpc;
#[cfg(feature = "websocket")]
pub mod ws;

pub use direct::Endpoint;
#[cfg(feature = "grpc")]
pub use grpc::{GrpcConsumer, GrpcEndpoint};
#[cfg(feature = "websocket")]
pub use ws::{WsConsumer, WsEndpoint};
