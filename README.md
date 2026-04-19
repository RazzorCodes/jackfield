# Jackfield

In-process message bus with affinity-based routing. Producers fire and forget; consumers register routing constraints and Jackfield picks the best match for each message.

## How it works

Producers send messages onto the bus. Consumers attach with routing constraints: hard requirements (`require`) that filter them out entirely, or soft preferences (`prefer`) that affect their score. The highest-scoring available consumer wins.

```rust
let mut bus = Bus::default();

bus.register_consumer(Box::new(my_consumer))
    .require(ProducerDim::only("sensor_a"));

bus.register_consumer(Box::new(fallback))
    .prefer(LabelDim::any_of(["temperature"]), 2.0);

bus.register_producer(&mut sensor_a);
bus.drain().await;
```

## Routing dimensions

| Dimension | |
|---|---|
| `ProducerDim` | match by sender name: `only`, `any_of`, `none_of` |
| `LabelDim` | score by label overlap: `any_of`, `all_of`, `none_of` |
| `SizeDim` | filter by payload size: `at_most`, `at_least`, `between` |

Custom dimensions implement the `Dimension` trait. The `observe` hook gets called with the routing outcome (Consumed / Skipped / Busy / Vetoed) so dimensions can update internal state, which is the entry point for adaptive routing.

## Network endpoints

- **gRPC**: bidirectional streaming, protobuf wire format
- **WebSocket**: binary frames, same protobuf format

Both fan out consumed messages to connected clients and can filter by label.

## Python

```python
bus = jackfield.MessageBus()
bus.register_consumer(lambda m: print(m.get_labels()), require_from="sensor")
bus.send("sensor", jackfield.Message(["temperature"], data))
bus.drain()
```

## Other bits

- Token bucket throttling per producer (`register_producer_throttled`)
- Unhandled messages are held in a pending buffer and retried on the next drain (`max_pending` caps it)
- `bus.dispatch()` for a continuous loop instead of manual drains

## Build

```sh
cargo test

# python (needs maturin in .venv)
make dev-py
```
