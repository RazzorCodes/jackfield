import jackfield


# ── e2e demo ──────────────────────────────────────────────────────────────────

def test_e2e():
    """
    Sensor pipeline: route alerts by producer origin, metrics by label,
    and bulk payloads by size — all from different producers into one bus.
    """
    bus = jackfield.MessageBus()
    assert bus.is_empty()

    alerts, metrics, large_payloads = [], [], []

    bus.register_consumer(
        lambda m: alerts.append(m.get_labels()),
        require_from="alerting",
    )
    bus.register_consumer(
        lambda m: metrics.append(m.get_labels()),
        require=[jackfield.LabelDim.any_of(["metric"])],
    )
    bus.register_consumer(
        lambda m: large_payloads.append(len(m.get_bytes())),
        require=[jackfield.SizeDim.at_least(100)],
    )

    # fan-in from multiple producers
    bus.send("alerting", jackfield.Message(["alert"], b"disk_full"))
    bus.send("alerting", jackfield.Message(["alert"], b"cpu_high"))
    bus.send("monitoring", jackfield.Message(["metric"], b"cpu=42"))
    bus.send("monitoring", jackfield.Message(["heartbeat"], b"x" * 200))

    bus.drain()

    assert alerts == [["alert"], ["alert"]]
    assert metrics == [["metric"]]
    assert large_payloads == [200]
    assert bus.is_empty()


# ── api smoke tests ───────────────────────────────────────────────────────────

def test_message_api():
    m = jackfield.Message(["a", "b"], b"hello")
    assert m.get_labels() == ["a", "b"]
    assert m.get_bytes() == b"hello"
    assert isinstance(m.get_uuid(), str)


def test_bus_api():
    bus = jackfield.MessageBus()
    bus.register_consumer(lambda m: None)
    bus.register_consumer(lambda m: None, require_from="p")
    bus.register_consumer(lambda m: None, require=[jackfield.ProducerDim.only("p")])
    bus.register_consumer(lambda m: None, prefer=[(jackfield.ProducerDim.only("p"), 1.0)])
    bus.send("p", jackfield.Message([], b""))
    bus.drain()
    assert bus.is_empty()


def test_dim_api():
    jackfield.ProducerDim.only("p")
    jackfield.ProducerDim.any_of(["a", "b"])
    jackfield.ProducerDim.none_of(["x"])
    jackfield.LabelDim.any_of(["l"])
    jackfield.LabelDim.all_of(["l1", "l2"])
    jackfield.LabelDim.none_of(["l"])
    jackfield.SizeDim.at_most(100)
    jackfield.SizeDim.at_least(10)
    jackfield.SizeDim.between(10, 100)
