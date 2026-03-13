import jackfield
import pytest


def test_bus_import():
    bus = jackfield.MessageBus()
    assert bus is not None


def test_message_creation():
    labels = ["test_label", "empty", "empty", "empty"]
    data = b"x" * 32

    msg = jackfield.Message(labels, data)

    assert msg.get_labels()[0] == "test_label"
    assert len(msg.get_bytes()) == 32
