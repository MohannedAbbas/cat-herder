//! NATS JetStream [`LeaseManager`] — shard leases for the sharded reconciler
//! (`PLAN.md` §6).
//!
//! Each shard is one key in a JetStream KV bucket (`LEASES`): the value is the owning
//! leader's id, and the bucket's `max_age` is the lease TTL. An entry that is not
//! renewed within the TTL is discarded by the server, so a surviving leader can
//! `create` the key again and take the shard over — that is the crash-recovery path.

use std::time::Duration;

use async_nats::jetstream;
use async_nats::jetstream::kv::{CreateErrorKind, UpdateErrorKind};
use cat_herder_domain::ports::{DomainResult, LeaseManager};

/// KV bucket holding one key per shard.
pub const LEASES_BUCKET: &str = "LEASES";

/// The KV key for a shard.
pub fn shard_key(shard: u32) -> String {
    format!("shard-{shard}")
}

/// Shard leases backed by a NATS JetStream KV bucket.
pub struct NatsLeaseManager {
    kv: jetstream::kv::Store,
}

impl NatsLeaseManager {
    /// Connect and idempotently ensure the `LEASES` bucket exists (reusing it across
    /// restarts). `ttl` is the lease lifetime: a lease that isn't renewed within it
    /// lapses and can be taken over.
    pub async fn connect(url: &str, ttl: Duration) -> DomainResult<Self> {
        let client = async_nats::connect(url).await?;
        let context = jetstream::new(client);
        let config = jetstream::kv::Config {
            bucket: LEASES_BUCKET.to_string(),
            history: 1,
            max_age: ttl,
            ..Default::default()
        };
        let kv = match context.get_key_value(LEASES_BUCKET).await {
            Ok(store) => store,
            Err(_) => context.create_key_value(config).await?,
        };
        Ok(Self { kv })
    }
}

impl LeaseManager for NatsLeaseManager {
    async fn acquire(&self, shard: u32, owner: &str) -> DomainResult<bool> {
        let key = shard_key(shard);
        // `create` is a CAS: it succeeds only if the key doesn't exist (fresh, or the
        // previous lease lapsed and was discarded by `max_age`).
        match self.kv.create(&key, owner.as_bytes().to_vec().into()).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == CreateErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    async fn renew(&self, shard: u32, owner: &str) -> DomainResult<bool> {
        let key = shard_key(shard);
        let Some(entry) = self.kv.entry(&key).await? else {
            // No entry: our lease lapsed. Try to take it back; only one contender
            // wins the `create` CAS.
            return self.acquire(shard, owner).await;
        };
        if entry.value.as_ref() != owner.as_bytes() {
            return Ok(false); // another owner holds it now
        }
        match self
            .kv
            .update(&key, entry.value.clone(), entry.revision)
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == UpdateErrorKind::WrongLastRevision => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    async fn release(&self, shard: u32, owner: &str) -> DomainResult<()> {
        let key = shard_key(shard);
        // Only release if we still hold it; a revision mismatch (already taken over)
        // is a no-op.
        if let Some(entry) = self.kv.entry(&key).await?
            && entry.value.as_ref() == owner.as_bytes()
        {
            let _ = self
                .kv
                .delete_expect_revision(&key, Some(entry.revision))
                .await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_key_formats() {
        assert_eq!(shard_key(0), "shard-0");
        assert_eq!(shard_key(7), "shard-7");
        assert_eq!(shard_key(16), "shard-16");
    }
}
