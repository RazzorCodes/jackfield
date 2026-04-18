import jackfield
import pytest


# ── helpers ──────────────────────────────────────────────────────────────────

def msg(labels, data=b""):
    return jackfield.Message(labels, data)


# ── Message ───────────────────────────────────────────────────────────────────

def test_message_labels_and_bytes():
    m = msg(["a", "b"], b"hello")
    assert m.get_labels() == ["a", "b"]
    assert m.get_bytes() == b"hello"


def test_message_uuid_is_string():
    m = msg([])
    uuid = m.get_uuid()
    assert isinstance(uuid, str)
    # nil UUID for messages created without explicit uuid
    assert uuid == "00000000-0000-0000-0000-000000000000"


def test_message_empty_defaults():
    m = msg([])
    assert m.get_labels() == []
    assert m.get_bytes() == b""


# ── MessageBus construction ───────────────────────────────────────────────────

def test_bus_construction():
    bus = jackfield.MessageBus()
    assert bus is not None
    assert bus.is_empty()


# ── send + drain ──────────────────────────────────────────────────────────────

def test_send_and_drain_single_message():
    received = []
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: received.append(m.get_labels()))
    bus.send("producer", msg(["ping"]))
    bus.drain()
    assert received == [["ping"]]


def test_drain_delivers_message_payload():
    payloads = []
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: payloads.append(m.get_bytes()))
    bus.send("p", msg([], b"\xde\xad\xbe\xef"))
    bus.drain()
    assert payloads == [b"\xde\xad\xbe\xef"]


def test_bus_empty_after_drain():
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: None)
    bus.send("p", msg(["x"]))
    assert not bus.is_empty()
    bus.drain()
    assert bus.is_empty()


# ── fan-in ────────────────────────────────────────────────────────────────────

def test_fan_in_from_multiple_producers():
    received = []
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: received.append(m.get_labels()[0]))
    bus.send("sensor_a", msg(["temp"]))
    bus.send("sensor_b", msg(["pressure"]))
    bus.send("sensor_a", msg(["humidity"]))
    bus.drain()
    assert len(received) == 3
    assert set(received) == {"temp", "pressure", "humidity"}


# ── routing by origin ─────────────────────────────────────────────────────────

def test_routing_by_producer_origin():
    from_a, from_b = [], []
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: from_a.append(m.get_labels()[0]), accept_from="a")
    bus.register_consumer(lambda m: from_b.append(m.get_labels()[0]), accept_from="b")
    bus.send("a", msg(["x"]))
    bus.send("a", msg(["y"]))
    bus.send("b", msg(["z"]))
    bus.drain()
    assert from_a == ["x", "y"]
    assert from_b == ["z"]


def test_origin_does_not_appear_in_message_payload():
    """Consumer receives a clean message — producer name is not injected into labels or bytes."""
    received = []
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: received.append((m.get_labels(), m.get_bytes())))
    bus.send("identified_producer", msg(["event"], b"data"))
    bus.drain()
    assert received == [(["event"], b"data")]


# ── unhandled messages ────────────────────────────────────────────────────────

def test_unhandled_messages_stay_in_bus():
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: None, accept_from="nobody")
    bus.send("p", msg([]))
    bus.drain()
    assert not bus.is_empty()


# ── multiple drains ───────────────────────────────────────────────────────────

def test_multiple_sends_then_drain():
    received = []
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: received.append(m.get_labels()))
    for i in range(10):
        bus.send("p", msg([str(i)]))
    bus.drain()
    assert len(received) == 10
    assert [r[0] for r in received] == [str(i) for i in range(10)]


def test_drain_is_idempotent_when_empty():
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: None)
    bus.drain()
    bus.drain()
    assert bus.is_empty()


# ── edge cases & potential bugs ──────────────────────────────────────────────

def test_bus_full_raises_not_deadlocks():
    """
    Sending past capacity raises RuntimeError("Bus channel is full") immediately.
    It must never block, because a blocked send holds the pyo3 borrow and makes
    drain() on the same object impossible (would raise "Already borrowed").
    """
    bus = jackfield.MessageBus()
    sent = 0
    with pytest.raises(RuntimeError, match="full"):
        for i in range(300):
            bus.send("p", msg([str(i)]))
            sent += 1
    assert sent == 256, f"Expected exactly 256 to succeed before full, got {sent}"


def test_unhandled_messages_not_lost_when_full():
    """
    If the bus is full and we drain it, unhandled messages should be requeued.
    If requeueing fails because the bus is full, they might be lost.
    """
    bus = jackfield.MessageBus()
    # Fill the bus with unhandled messages
    for i in range(256):
        bus.send("p", msg([f"msg_{i}"]))
    
    # Bus is now full.
    assert not bus.is_empty()
    
    # Drain. This should try to requeue 256 messages.
    bus.drain()
    
    # Check if we still have 256 messages.
    # We can count them by draining into a consumer that accepts everything.
    received = []
    bus.register_consumer(lambda m: received.append(m.get_labels()[0]))
    bus.drain()
    
    assert len(received) == 256, f"Expected 256 messages, but got {len(received)}. Messages were lost during requeue!"
