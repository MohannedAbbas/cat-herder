# Multi-stage build for the Cat Herder control plane.
# Build context is the repo root.

# --- build ---
FROM rust:1.95-slim AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p cat-herder

# --- runtime ---
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/cat-herder /usr/local/bin/cat-herder
EXPOSE 8080
ENTRYPOINT ["cat-herder"]
