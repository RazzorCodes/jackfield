ROOT           := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
VENV           := $(ROOT).venv
CARGO_MANIFEST := $(ROOT)Cargo.toml
PY_INTEGRATION := $(ROOT)src/integrations/python

MATURIN := $(VENV)/bin/maturin
PYTEST  := $(VENV)/bin/pytest
TWINE   := $(VENV)/bin/twine

.PHONY: test test-rs test-py dev release-dev venv

venv: $(VENV)/bin/activate

$(VENV)/bin/activate:
	python3 -m venv $(VENV)
	$(VENV)/bin/pip install -q maturin pytest twine

test: test-rs test-py

test-rs:
	cargo test --features network

test-py: dev
	$(PYTEST) tests/test_integration.py -v

dev: venv
	cd $(PY_INTEGRATION) && $(MATURIN) develop

release-dev: venv
	$(VENV)/bin/pip install -q twine
	cd $(PY_INTEGRATION) && $(MATURIN) build --release --out $(ROOT)target/wheels
	TWINE_USERNAME=homelab TWINE_PASSWORD=homelab \
		$(TWINE) upload --verbose --repository-url http://pypi.lan/ \
		$(ROOT)target/wheels/*.whl
