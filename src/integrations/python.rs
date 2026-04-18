use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use pyo3::prelude::*;
use tokio::runtime::Runtime;

use crate::components::bus::bus::Bus;
use crate::components::bus::envelope::{Envelope, ProducerHandle};
use crate::components::endpoint::Consumer;
use crate::components::message::{BaseMessage, Message};

type PendingMessages = Arc<Mutex<Vec<(Vec<String>, Vec<u8>)>>>;

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
    /// Parallel to the bus consumer registry: (callback, shared pending queue).
    consumer_queues: Vec<(Arc<PyObject>, PendingMessages)>,
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
            consumer_queues: Vec::new(),
        })
    }

    /// Register a Python callable as a consumer.
    /// `accept_from`: if provided, only messages from that producer name are delivered.
    #[pyo3(signature = (callback, accept_from=None))]
    pub fn register_consumer(&mut self, callback: PyObject, accept_from: Option<String>) {
        let callback = Arc::new(callback);
        let pending: PendingMessages = Arc::new(Mutex::new(Vec::new()));
        self.consumer_queues.push((Arc::clone(&callback), Arc::clone(&pending)));
        self.bus.register_consumer(Box::new(PyConsumerWrapper { accept_from, pending }));
    }

    /// Send a message from the named producer. The producer is registered lazily on first use.
    pub fn send(&mut self, producer_name: String, msg: &PyMessage) -> PyResult<()> {
        let labels = msg.inner.get_labels().to_vec();
        let data = msg.inner.get_bytes().to_vec();

        if !self.handles.contains_key(&producer_name) {
            let h = self.bus.make_handle(&producer_name);
            self.handles.insert(producer_name.clone(), h);
        }

        self.handles[&producer_name]
            .try_make_send(Box::new(BaseMessage::new(None, Some(labels), Some(data))))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Drain all pending messages through registered consumers, then invoke Python
    /// callbacks in Python context. Exceptions raised by callbacks propagate as `RuntimeError`.
    pub fn drain(&mut self, py: Python<'_>) -> PyResult<()> {
        // Run the async drain without holding the GIL.
        py.allow_threads(|| self.rt.block_on(self.bus.drain()));

        // Now call Python callbacks — GIL is held, no async runtime involved.
        for (callback, pending) in &self.consumer_queues {
            let msgs: Vec<_> = pending.lock().unwrap().drain(..).collect();
            for (labels, data) in msgs {
                let py_msg = Py::new(py, PyMessage {
                    inner: Box::new(BaseMessage::new(None, Some(labels), Some(data))),
                })?;
                callback.call1(py, (py_msg,))?;
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.bus.is_empty()
    }
}

// ── PyConsumerWrapper ────────────────────────────────────────────────────────

struct PyConsumerWrapper {
    accept_from: Option<String>,
    pending: PendingMessages,
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
        let labels = message.get_labels().to_vec();
        let data = message.get_bytes().to_vec();
        let pending = Arc::clone(&self.pending);
        Box::pin(async move {
            pending.lock().unwrap().push((labels, data));
        })
    }
}
