//! Redis [`BudgetCounter`] — the atomic, authoritative budget counter (`PLAN.md` §12).
//!
//! All model traffic is metered against a central atomic counter keyed by `run_id`.
//! `consume` runs a Lua script so the increment-and-check is atomic (a single Redis
//! command execution): parallel subagent fan-out cannot overshoot the cap, because the
//! counter is the one serialization point. This is the hard stop — the metering proxy
//! calls it before each model call and rejects the call on `Exceeded`.

use cat_herder_domain::entities::RunId;
use cat_herder_domain::ports::{BudgetCounter, ConsumeOutcome, DedupRepository, DomainResult};
use redis::Script;
use redis::aio::ConnectionManager;

/// The Redis key holding a run's consumed-token count.
pub fn consumed_key(run_id: RunId) -> String {
    format!("budget:{}:consumed", run_id.0)
}

/// Atomic `consume`: INCRBY then check against `cap`; roll back the increment and
/// report `Exceeded` when the cap is crossed. Returns `{status, consumed}` where
/// `status` is `1` (ok) or `0` (exceeded) and `consumed` is the running total (the
/// pre-increment total when exceeded).
const CONSUME_LUA: &str = r#"
local consumed = tonumber(redis.call('INCRBY', KEYS[1], ARGV[1]) or 0)
local cap = tonumber(ARGV[2])
if cap > 0 and consumed > cap then
  redis.call('DECRBY', KEYS[1], ARGV[1])
  return {0, consumed - tonumber(ARGV[1])}
end
return {1, consumed}
"#;

/// Redis-backed budget counter. Constructed only when `CAT_HERDER_REDIS_URL` is
/// configured; otherwise the composition root uses the in-memory fallback.
#[derive(Clone)]
pub struct RedisBudgetCounter {
    conn: ConnectionManager,
}

impl RedisBudgetCounter {
    /// Connect to Redis and validate the connection.
    pub async fn connect(url: &str) -> DomainResult<Self> {
        let client = redis::Client::open(url)?;
        let conn = ConnectionManager::new(client).await?;
        Ok(Self { conn })
    }
}

impl BudgetCounter for RedisBudgetCounter {
    async fn consume(&self, run_id: RunId, cap: u64, tokens: u64) -> DomainResult<ConsumeOutcome> {
        let key = consumed_key(run_id);
        let mut conn = self.conn.clone();
        let result: Vec<i64> = Script::new(CONSUME_LUA)
            .key(&key)
            .arg(tokens)
            .arg(cap)
            .invoke_async(&mut conn)
            .await?;
        match result.as_slice() {
            [1, consumed] => Ok(ConsumeOutcome::Ok {
                consumed: *consumed as u64,
            }),
            [0, consumed] => Ok(ConsumeOutcome::Exceeded {
                cap,
                consumed: *consumed as u64,
            }),
            other => Err(format!("unexpected consume script result: {other:?}").into()),
        }
    }

    async fn remaining(&self, run_id: RunId, cap: u64) -> DomainResult<u64> {
        let key = consumed_key(run_id);
        let mut conn = self.conn.clone();
        let consumed: Option<u64> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
        Ok(cap.saturating_sub(consumed.unwrap_or(0)))
    }

    async fn reset(&self, run_id: RunId) -> DomainResult<()> {
        let key = consumed_key(run_id);
        let mut conn = self.conn.clone();
        let _: i64 = redis::cmd("DEL").arg(&key).query_async(&mut conn).await?;
        Ok(())
    }
}

/// Redis-backed dedup/replay cache (`PLAN.md` §11.5): `SET key NX PX ttl` is atomic, so
/// `check_and_record` is a single-command check-and-set (no TOCTOU race).
#[derive(Clone)]
pub struct RedisDedupRepository {
    conn: ConnectionManager,
}

impl RedisDedupRepository {
    /// Connect to Redis and validate the connection.
    pub async fn connect(url: &str) -> DomainResult<Self> {
        let client = redis::Client::open(url)?;
        let conn = ConnectionManager::new(client).await?;
        Ok(Self { conn })
    }
}

impl DedupRepository for RedisDedupRepository {
    async fn check_and_record(&self, key: &str, ttl: std::time::Duration) -> DomainResult<bool> {
        let mut conn = self.conn.clone();
        let millis = ttl.as_millis().max(1) as u64;
        // NX: set only if absent. `Option<String>` is `None` when the key already
        // existed (nil reply) -> duplicate.
        let set: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg("1")
            .arg("NX")
            .arg("PX")
            .arg(millis)
            .query_async(&mut conn)
            .await?;
        Ok(set.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumed_key_formats() {
        assert_eq!(consumed_key(RunId(7)), "budget:7:consumed");
        assert_eq!(consumed_key(RunId(0)), "budget:0:consumed");
    }

    #[test]
    fn consume_lua_is_defined() {
        assert!(CONSUME_LUA.contains("INCRBY"));
        assert!(CONSUME_LUA.contains("DECRBY"));
    }
}
