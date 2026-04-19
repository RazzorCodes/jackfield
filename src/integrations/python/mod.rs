// Python integration module index; re-exports pyclass types for registration in the extension module.
pub mod integration;
pub use integration::{PyDim, PyLabelDim, PyMessage, PyMessageBus, PyProducerDim, PySizeDim};
