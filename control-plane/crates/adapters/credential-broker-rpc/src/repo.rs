//! Concrete broker composition for the gRPC server (`PLAN.md` §11.3).
//!
//! tonic's service trait requires `Send` futures, but the domain's native-`async fn`
//! ports don't guarantee `Send` *generically* — only once they're instantiated with
//! concrete repository types. So the server holds a fully-instantiated
//! [`CredentialBrokerService`] over these concrete enums rather than a generic
//! `C: CredentialBroker`. The control-plane client side ([`crate::RemoteCredentialBroker`])
//! has no such constraint and stays a plain [`cat_herder_domain::ports::CredentialBroker`]
//! impl.

use std::sync::Arc;

use cat_herder_credential_broker::CredentialBrokerService;
use cat_herder_domain::entities::{CredentialGrant, RunId};
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{
    CredentialGrantRepository, DomainResult, EventBus, EventRepository,
};
use cat_herder_store_inmemory::{InMemoryCredentialGrantRepository, InMemoryEventRepository};
use cat_herder_store_nats::NatsEventBus;
use cat_herder_store_postgres::{PostgresCredentialGrantRepository, PostgresEventRepository};

/// The concrete broker the gRPC server wraps.
pub type BrokerService = CredentialBrokerService<GrantRepo, EventRepo, Bus>;

/// Grant store selector: Postgres in production, in-memory otherwise.
#[derive(Clone)]
pub enum GrantRepo {
    InMemory(InMemoryCredentialGrantRepository),
    Postgres(PostgresCredentialGrantRepository),
}

impl CredentialGrantRepository for GrantRepo {
    async fn create(&self, grant: &CredentialGrant) -> DomainResult<CredentialGrant> {
        match self {
            GrantRepo::InMemory(r) => r.create(grant).await,
            GrantRepo::Postgres(p) => p.create(grant).await,
        }
    }
    async fn get(&self, id: &str) -> DomainResult<Option<CredentialGrant>> {
        match self {
            GrantRepo::InMemory(r) => r.get(id).await,
            GrantRepo::Postgres(p) => p.get(id).await,
        }
    }
    async fn list_active(&self, run_id: RunId) -> DomainResult<Vec<CredentialGrant>> {
        match self {
            GrantRepo::InMemory(r) => r.list_active(run_id).await,
            GrantRepo::Postgres(p) => p.list_active(run_id).await,
        }
    }
    async fn revoke(&self, id: &str) -> DomainResult<CredentialGrant> {
        match self {
            GrantRepo::InMemory(r) => r.revoke(id).await,
            GrantRepo::Postgres(p) => p.revoke(id).await,
        }
    }
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()> {
        match self {
            GrantRepo::InMemory(r) => r.delete_run(run_id).await,
            GrantRepo::Postgres(p) => p.delete_run(run_id).await,
        }
    }
}

/// Event-log selector (the broker appends the audited `credential.*` events).
#[derive(Clone)]
pub enum EventRepo {
    InMemory(InMemoryEventRepository),
    Postgres(PostgresEventRepository),
}

impl EventRepository for EventRepo {
    async fn append(&self, run_id: RunId, event: &Event) -> DomainResult<Event> {
        match self {
            EventRepo::InMemory(r) => r.append(run_id, event).await,
            EventRepo::Postgres(p) => p.append(run_id, event).await,
        }
    }
    async fn list(&self, run_id: RunId, after_seq: i64, limit: i64) -> DomainResult<Vec<Event>> {
        match self {
            EventRepo::InMemory(r) => r.list(run_id, after_seq, limit).await,
            EventRepo::Postgres(p) => p.list(run_id, after_seq, limit).await,
        }
    }
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()> {
        match self {
            EventRepo::InMemory(r) => r.delete_run(run_id).await,
            EventRepo::Postgres(p) => p.delete_run(run_id).await,
        }
    }
}

/// Fan-out bus selector: NATS JetStream when configured, no-op otherwise.
#[derive(Clone)]
pub enum Bus {
    Noop,
    Nats(Arc<NatsEventBus>),
}

impl EventBus for Bus {
    async fn publish(&self, event: &Event) -> DomainResult<()> {
        match self {
            Bus::Noop => Ok(()),
            Bus::Nats(n) => n.publish(event).await,
        }
    }
}
