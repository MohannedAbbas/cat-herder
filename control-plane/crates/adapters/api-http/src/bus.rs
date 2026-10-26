//! Event bus selector — NATS JetStream in production, no-op otherwise.
//!
//! The bus is the fan-out *transport* (`PLAN.md` §9); the durable authority is the
//! event repository. A no-op is used when NATS is not configured (tests, local dev).

use std::sync::Arc;

use cat_herder_domain::api::EventResponse;
use cat_herder_domain::entities::RunId;
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{DomainResult, EventBus};
use cat_herder_store_nats::NatsEventBus;
use futures::stream::BoxStream;

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

impl EventBusImpl {
    /// Subscribe to a run's live event tail (SSE). Returns `None` when NATS is not
    /// configured (clients fall back to polling the paginated endpoint, `PLAN.md` §9).
    pub async fn subscribe(
        &self,
        run_id: RunId,
    ) -> Option<BoxStream<'static, DomainResult<EventResponse>>> {
        match self {
            EventBusImpl::Noop => None,
            EventBusImpl::Nats(n) => match n.subscribe(run_id.0).await {
                Ok(stream) => Some(stream),
                Err(e) => {
                    eprintln!("event subscription failed for run {}: {e}", run_id.0);
                    None
                }
            },
        }
    }
}
