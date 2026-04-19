use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use pyo3::prelude::*;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::components::bus::bus::Bus;
use crate::components::bus::dims::ProducerDim;
use crate::components::bus::envelope::{Envelope, ProducerHandle};
use crate::components::endpoint::Consumer;
use crate::components::message::{BaseMessage, Message};

type PendingMessages = Arc<Mutex<Vec<(Uuid, Vec<String>, Vec<u8>)>>>;

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
    /// `require_from`: if provided, only messages from that producer name are delivered.
    #[pyo3(signature = (callback, require_from=None))]
    pub fn register_consumer(&mut self, callback: PyObject, require_from: Option<String>) {
        let callback = Arc::new(callback);
        let pending: PendingMessages = Arc::new(Mutex::new(Vec::new()));
        self.consumer_queues.push((Arc::clone(&callback), Arc::clone(&pending)));
        let wrapper = PyConsumerWrapper { pending };
        let builder = self.bus.register_consumer(Box::new(wrapper));
        if let Some(name) = require_from {
            builder.require(ProducerDim::only(name));
        }
    }

    /// Send a message from the named producer. The producer is registered lazily on first use.
    pub fn send(&mut self, producer_name: String, msg: &PyMessage) -> PyResult<()> {
        let uuid = msg.inner.get_uuid();
        let labels = msg.inner.get_labels().to_vec();
        let data = msg.inner.get_bytes().to_vec();

        if !self.handles.contains_key(&producer_name) {
            let h = self.bus.make_handle(&producer_name);
            self.handles.insert(producer_name.clone(), h);
        }

        self.handles[&producer_name]
            .try_make_send(Box::new(BaseMessage::new(Some(uuid), Some(labels), Some(data))))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Drain all pending messages and invoke Python callbacks.
    ///
    /// If a callback raises, its undelivered messages are re-queued for the next drain
    /// and the first exception is returned after all consumers have been serviced.
    pub fn drain(&mut self, py: Python<'_>) -> PyResult<()> {
        py.allow_threads(|| self.rt.block_on(self.bus.drain()));

        let mut first_err: Option<PyErr> = None;
        for (callback, pending) in &self.consumer_queues {
            let msgs: Vec<_> = pending.lock().unwrap().drain(..).collect();
            for (i, (uuid, labels, data)) in msgs.iter().enumerate() {
                let result = Py::new(py, PyMessage {
                    inner: Box::new(BaseMessage::new(
                        Some(*uuid),
                        Some(labels.clone()),
                        Some(data.clone()),
                    )),
                })
                .and_then(|py_msg| callback.call1(py, (py_msg,)).map(|_| ()));

                if let Err(e) = result {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                    pending.lock().unwrap().extend(msgs[i..].iter().cloned());
                    break;
                }
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    pub fn is_empty(&self) -> bool {
        self.bus.is_empty()
    }
}

// ── PyConsumerWrapper ────────────────────────────────────────────────────────

struct PyConsumerWrapper {
    pending: PendingMessages,
}

impl Consumer for PyConsumerWrapper {
    fn available(&self) -> bool {
        true
    }

    fn validate(&self, _envelope: &Envelope) -> bool {
        true
    }

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let uuid = message.get_uuid();
        let labels = message.get_labels().to_vec();
        let data = message.get_bytes().to_vec();
        let pending = Arc::clone(&self.pending);
        Box::pin(async move {
            pending.lock().unwrap().push((uuid, labels, data));
        })
    }
}
