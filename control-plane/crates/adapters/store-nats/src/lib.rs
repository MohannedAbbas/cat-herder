//! NATS JetStream [`EventBus`] — fan-out transport for events (`PLAN.md` §9).
//!
//! Postgres is the durable authority; this only publishes a copy of each appended
//! event so consumers can tail live. Publish is best-effort (a failed publish never
//! fails the append). The stream is a short-lived buffer — replay always reads
//! Postgres.

use async_nats::jetstream;
use cat_herder_domain::api::EventResponse;
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{DomainResult, EventBus};

/// JetStream stream name and subject pattern (per-run subjects `run.<id>.events`).
pub const STREAM_NAME: &str = "EVENTS";
const SUBJECT_PREFIX: &str = "run.";
const SUBJECT_SUFFIX: &str = ".events";

/// The per-run subject an event is published to.
pub fn subject_for(run_id: u64) -> String {
    format!("{SUBJECT_PREFIX}{run_id}{SUBJECT_SUFFIX}")
}

/// Publish events to NATS JetStream. Constructed only when `CAT_HERDER_NATS_URL` is
/// configured; otherwise the composition root uses a no-op bus.
pub struct NatsEventBus {
    context: jetstream::Context,
}

impl NatsEventBus {
    /// Connect and idempotently ensure the `EVENTS` stream exists.
    pub async fn connect(url: &str) -> DomainResult<Self> {
        let client = async_nats::connect(url).await?;
        let context = jetstream::new(client);

        context
            .get_or_create_stream(jetstream::stream::Config {
                name: STREAM_NAME.to_string(),
                subjects: vec![format!("{SUBJECT_PREFIX}*{SUBJECT_SUFFIX}")],
                storage: jetstream::stream::StorageType::File,
                ..Default::default()
            })
            .await?;

        Ok(Self { context })
    }
}

impl EventBus for NatsEventBus {
    async fn publish(&self, event: &Event) -> DomainResult<()> {
        let subject = subject_for(event.run_id.0);
        let payload = serde_json::to_vec(&EventResponse::from(event))?;
        self.context.publish(subject, payload.into()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_for_formats_per_run_subject() {
        assert_eq!(subject_for(7), "run.7.events");
        assert_eq!(subject_for(0), "run.0.events");
    }
}
