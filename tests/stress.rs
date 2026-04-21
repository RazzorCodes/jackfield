use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jackfield::{
    BaseMessage, Bus, Consumer, Dimension, DimState, DispatchEvent, Endpoint, EndpointType,
    Envelope, Message, Producer, Throttle, Verdict,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn msg(labels: &[&str], data: &[u8]) -> Box<dyn Message> {
    Box::new(BaseMessage::new(
        None,
        Some(labels.iter().map(|s| s.to_string()).collect()),
        Some(data.to_vec()),
    ))
}

fn report(label: &str, elapsed: Duration, messages: usize) {
    let throughput = messages as f64 / elapsed.as_secs_f64();
    println!("[{label}] {messages} msgs in {elapsed:.2?} ({throughput:.0} msg/s)");
}

struct CountConsumer {
    count: Arc<AtomicUsize>,
}

impl Consumer for CountConsumer {
    fn available(&self) -> bool { true }
    fn validate(&self, _: &Envelope) -> bool { true }
    fn consume(&mut self, _: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.count.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

struct RecordConsumer {
    labels: Arc<Mutex<Vec<String>>>,
}

impl Consumer for RecordConsumer {
    fn available(&self) -> bool { true }
    fn validate(&self, _: &Envelope) -> bool { true }
    fn consume(&mut self, message: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        if let Some(first) = message.get_labels().first() {
            self.labels.lock().unwrap().push(first.clone());
        }
        Box::pin(async {})
    }
}

struct RejectAll;

impl Consumer for RejectAll {
    fn available(&self) -> bool { true }
    fn validate(&self, _: &Envelope) -> bool { false }
    fn consume(&mut self, _: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }
}

struct ToggleConsumer {
    available: Arc<AtomicBool>,
    count: Arc<AtomicUsize>,
}

impl Consumer for ToggleConsumer {
    fn available(&self) -> bool { self.available.load(Ordering::Relaxed) }
    fn validate(&self, _: &Envelope) -> bool { true }
    fn consume(&mut self, _: Box<dyn Message>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.count.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

// ── AdaptiveDim ───────────────────────────────────────────────────────────────
//
// A test-local dimension that rewards consumption and penalises busy periods.
// Uses DimState.weight directly — no inner state needed.

struct AdaptiveDim;

impl Dimension for AdaptiveDim {
    fn evaluate(&self, _: &Envelope, _: &DimState) -> Verdict {
        Verdict::Score(1.0)
    }

    fn observe(&self, event: &DispatchEvent, state: &mut DimState) {
        match event {
            DispatchEvent::Consumed { .. } => {
                state.weight = (state.weight * 1.2).min(10.0);
            }
            DispatchEvent::Busy { .. } => {
                state.weight = (state.weight * 0.7).max(0.01);
            }
            _ => {}
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn throughput() {
    const PRODUCERS: usize = 4;
    const PER_PRODUCER: usize = 250;
    const TOTAL: usize = PRODUCERS * PER_PRODUCER;

    let count = Arc::new(AtomicUsize::new(0));
    let mut bus = Bus::new(TOTAL);

    bus.register_consumer(Box::new(CountConsumer { count: count.clone() }));

    let handles: Vec<_> = (0..PRODUCERS)
        .map(|i| bus.make_handle(format!("producer_{i}")))
        .collect();

    for handle in &handles {
        for _ in 0..PER_PRODUCER {
            handle.try_make_send(msg(&[], &[])).unwrap();
        }
    }

    let start = Instant::now();
    bus.drain().await;
    let elapsed = start.elapsed();

    assert_eq!(count.load(Ordering::Relaxed), TOTAL);
    assert!(bus.is_empty());
    assert!(elapsed < Duration::from_millis(500), "drain took {:?}", elapsed);

    report("throughput/drain", elapsed, TOTAL);
}

#[tokio::test]
async fn pending_queue_expiry() {
    let mut bus = Bus::new(512).max_pending(10);

    let reject_id = bus.register_consumer(Box::new(RejectAll)).id;

    let handle = bus.make_handle("producer");
    for i in 0..30u32 {
        handle.try_make_send(msg(&[&i.to_string()], &[])).unwrap();
    }

    let t0 = Instant::now();
    bus.drain().await;
    // All 30 went to pending; queue capped at 10, oldest 20 dropped.
    assert!(!bus.is_empty());

    bus.deregister_consumer(reject_id);

    let surviving = Arc::new(Mutex::new(Vec::<String>::new()));
    bus.register_consumer(Box::new(RecordConsumer { labels: surviving.clone() }));

    bus.drain().await;
    let elapsed = t0.elapsed();

    assert!(bus.is_empty());
    let kept = surviving.lock().unwrap();
    assert_eq!(kept.len(), 10, "expected exactly 10 surviving messages, got {}", kept.len());

    // The surviving messages should be the last 10 sent (oldest dropped).
    let expected: Vec<String> = (20..30u32).map(|i| i.to_string()).collect();
    assert_eq!(*kept, expected, "wrong messages survived: {:?}", *kept);

    report("pending_queue_expiry/total", elapsed, 30);
}

#[tokio::test]
async fn consumer_ranking() {
    let a_available = Arc::new(AtomicBool::new(true));
    let b_available = Arc::new(AtomicBool::new(true));
    let a_count = Arc::new(AtomicUsize::new(0));
    let b_count = Arc::new(AtomicUsize::new(0));

    let mut bus = Bus::default();
    let handle = bus.make_handle("producer");

    // A registered first → lower ID → wins tiebreaker when weights are equal.
    bus.register_consumer(Box::new(ToggleConsumer {
        available: a_available.clone(),
        count: a_count.clone(),
    }))
    .prefer(AdaptiveDim, 1.0);

    bus.register_consumer(Box::new(ToggleConsumer {
        available: b_available.clone(),
        count: b_count.clone(),
    }))
    .prefer(AdaptiveDim, 1.0);

    // Phase A: equal weights, A wins tiebreaker → A earns rank.
    for _ in 0..30 {
        handle.try_make_send(msg(&[], &[])).unwrap();
    }
    let t0 = Instant::now();
    bus.drain().await;
    report("consumer_ranking/phase_a", t0.elapsed(), 30);
    assert_eq!(a_count.load(Ordering::Relaxed), 30, "A should handle all messages while weights are equal");
    assert_eq!(b_count.load(Ordering::Relaxed), 0);

    // Phase B: A goes unavailable → gets Busy penalties, B earns rank.
    a_available.store(false, Ordering::Relaxed);
    for _ in 0..30 {
        handle.try_make_send(msg(&[], &[])).unwrap();
    }
    let t1 = Instant::now();
    bus.drain().await;
    report("consumer_ranking/phase_b", t1.elapsed(), 30);
    assert_eq!(a_count.load(Ordering::Relaxed), 30, "A must not have consumed during unavailability");
    assert_eq!(b_count.load(Ordering::Relaxed), 30, "B should handle all messages while A is unavailable");

    // Phase C: A restored, but B's weight now exceeds A's → B keeps preference.
    a_available.store(true, Ordering::Relaxed);
    for _ in 0..30 {
        handle.try_make_send(msg(&[], &[])).unwrap();
    }
    let t2 = Instant::now();
    bus.drain().await;
    report("consumer_ranking/phase_c", t2.elapsed(), 30);
    assert_eq!(b_count.load(Ordering::Relaxed), 60, "B should remain preferred after earning higher weight");
    assert_eq!(a_count.load(Ordering::Relaxed), 30, "A should not receive messages while B outranks it");
}

#[tokio::test]
async fn runtime_registration_deregistration() {
    let mut bus = Bus::default();
    let handle = bus.make_handle("producer");

    // Phase A: no consumers → messages go to pending.
    for _ in 0..5 {
        handle.try_make_send(msg(&[], &[])).unwrap();
    }
    bus.drain().await;
    assert!(!bus.is_empty(), "messages should be pending with no consumers");

    // Phase B: register A → pending messages are consumed.
    let a_count = Arc::new(AtomicUsize::new(0));
    let a_id = bus
        .register_consumer(Box::new(CountConsumer { count: a_count.clone() }))
        .id;
    bus.drain().await;
    assert_eq!(a_count.load(Ordering::Relaxed), 5);
    assert!(bus.is_empty());

    // Phase C: deregister A, register B → new messages go to B only.
    bus.deregister_consumer(a_id);
    let b_count = Arc::new(AtomicUsize::new(0));
    let b_id = bus
        .register_consumer(Box::new(CountConsumer { count: b_count.clone() }))
        .id;
    for _ in 0..5 {
        handle.try_make_send(msg(&[], &[])).unwrap();
    }
    bus.drain().await;
    assert_eq!(b_count.load(Ordering::Relaxed), 5);
    assert_eq!(a_count.load(Ordering::Relaxed), 5, "A must not receive after deregistration");
    assert!(bus.is_empty());

    // Phase D: deregister B → messages go to pending again.
    bus.deregister_consumer(b_id);
    for _ in 0..5 {
        handle.try_make_send(msg(&[], &[])).unwrap();
    }
    let t = Instant::now();
    bus.drain().await;
    report("runtime_reg_dereg/total", t.elapsed(), 15);
    assert!(!bus.is_empty(), "messages should be pending again after deregistering B");
}

#[tokio::test]
async fn throttle_limits_throughput() {
    const MSGS: usize = 10;
    let count = Arc::new(AtomicUsize::new(0));
    let mut bus = Bus::default();
    bus.register_consumer(Box::new(CountConsumer { count: count.clone() }));

    let mut ep = Endpoint::new("throttled", EndpointType::PRODUCER);
    // 20 msg/sec, burst=2: first 2 free, remaining 8 at 50ms/msg → ≥ 400ms.
    bus.register_producer_throttled(&mut ep, Throttle::new(20, 2));

    let start = Instant::now();
    for _ in 0..MSGS {
        ep.send_bus(msg(&[], &[])).await.unwrap();
    }
    let send_elapsed = start.elapsed();

    assert!(
        send_elapsed >= Duration::from_millis(380),
        "throttle should enforce ~400ms for 8 post-burst messages at 20/s, got {:?}",
        send_elapsed
    );

    let drain_start = Instant::now();
    bus.drain().await;
    let drain_elapsed = drain_start.elapsed();

    assert_eq!(count.load(Ordering::Relaxed), MSGS);

    report("throttle/send_phase", send_elapsed, MSGS);
    report("throttle/drain_phase", drain_elapsed, MSGS);
}

#[tokio::test]
async fn wide_fan_out_100k_messages_1k_consumers() {
    const CONSUMERS: usize = 1_000;
    const MESSAGES: usize = 100_000;
    // Channel must fit all messages so try_make_send never hits capacity.
    const CHANNEL_CAP: usize = MESSAGES;

    let total = Arc::new(AtomicUsize::new(0));
    let mut bus = Bus::new(CHANNEL_CAP);

    // Register 1 000 consumers, all unconditionally accept every message.
    // No dims — pure affinity-router scoring overhead across the full registry.
    for _ in 0..CONSUMERS {
        bus.register_consumer(Box::new(CountConsumer { count: total.clone() }));
    }

    let handle = bus.make_handle("producer");

    let enqueue_start = Instant::now();
    for _ in 0..MESSAGES {
        handle.try_make_send(msg(&[], &[])).unwrap();
    }
    let enqueue_elapsed = enqueue_start.elapsed();

    let drain_start = Instant::now();
    bus.drain().await;
    let drain_elapsed = drain_start.elapsed();

    assert_eq!(
        total.load(Ordering::Relaxed),
        MESSAGES,
        "all messages must be consumed"
    );
    assert!(bus.is_empty());

    report("wide_fan_out/enqueue", enqueue_elapsed, MESSAGES);
    report("wide_fan_out/drain  ", drain_elapsed, MESSAGES);
    report("wide_fan_out/total  ", enqueue_elapsed + drain_elapsed, MESSAGES);
    println!(
        "[wide_fan_out] registry: {CONSUMERS} consumers × {MESSAGES} messages = {} scoring ops",
        CONSUMERS * MESSAGES
    );
}
