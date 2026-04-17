use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jackfield::components::bus::bus::Bus;
use jackfield::components::bus::envelope::{Envelope, ProducerId};
use jackfield::components::bus::throttle::Throttle;
use jackfield::components::endpoint::{Consumer, Endpoint, EndpointType, Producer};
use jackfield::components::message::{BaseMessage, Message};

// --- helpers ---

fn msg(labels: &[&str]) -> Box<dyn Message> {
    Box::new(BaseMessage::new(
        None,
        Some(labels.iter().map(|s| s.to_string()).collect()),
        None,
    ))
}

struct Collector {
    messages: Arc<Mutex<Vec<Vec<String>>>>,
    accept_from: Option<ProducerId>,
}

impl Collector {
    fn new(store: Arc<Mutex<Vec<Vec<String>>>>) -> Self {
        Collector { messages: store, accept_from: None }
    }

    fn only_from(mut self, id: &str) -> Self {
        self.accept_from = Some(ProducerId(id.to_string()));
        self
    }
}

impl Consumer for Collector {
    fn available(&self) -> bool {
        true
    }

    fn validate(&self, envelope: &Envelope) -> bool {
        match &self.accept_from {
            Some(id) => &envelope.origin == id,
            None => true,
        }
    }

    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.messages
            .lock()
            .unwrap()
            .push(message.get_labels().to_vec());
        Box::pin(async {})
    }
}

// --- tests ---

#[tokio::test]
async fn fan_in_from_multiple_producers() {
    let received = Arc::new(Mutex::new(vec![]));
    let mut bus = Bus::default();

    bus.register_consumer(Box::new(Collector::new(received.clone())));

    let mut ep_a = Endpoint::new("sensor_a", EndpointType::PRODUCER);
    let mut ep_b = Endpoint::new("sensor_b", EndpointType::PRODUCER);
    bus.register_producer(&mut ep_a);
    bus.register_producer(&mut ep_b);

    ep_a.send_bus(msg(&["temp"])).await.unwrap();
    ep_b.send_bus(msg(&["pressure"])).await.unwrap();
    ep_a.send_bus(msg(&["humidity"])).await.unwrap();

    bus.drain().await;

    let got = received.lock().unwrap();
    assert_eq!(got.len(), 3);
}

#[tokio::test]
async fn route_by_producer_origin() {
    let from_a = Arc::new(Mutex::new(vec![]));
    let from_b = Arc::new(Mutex::new(vec![]));
    let mut bus = Bus::default();

    bus.register_consumer(Box::new(Collector::new(from_a.clone()).only_from("ep_a")));
    bus.register_consumer(Box::new(Collector::new(from_b.clone()).only_from("ep_b")));

    let mut ep_a = Endpoint::new("ep_a", EndpointType::PRODUCER);
    let mut ep_b = Endpoint::new("ep_b", EndpointType::PRODUCER);
    bus.register_producer(&mut ep_a);
    bus.register_producer(&mut ep_b);

    ep_a.send_bus(msg(&["from_a"])).await.unwrap();
    ep_a.send_bus(msg(&["also_from_a"])).await.unwrap();
    ep_b.send_bus(msg(&["from_b"])).await.unwrap();

    bus.drain().await;

    assert_eq!(from_a.lock().unwrap().len(), 2);
    assert_eq!(from_b.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn origin_is_at_routing_layer_not_in_message() {
    let validated_origins: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let consumed_labels: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));

    struct Probe {
        origins: Arc<Mutex<Vec<String>>>,
        labels: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl Consumer for Probe {
        fn available(&self) -> bool { true }

        fn validate(&self, envelope: &Envelope) -> bool {
            self.origins.lock().unwrap().push(envelope.origin.as_str().to_string());
            true
        }

        fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.labels.lock().unwrap().push(message.get_labels().to_vec());
            Box::pin(async {})
        }
    }

    let mut bus = Bus::default();
    bus.register_consumer(Box::new(Probe {
        origins: validated_origins.clone(),
        labels: consumed_labels.clone(),
    }));

    let mut ep = Endpoint::new("identified_producer", EndpointType::PRODUCER);
    bus.register_producer(&mut ep);
    ep.send_bus(msg(&["event"])).await.unwrap();

    bus.drain().await;

    let origins = validated_origins.lock().unwrap();
    let labels = consumed_labels.lock().unwrap();

    assert_eq!(origins[0], "identified_producer");
    assert_eq!(labels[0], vec!["event"]);
}

#[tokio::test]
async fn unhandled_messages_stay_in_bus() {
    struct RejectAll;
    impl Consumer for RejectAll {
        fn available(&self) -> bool { true }
        fn validate(&self, _: &Envelope) -> bool { false }
        fn consume(&mut self, _: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            Box::pin(async {})
        }
    }

    let mut bus = Bus::default();
    bus.register_consumer(Box::new(RejectAll));

    let mut ep = Endpoint::new("ep", EndpointType::PRODUCER);
    bus.register_producer(&mut ep);

    ep.send_bus(msg(&[])).await.unwrap();
    ep.send_bus(msg(&[])).await.unwrap();

    bus.drain().await;

    assert!(!bus.is_empty(), "unhandled messages must remain in the bus");
}

#[tokio::test]
async fn dispatch_processes_messages_concurrently() {
    let received = Arc::new(Mutex::new(vec![]));
    let received_clone = received.clone();

    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    let done_tx = Arc::new(Mutex::new(Some(done_tx)));

    struct SignalOnThird {
        store: Arc<Mutex<Vec<Vec<String>>>>,
        signal: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    }

    impl Consumer for SignalOnThird {
        fn available(&self) -> bool { true }
        fn validate(&self, _: &Envelope) -> bool { true }
        fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            let mut store = self.store.lock().unwrap();
            store.push(message.get_labels().to_vec());
            if store.len() >= 3 {
                if let Some(tx) = self.signal.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
            Box::pin(async {})
        }
    }

    let mut bus = Bus::default();
    bus.register_consumer(Box::new(SignalOnThird {
        store: received_clone,
        signal: done_tx,
    }));

    let mut ep = Endpoint::new("live_producer", EndpointType::PRODUCER);
    bus.register_producer(&mut ep);

    let dispatch_handle = tokio::spawn(async move {
        bus.dispatch().await;
    });

    ep.send_bus(msg(&["a"])).await.unwrap();
    ep.send_bus(msg(&["b"])).await.unwrap();
    ep.send_bus(msg(&["c"])).await.unwrap();

    tokio::time::timeout(Duration::from_secs(1), done_rx)
        .await
        .expect("dispatch timed out")
        .unwrap();

    dispatch_handle.abort();

    assert_eq!(received.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn send_without_registration_errors() {
    let mut ep = Endpoint::new("unattached", EndpointType::PRODUCER);
    let result = ep.send_bus(msg(&[])).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn endpoint_with_both_flags_routes_correctly() {
    let received = Arc::new(Mutex::new(vec![]));
    let mut bus = Bus::default();

    let mut relay = Endpoint::new("relay", EndpointType::PRODUCER | EndpointType::CONSUMER);
    bus.register_producer(&mut relay);
    bus.register_consumer(Box::new(Collector::new(received.clone()).only_from("relay")));

    relay.send_bus(msg(&["relayed"])).await.unwrap();
    bus.drain().await;

    assert_eq!(received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn throttle_limits_send_rate() {
    // 10 msg/sec, burst=1 => first token is free, then ~100ms per subsequent token.
    // Sending 5 messages: 1 free + 4 * 100ms = ~400ms minimum.
    let timestamps: Arc<Mutex<Vec<tokio::time::Instant>>> = Arc::new(Mutex::new(vec![]));
    let timestamps_clone = timestamps.clone();

    struct TimestampCollector {
        ts: Arc<Mutex<Vec<tokio::time::Instant>>>,
    }

    impl Consumer for TimestampCollector {
        fn available(&self) -> bool { true }
        fn validate(&self, _: &Envelope) -> bool { true }
        fn consume(&mut self, _: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.ts.lock().unwrap().push(tokio::time::Instant::now());
            Box::pin(async {})
        }
    }

    let mut bus = Bus::default();
    bus.register_consumer(Box::new(TimestampCollector { ts: timestamps_clone }));

    let mut ep = Endpoint::new("throttled_producer", EndpointType::PRODUCER);
    bus.register_producer_throttled(&mut ep, Throttle::new(10, 1));

    let start = tokio::time::Instant::now();

    for _ in 0..5 {
        ep.send_bus(msg(&["tick"])).await.unwrap();
    }

    let elapsed_send = start.elapsed();

    // The throttle delays happen on send_bus, so the 5 sends themselves
    // should have taken >= 400ms (burst=1 means first is free, 4 more at 100ms each).
    assert!(
        elapsed_send >= Duration::from_millis(380),
        "Expected send phase >= 380ms, got {:?}",
        elapsed_send
    );

    bus.drain().await;

    let ts = timestamps.lock().unwrap();
    assert_eq!(ts.len(), 5, "All 5 messages should be consumed");
}
