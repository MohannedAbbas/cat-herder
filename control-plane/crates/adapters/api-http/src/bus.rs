//! Event bus selector — NATS JetStream in production, no-op otherwise.
//!
//! The bus is the fan-out *transport* (`PLAN.md` §9); the durable authority is the
//! event repository. A no-op is used when NATS is not configured (tests, local dev).

use std::sync::Arc;

use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{DomainResult, EventBus};
use cat_herder_store_nats::NatsEventBus;

/// The fan-out bus the application publishes appended events to.
#[derive(Clone)]
pub enum EventBusImpl {
    /// Fan-out disabled (no `CAT_HERDER_NATS_URL`, tests, local dev).
    Noop,
    Nats(Arc<NatsEventBus>),
}

impl EventBus for EventBusImpl {
    async fn publish(&self, event: &Event) -> DomainResult<()> {
        match self {
            EventBusImpl::Noop => Ok(()),
            EventBusImpl::Nats(n) => n.publish(event).await,
        }
    }
}
