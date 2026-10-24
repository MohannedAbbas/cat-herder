.PHONY: build test coverage fmt lint run chart tilt harness-image sandbox-image

build:
	cargo build

test:
	cargo test

# Measure test coverage and fail if line coverage is below 85%.
# Requires: cargo install cargo-llvm-cov (and `llvm-tools-preview` rustup component).
coverage:
	cargo llvm-cov --fail-under-lines 85 --html

fmt:
	cargo fmt

lint:
	cargo clippy --workspace -- -D warnings

run:
	cargo run -p cat-herder

chart:
	helm lint helm/cat-herder

tilt:
	tilt up

harness-image:
	podman build -t cat-herder-agent-harness:dev -f agent-harness/Containerfile agent-harness

sandbox-image:
	podman build -t cat-herder-sandbox:dev -f sandbox/Containerfile sandbox
