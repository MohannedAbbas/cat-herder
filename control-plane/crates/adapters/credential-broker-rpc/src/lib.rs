//! gRPC transport for the JIT credential broker (`PLAN.md` §11.3).
//!
//! The broker is its own hardened service behind a gRPC interface with mTLS caller
//! auth (tonic + rustls; SPIFFE-style URI SANs in the client cert). This crate
//! provides both sides:
//!
//! - [`RemoteCredentialBroker`] — a [`cat_herder_domain::ports::CredentialBroker`]
//!   client adapter the control plane (and, later, the harness sidecar) uses to reach
//!   the broker.
//! - [`CredentialBrokerGrpc`] — the tonic service that wraps any `CredentialBroker`
//!   implementation and enforces the caller-auth allowlist.
//! - [`tls`] — mTLS configuration helpers (server/client) and SPIFFE SAN verification.
//!
//! The materialized secret is returned to the authenticated caller for out-of-band
//! injection, never into model context — the same trust boundary as the in-process
//! [`cat_herder_credential_broker::CredentialBrokerService`].

pub mod client;
mod convert;
pub mod repo;
pub mod server;
pub mod tls;

pub use client::RemoteCredentialBroker;
pub use repo::{BrokerService, Bus, EventRepo, GrantRepo};
pub use server::CredentialBrokerGrpc;

/// Generated protobuf types (from `proto/credential.proto`).
pub mod proto {
    pub mod v1 {
        tonic::include_proto!("cat_herder.v1");
    }
}
