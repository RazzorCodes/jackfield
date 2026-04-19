pub mod components;
pub mod integrations;

pub use components::{BaseMessage, Bus, Consumer, Dimension, DimState, DispatchEvent, Endpoint, EndpointType, Envelope, EventMeta, JackfieldError, LabelDim, Message, Producer, ProducerDim, ProducerHandle, ProducerId, RegistrationBuilder, SizeDim, Throttle, Verdict};
#[cfg(feature = "grpc")]
pub use components::{GrpcConsumer, GrpcEndpoint};
#[cfg(feature = "websocket")]
pub use components::{WsConsumer, WsEndpoint};

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
