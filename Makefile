.PHONY: build test fmt lint run chart harness-image sandbox-image

build:
	cargo build

test:
	cargo test

fmt:
	cargo fmt

lint:
	cargo clippy --workspace -- -D warnings

run:
	cargo run -p cat-herder

chart:
	helm lint helm/cat-herder

harness-image:
	podman build -t cat-herder-agent-harness:dev -f agent-harness/Containerfile agent-harness

sandbox-image:
	podman build -t cat-herder-sandbox:dev -f sandbox/Containerfile sandbox
