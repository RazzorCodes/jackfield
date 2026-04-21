// Crate root. Re-exports the public API and wires up the optional Python extension module.
pub mod components;
pub mod integrations;

pub use components::{
    AffinityRouter, BaseMessage, BlindRouter, Bus, BusCmdHandle, Consumer, Dimension, DimState,
    DispatchEvent, Endpoint, EndpointType, Envelope, EventMeta, JackfieldError, LabelDim, Message,
    Producer, ProducerDim, ProducerHandle, ProducerId, RegistrationBuilder, Router, SizeDim,
    Throttle, Verdict,
};
#[cfg(feature = "grpc")]
pub use components::GrpcEndpoint;
#[cfg(feature = "websocket")]
pub use components::WsEndpoint;

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
#[pymodule]
fn jackfield(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<integrations::python::PyMessage>()?;
    m.add_class::<integrations::python::PyMessageBus>()?;
    m.add_class::<integrations::python::PyDim>()?;
    m.add_class::<integrations::python::PyProducerDim>()?;
    m.add_class::<integrations::python::PyLabelDim>()?;
    m.add_class::<integrations::python::PySizeDim>()?;
    Ok(())
}
