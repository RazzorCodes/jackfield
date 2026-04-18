use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use pyo3::prelude::*;
use tokio::runtime::Runtime;

use crate::components::bus::bus::Bus;
use crate::components::bus::envelope::{Envelope, ProducerHandle};
use crate::components::endpoint::Consumer;
use crate::components::message::{BaseMessage, Message};

// ── PyMessage ────────────────────────────────────────────────────────────────

#[pyclass(name = "Message")]
pub struct PyMessage {
    pub inner: Box<dyn Message>,
}

#[pymethods]
impl PyMessage {
    #[new]
    pub fn new(labels: Vec<String>, data: Vec<u8>) -> PyResult<Self> {
        Ok(PyMessage {
            inner: Box::new(BaseMessage::new(None, Some(labels), Some(data))),
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

// ── PyMessageBus ─────────────────────────────────────────────────────────────

#[pyclass(name = "MessageBus")]
pub struct PyMessageBus {
    rt: Runtime,
    bus: Bus,
    handles: HashMap<String, ProducerHandle>,
}

#[pymethods]
impl PyMessageBus {
    #[new]
    pub fn new() -> PyResult<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        Ok(Self {
            rt,
            bus: Bus::default(),
            handles: HashMap::new(),
        })
    }

    /// Register a Python callable as a consumer.
    /// `accept_from`: if provided, only messages from that producer name are delivered.
    #[pyo3(signature = (callback, accept_from=None))]
    pub fn register_consumer(&mut self, callback: PyObject, accept_from: Option<String>) {
        self.bus.register_consumer(Box::new(PyConsumerWrapper {
            callback: Arc::new(callback),
            accept_from,
        }));
    }

    /// Send a message from the named producer. The producer is registered lazily on first use.
    pub fn send(&mut self, py: Python<'_>, producer_name: String, msg: &PyMessage) -> PyResult<()> {
        let labels = msg.inner.get_labels().to_vec();
        let data = msg.inner.get_bytes().to_vec();

        if !self.handles.contains_key(&producer_name) {
            let h = self.bus.make_handle(&producer_name);
            self.handles.insert(producer_name.clone(), h);
        }

        let fut = self.handles[&producer_name].make_send(
            Box::new(BaseMessage::new(None, Some(labels), Some(data))),
        );

        py.allow_threads(|| self.rt.block_on(fut))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Drain all pending messages through registered consumers.
    /// Unhandled messages remain in the bus (same semantics as the Rust API).
    pub fn drain(&mut self, py: Python<'_>) {
        let fut = self.bus.drain();
        py.allow_threads(|| self.rt.block_on(fut));
    }

    pub fn is_empty(&self) -> bool {
        self.bus.is_empty()
    }
}

// ── PyConsumerWrapper ────────────────────────────────────────────────────────

struct PyConsumerWrapper {
    callback: Arc<PyObject>,
    accept_from: Option<String>,
}

impl Consumer for PyConsumerWrapper {
    fn available(&self) -> bool {
        true
    }

    fn validate(&self, envelope: &Envelope) -> bool {
        match &self.accept_from {
            Some(name) => envelope.origin.as_str() == name,
            None => true,
        }
    }

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let callback = Arc::clone(&self.callback);
        let labels = message.get_labels().to_vec();
        let data = message.get_bytes().to_vec();
        Box::pin(async move {
            Python::with_gil(|py| {
                let py_msg = Py::new(py, PyMessage {
                    inner: Box::new(BaseMessage::new(None, Some(labels), Some(data))),
                })
                .expect("PyMessage allocation failed");
                let _ = callback.call1(py, (py_msg,));
            });
        })
    }
}
