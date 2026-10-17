CARGO ?= cargo
GODOT_QA ?= $(CARGO) run --bin godot-qa --

.PHONY: help prepare-sample test build build-release doctor scenario-list

help:
	@echo "Targets: prepare-sample test build build-release doctor scenario-list"

prepare-sample:
	$(CARGO) xtask prepare-sample

test:
	$(CARGO) xtask check

build:
	$(CARGO) build --bin godot-qa

build-release:
	$(CARGO) build --release --bin godot-qa

doctor:
	$(GODOT_QA) doctor --project examples/sample_project --json

scenario-list:
	$(GODOT_QA) scenario list --project examples/sample_project --json
