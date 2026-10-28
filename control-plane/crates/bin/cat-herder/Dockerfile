# Multi-stage build for the Cat Herder control plane.
# Build context is the repository root (see Tiltfile), so the gRPC contract at `proto/`
# is available to the build script — `crates/adapters/api-http/build.rs` resolves the
# repo root four levels up from the crate, which requires the `control-plane/` layout
# to be preserved in the image.

# --- build ---
FROM rust:1.95-slim AS build
WORKDIR /app
COPY control-plane/Cargo.toml control-plane/Cargo.lock ./control-plane/
COPY control-plane/crates ./control-plane/crates
COPY proto ./proto
RUN cargo build --release --manifest-path control-plane/Cargo.toml -p cat-herder

# --- runtime ---
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/control-plane/target/release/cat-herder /usr/local/bin/cat-herder
EXPOSE 8080
ENTRYPOINT ["cat-herder"]
