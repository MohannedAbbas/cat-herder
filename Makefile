.PHONY: build test coverage fmt lint run chart tilt prune harness-image sandbox-image

# The Rust workspace lives in control-plane/ (see control-plane/Cargo.toml).
WORKSPACE := control-plane

build:
	cd $(WORKSPACE) && cargo build

test:
	cd $(WORKSPACE) && cargo test

# Measure test coverage and fail if line coverage is below 85%.
# Excludes infra that needs a live cluster/DB (bin, kube runtime, Postgres/NATS adapters)
# from the denominator; those are covered by DB-backed integration tests (see CI).
# Requires: cargo install cargo-llvm-cov (and `llvm-tools-preview` rustup component).
coverage:
	cd $(WORKSPACE) && cargo llvm-cov --fail-under-lines 85 --ignore-filename-regex 'bin/cat-herder|runtime-agent-sandbox|store-postgres|store-nats|store-redis' --html

fmt:
	cd $(WORKSPACE) && cargo fmt

lint:
	cd $(WORKSPACE) && cargo clippy --workspace -- -D warnings

run:
	cd $(WORKSPACE) && cargo run -p cat-herder

chart:
	helm lint helm/cat-herder

tilt:
	tilt up

# Reclaim disk from local builds: clears the Rust target/ (regenerable) and the
# dangling container layers that every Tilt rebuild leaves behind. Tilt's Rust-image
# builds are disk-hungry (each leaves a ~1 GB layer), so run this when the disk fills.
prune:
	cd $(WORKSPACE) && cargo clean
	podman image prune -f

harness-image:
	podman build -t cat-herder-agent-harness:dev -f agent-harness/Containerfile agent-harness

sandbox-image:
	podman build -t cat-herder-sandbox:dev -f sandbox/Containerfile sandbox
