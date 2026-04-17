use crate::components::bus::bus::Bus;
use crate::components::bus::envelope::Envelope;
use crate::components::endpoint::Consumer;
use crate::components::message::{BaseMessage, Message};
use pyo3::prelude::*;

#[pymodule]
fn jackfield(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
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
    pub inner: Bus,
}

#[pymethods]
impl PyMessageBus {
    #[new]
    fn new() -> Self {
        Self {
            inner: Bus::default(),
        }
    }
}

struct PyConsumerWrapper {
    inner: PyObject,
}

impl Consumer for PyConsumerWrapper {
    fn available(&self) -> bool {
        true
    }
    fn validate(&self, _envelope: &Envelope) -> bool {
        true
    }
    fn consume(&mut self, _message: Box<dyn Message>) {}
}
