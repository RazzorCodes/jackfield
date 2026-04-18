VENV := .venv/bin

.PHONY: test test-rs test-py dev

test: test-rs test-py

test-rs:
	cargo test

test-py: dev
	$(VENV)/pytest tests/test_integration.py -v

dev:
	maturin develop --features python
