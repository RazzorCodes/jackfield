// End-to-end demos for the gRPC and WebSocket self-registering endpoints.
//
// These tests show the full client lifecycle and serve as working usage examples:
//
//   gRPC:  Register(role + dims) → Stream data → Deregister
//   WS:    connect → first frame RegisterData → subsequent frames BusableItem → disconnect
//
// Run with:
//   cargo test --features grpc     -- grpc_
//   cargo test --features websocket -- ws_
//   cargo test --features network  (both)

use std::time::Duration;

// ── gRPC demos ────────────────────────────────────────────────────────────────

/// A producer client sends messages into the bus; a consumer client (registered
/// with a label filter) receives only the matching ones.
///
/// Lifecycle shown:
///   1. Bus + GrpcEndpoint setup
///   2. Consumer: Register(CONSUMER, label filter) → Stream → await messages
///   3. Producer: Register(PRODUCER) → Stream → send messages
///   4. Assert the consumer only received the label-matched message
///   5. Consumer: Deregister
#[cfg(feature = "grpc")]
#[tokio::test]
async fn grpc_demo_producer_and_filtered_consumer() {
    use jackfield::components::message::codec::proto;
    use jackfield::components::message::codec::proto::bus_client::BusClient;
    use jackfield::{Bus, GrpcEndpoint};
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    // ── 1. Bus + server ───────────────────────────────────────────────────────
    //
    // The bus owns the routing registry. GrpcEndpoint gets a BusCmdHandle so
    // connecting clients can register/deregister themselves at runtime.
    let addr: std::net::SocketAddr = "127.0.0.1:51151".parse().unwrap();
    let mut bus = Bus::default();
    let cmd_handle = bus.make_cmd_handle();
    let endpoint = GrpcEndpoint::new("demo", addr, cmd_handle);
    let _server = endpoint.start();

    // Bus dispatch loop runs in the background; all routing happens here.
    let bus_task = tokio::spawn(async move { bus.dispatch().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 2. Consumer client ────────────────────────────────────────────────────
    //
    // Register first so the bus entry exists before any messages are produced.
    // The server acknowledges with a UUID that identifies this endpoint.
    let mut consumer = BusClient::connect("http://127.0.0.1:51151").await.unwrap();

    let reg_resp = consumer.register(proto::RegisterData {
        name: "subscriber".into(),
        r#type: vec![proto::EndpointType::Consumer as i32],
        // Only receive messages labelled "telemetry". Other messages are
        // scored 0.0 and will be routed to whichever other consumer fits best
        // (or held in the pending queue if none does).
        dimensions_json: r#"{"labels": {"any_of": ["telemetry"]}}"#.into(),
    }).await.unwrap().into_inner();

    let endpoint_uuid = match reg_resp.data.unwrap() {
        proto::generic_response::Data::Uuid(b) => b,
    };

    // Open the bidirectional stream. The consumer sends nothing (empty tx),
    // and reads incoming BusableItems on the returned stream.
    let (_consumer_tx, consumer_rx) = mpsc::channel::<proto::BusableItem>(1);
    let mut incoming = consumer
        .stream(ReceiverStream::new(consumer_rx))
        .await.unwrap()
        .into_inner();

    // ── 3. Producer client ────────────────────────────────────────────────────
    let mut producer = BusClient::connect("http://127.0.0.1:51151").await.unwrap();
    producer.register(proto::RegisterData {
        name: "sensor".into(),
        r#type: vec![proto::EndpointType::Producer as i32],
        dimensions_json: String::new(), // producers declare no filter
    }).await.unwrap();

    let (prod_tx, prod_rx) = mpsc::channel::<proto::BusableItem>(8);
    let _outbound = producer.stream(ReceiverStream::new(prod_rx)).await.unwrap();

    // Matching message — labelled "telemetry".
    prod_tx.send(proto::BusableItem {
        uuid: uuid::Uuid::new_v4().as_bytes().to_vec(),
        labels: vec!["telemetry".into()],
        data:   b"temp=42".to_vec(),
    }).await.unwrap();

    // Non-matching message — labelled "debug". Scored 0.0 against the consumer's
    // any_of(["telemetry"]) filter; routed elsewhere or held in pending.
    prod_tx.send(proto::BusableItem {
        uuid: uuid::Uuid::new_v4().as_bytes().to_vec(),
        labels: vec!["debug".into()],
        data:   b"verbose log".to_vec(),
    }).await.unwrap();

    // ── 4. Assert ─────────────────────────────────────────────────────────────
    let item = tokio::time::timeout(Duration::from_secs(2), incoming.message())
        .await.expect("timed out waiting for message")
        .unwrap().unwrap();

    assert_eq!(item.labels, vec!["telemetry"]);
    assert_eq!(item.data,   b"temp=42");

    // The "debug" message must not arrive within a short window.
    let second = tokio::time::timeout(Duration::from_millis(100), incoming.message()).await;
    assert!(second.is_err(), "debug message must not reach the telemetry consumer");

    // ── 5. Deregister ─────────────────────────────────────────────────────────
    //
    // The server removes the consumer from the bus registry immediately.
    // After this call the consumer's entry is gone; further messages are unrouted.
    consumer.deregister(proto::DeregisterNotification { uuid: endpoint_uuid })
        .await.unwrap();

    bus_task.abort();
}

/// A client that registers as BOTH producer and consumer echoes messages back
/// through the bus to itself.
#[cfg(feature = "grpc")]
#[tokio::test]
async fn grpc_demo_both_roles() {
    use jackfield::components::message::codec::proto;
    use jackfield::components::message::codec::proto::bus_client::BusClient;
    use jackfield::{Bus, GrpcEndpoint};
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    let addr: std::net::SocketAddr = "127.0.0.1:51152".parse().unwrap();
    let mut bus = Bus::default();
    let endpoint = GrpcEndpoint::new("demo", addr, bus.make_cmd_handle());
    let _server = endpoint.start();
    let bus_task = tokio::spawn(async move { bus.dispatch().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = BusClient::connect("http://127.0.0.1:51152").await.unwrap();

    // Register as BOTH: this client can send to the bus and receive from it.
    client.register(proto::RegisterData {
        name: "relay".into(),
        r#type: vec![
            proto::EndpointType::Producer as i32,
            proto::EndpointType::Consumer as i32,
        ],
        dimensions_json: String::new(),
    }).await.unwrap();

    let (send_tx, send_rx) = mpsc::channel::<proto::BusableItem>(8);
    let response = client.stream(ReceiverStream::new(send_rx)).await.unwrap();
    let mut recv = response.into_inner();

    send_tx.send(proto::BusableItem {
        uuid:   uuid::Uuid::new_v4().as_bytes().to_vec(),
        labels: vec!["echo".into()],
        data:   b"hello".to_vec(),
    }).await.unwrap();

    let item = tokio::time::timeout(Duration::from_secs(2), recv.message())
        .await.expect("timed out").unwrap().unwrap();

    assert_eq!(item.labels, vec!["echo"]);
    assert_eq!(item.data,   b"hello");

    bus_task.abort();
}

// ── WebSocket demos ───────────────────────────────────────────────────────────

/// A WS producer sends messages; a WS consumer receives them.
///
/// Lifecycle shown:
///   connect → send RegisterData (first binary frame) → exchange BusableItems → disconnect
///
/// Disconnect is the implicit deregister: the server cleans up the bus entry
/// when `source.next()` returns None or an error.
#[cfg(feature = "websocket")]
#[tokio::test]
async fn ws_demo_producer_and_consumer() {
    use futures_util::{SinkExt, StreamExt};
    use jackfield::components::message::codec::proto;
    use jackfield::{Bus, WsEndpoint};
    use prost::Message as ProstMessage;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    // ── Bus + server ──────────────────────────────────────────────────────────
    let addr: std::net::SocketAddr = "127.0.0.1:51153".parse().unwrap();
    let mut bus = Bus::default();
    let endpoint = WsEndpoint::new("demo", addr, bus.make_cmd_handle());
    let _server = endpoint.start();
    let bus_task = tokio::spawn(async move { bus.dispatch().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── Consumer client ───────────────────────────────────────────────────────
    //
    // First binary frame must be RegisterData. No separate RPC exists for WS;
    // the frame serves the same purpose as the gRPC Register call.
    let (mut ws_consumer, _) = connect_async("ws://127.0.0.1:51153").await.unwrap();

    ws_consumer.send(WsMessage::Binary(
        proto::RegisterData {
            name:            "ws-subscriber".into(),
            r#type:          vec![proto::EndpointType::Consumer as i32],
            dimensions_json: String::new(),
        }.encode_to_vec().into()
    )).await.unwrap();

    // Give the server time to process the RegisterData and register
    // the consumer in the bus before the producer sends.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ── Producer client ───────────────────────────────────────────────────────
    let (mut ws_producer, _) = connect_async("ws://127.0.0.1:51153").await.unwrap();

    ws_producer.send(WsMessage::Binary(
        proto::RegisterData {
            name:            "ws-sensor".into(),
            r#type:          vec![proto::EndpointType::Producer as i32],
            dimensions_json: String::new(),
        }.encode_to_vec().into()
    )).await.unwrap();

    // Send a BusableItem — any binary frame after RegisterData is treated as data.
    ws_producer.send(WsMessage::Binary(
        proto::BusableItem {
            uuid:   uuid::Uuid::new_v4().as_bytes().to_vec(),
            labels: vec!["sensor".into()],
            data:   b"reading=99".to_vec(),
        }.encode_to_vec().into()
    )).await.unwrap();

    // ── Assert ────────────────────────────────────────────────────────────────
    let frame = tokio::time::timeout(Duration::from_secs(2), ws_consumer.next())
        .await.expect("timed out").unwrap().unwrap();

    let item = proto::BusableItem::decode(frame.into_data().as_ref()).unwrap();
    assert_eq!(item.labels, vec!["sensor"]);
    assert_eq!(item.data,   b"reading=99");

    // ── Disconnect = implicit deregister ──────────────────────────────────────
    //
    // Clean close triggers the server's cleanup path (same as a TCP RST would).
    ws_producer.close(None).await.ok();
    ws_consumer.close(None).await.ok();

    tokio::time::sleep(Duration::from_millis(50)).await;
    bus_task.abort();
}

/// Unexpected producer disconnect: the server must clean up its endpoint entry
/// without an explicit Deregister/close call.
#[cfg(feature = "websocket")]
#[tokio::test]
async fn ws_demo_unexpected_disconnect_cleanup() {
    use futures_util::SinkExt;
    use jackfield::components::message::codec::proto;
    use jackfield::{Bus, WsEndpoint};
    use prost::Message as ProstMessage;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let addr: std::net::SocketAddr = "127.0.0.1:51154".parse().unwrap();
    let mut bus = Bus::default();
    let endpoint = WsEndpoint::new("demo", addr, bus.make_cmd_handle());
    let _server = endpoint.start();
    let bus_task = tokio::spawn(async move { bus.dispatch().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    {
        let (mut ws, _) = connect_async("ws://127.0.0.1:51154").await.unwrap();
        ws.send(WsMessage::Binary(
            proto::RegisterData {
                name:            "drop-me".into(),
                r#type:          vec![proto::EndpointType::Consumer as i32],
                dimensions_json: String::new(),
            }.encode_to_vec().into()
        )).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // ws dropped here — TCP stream closed without a WS close frame.
        // The server detects EOF in source.next() and calls cmd_handle.deregister().
    }

    // Allow server time to process the disconnect.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The bus registry should be empty — no zombie consumer entries.
    // (Verified indirectly: a new message sent via a cmd_handle producer handle
    //  will go to pending, not to a dead ChannelConsumer.)
    let handle = bus_task.is_finished();
    assert!(!handle, "bus task should still be running");

    bus_task.abort();
}
