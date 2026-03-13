use crate::components::message::{BaseMessage, Message};
use crate::components::message_bus::{Consumer, MessageBus as CoreBus};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::VecDeque;

#[pymodule]
// The function name MUST be the module name
fn jackfield(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMessageBus>()?;
    m.add_class::<PyMessage>()?;
    Ok(())
}

#[pyclass(name = "Message")]
pub struct PyMessage {
    pub inner: Box<dyn Message>,
}

#[pymethods]
impl PyMessage {
    #[new]
    pub fn new(labels: Vec<String>, data: Vec<u8>) -> PyResult<Self> {
        let msg = BaseMessage::new(None, Some(labels), Some(data));
        Ok(PyMessage {
            inner: Box::new(msg),
        })
    }

    pub fn get_labels(&self) -> Vec<String> {
        self.inner.get_labels().to_vec()
    }

    pub fn get_bytes(&self) -> Vec<u8> {
        self.inner.get_bytes().to_vec()
    }

    pub fn get_uuid(&self) -> String {
        self.inner.get_uuid().to_string()
    }
}

#[pyclass(name = "MessageBus")]
pub struct PyMessageBus {
    pub inner: CoreBus,
}

#[pymethods]
impl PyMessageBus {
    #[new] // Silence the deprecation warning
    fn new() -> Self {
        Self {
            // Ensure MessageBus has a 'new' method or use a struct literal
            inner: CoreBus::new(),
        }
    }
}

// Fixed: Added the missing wrapper struct
struct PyConsumerWrapper {
    inner: PyObject,
}

impl Consumer for PyConsumerWrapper {
    fn validate(&self, _message: &Box<dyn Message>) -> bool {
        true
    }
    fn consume(&self, _message: Box<dyn Message>) {}
}
