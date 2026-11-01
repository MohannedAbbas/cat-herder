//! MCP server-to-client push: the local SSE last-mile plus a cross-replica notifier.
//!
//! The MCP server is otherwise stateless, but Streamable HTTP lets the server push
//! messages to the client over a long-lived SSE stream (`GET /mcp`). Delivery has two
//! tiers:
//!
//! * [`McpHub`] — the process-local last-mile: `session_id -> broadcast::Sender` feeding
//!   the open SSE connection. This is inherently per-replica (an SSE connection is
//!   pinned to one process).
//! * [`RunStatusNotifierImpl`] — the [`RunStatusNotifier`] port implementation. In
//!   single-replica/dev it publishes into the local `McpHub`; in multi-replica it
//!   writes the run→session association to Redis and publishes to a Redis pub/sub
//!   channel, which every replica's subscriber forwards into its local `McpHub`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use tokio::sync::broadcast;

use cat_herder_domain::entities::Run;
use cat_herder_domain::ports::{DomainResult, RunStatusNotifier};
use cat_herder_domain::redact::redact;

use redis::aio::ConnectionManager;

/// How many messages a subscriber may fall behind before it observes a lag.
const BROADCAST_CAPACITY: usize = 64;

/// Redis key prefix for the run→session association (`mcp:run:<run_id>`).
const MCP_RUN_KEY_PREFIX: &str = "mcp:run:";
/// Redis pub/sub channel per session (`mcp:notify:<session_id>`).
const MCP_NOTIFY_PREFIX: &str = "mcp:notify:";
/// Glob pattern every replica subscribes to.
const MCP_NOTIFY_PATTERN: &str = "mcp:notify:*";
/// TTL (seconds) for the run→session association — a run's live status only matters
/// for this long after its terminal state.
const MCP_ASSOC_TTL_SECS: u64 = 86_400;

/// Build the JSON-RPC `notifications/run/status` message pushed to an MCP client.
/// The report is redacted before it leaves the control plane.
pub fn mcp_run_status_notification(run: &Run) -> String {
    let state_str = serde_json::to_value(run.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{:?}", run.state).to_lowercase());
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/run/status",
        "params": {
            "run_id": run.id.0,
            "state": state_str,
            "report": run.report.as_deref().map(redact),
            "pr_url": run.pr_url,
        },
    })
    .to_string()
}

#[derive(Default)]
struct HubInner {
    /// session id -> outbound channel (the local SSE last-mile).
    sessions: HashMap<String, broadcast::Sender<String>>,
    /// run id -> session id (single-replica association only).
    run_to_session: HashMap<u64, String>,
}

/// Process-local MCP SSE last-mile: maps sessions to their open SSE stream and (in
/// single-replica mode) maps runs to sessions. Cloneable over shared state.
#[derive(Clone, Default)]
pub struct McpHub {
    inner: Arc<Mutex<HubInner>>,
}

impl McpHub {
    /// A fresh, empty hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure a session has an outbound channel (called on `initialize` so a client
    /// can open its SSE stream before any run exists for it).
    pub fn register_session(&self, session_id: String) {
        let mut inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        inner
            .sessions
            .entry(session_id)
            .or_insert_with(|| broadcast::channel(BROADCAST_CAPACITY).0);
    }

    /// Subscribe to a session's outbound stream, creating the channel if needed. The
    /// channel is created lazily so an SSE request can land on any replica regardless
    /// of which one served `initialize`/`run_agent`.
    pub fn subscribe(&self, session_id: &str) -> broadcast::Receiver<String> {
        let mut inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        inner
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(BROADCAST_CAPACITY).0)
            .subscribe()
    }

    /// Associate a run with a session (single-replica path). Creates the session's
    /// outbound channel on first use.
    pub fn associate(&self, run_id: u64, session_id: String) {
        let mut inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        inner
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| broadcast::channel(BROADCAST_CAPACITY).0);
        inner.run_to_session.insert(run_id, session_id);
    }

    /// Publish a message to a run's session (single-replica path). A no-op when the
    /// run has no associated session, or no client is subscribed (the send simply
    /// returns an error, which is ignored).
    pub fn publish(&self, run_id: u64, json: String) {
        let inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        if let Some(session_id) = inner.run_to_session.get(&run_id)
            && let Some(tx) = inner.sessions.get(session_id)
        {
            let _ = tx.send(json);
        }
    }

    /// Deliver a message to a session's local stream (the Redis subscriber's target).
    /// Creates the channel lazily; a no-op send when no client is connected.
    pub fn dispatch(&self, session_id: &str, json: String) {
        let mut inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        let tx = inner
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(BROADCAST_CAPACITY).0);
        let _ = tx.send(json);
    }
}

/// Cross-replica MCP push backend: Redis pub/sub. The run→session association and the
/// fan-out both live in Redis, so any replica that ingests a transition can route the
/// notification to the replica holding the client's SSE connection.
pub struct RedisMcpNotifier {
    client: redis::Client,
    conn: ConnectionManager,
}

impl RedisMcpNotifier {
    /// Connect to Redis and validate the connection.
    pub async fn connect(url: &str) -> DomainResult<Self> {
        let client = redis::Client::open(url)?;
        let conn = ConnectionManager::new(client.clone()).await?;
        Ok(Self { client, conn })
    }

    /// Associate a run with the MCP session that created it (shared across replicas,
    /// TTL'd).
    pub async fn associate(&self, run_id: u64, session_id: &str) -> DomainResult<()> {
        let key = format!("{MCP_RUN_KEY_PREFIX}{run_id}");
        let mut conn = self.conn.clone();
        let _: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(session_id)
            .arg("EX")
            .arg(MCP_ASSOC_TTL_SECS)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Spawn this replica's subscriber: listen on `mcp:notify:*` and forward each
    /// message into the local [`McpHub`] that feeds the open SSE connections.
    pub async fn spawn_subscriber(&self, hub: McpHub) -> DomainResult<()> {
        let pubsub = self.client.get_async_pubsub().await?;
        let mut pubsub = pubsub;
        pubsub.psubscribe(MCP_NOTIFY_PATTERN).await?;
        tokio::spawn(async move {
            let mut stream = pubsub.into_on_message();
            while let Some(msg) = stream.next().await {
                let channel = msg.get_channel_name().to_string();
                let Some(session_id) = channel.strip_prefix(MCP_NOTIFY_PREFIX) else {
                    continue;
                };
                let Ok(json) = msg.get_payload::<String>() else {
                    continue;
                };
                hub.dispatch(session_id, json);
            }
        });
        Ok(())
    }
}

impl RunStatusNotifier for RedisMcpNotifier {
    async fn notify(&self, run: &Run) -> DomainResult<()> {
        let key = format!("{MCP_RUN_KEY_PREFIX}{}", run.id.0);
        let mut conn = self.conn.clone();
        let session: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
        let Some(session_id) = session else {
            return Ok(()); // the run has no associated MCP session
        };
        let channel = format!("{MCP_NOTIFY_PREFIX}{session_id}");
        let json = mcp_run_status_notification(run);
        let _: i64 = redis::cmd("PUBLISH")
            .arg(&channel)
            .arg(&json)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

/// The [`RunStatusNotifier`] port implementation: local (single-replica/dev) or
/// Redis-backed (multi-replica).
#[derive(Clone)]
pub enum RunStatusNotifierImpl {
    Local(McpHub),
    Redis(Arc<RedisMcpNotifier>),
}

impl RunStatusNotifierImpl {
    /// Associate a run with the MCP session that created it, routing to the correct
    /// backend. Best-effort: a failure is logged, never fatal to the run.
    pub async fn associate(&self, run_id: u64, session_id: &str) {
        match self {
            RunStatusNotifierImpl::Local(hub) => hub.associate(run_id, session_id.to_string()),
            RunStatusNotifierImpl::Redis(n) => {
                if let Err(e) = n.associate(run_id, session_id).await {
                    eprintln!("mcp run->session association failed for run {run_id}: {e}");
                }
            }
        }
    }

    /// Connect the Redis backend and spawn its local subscriber, wiring the given
    /// [`McpHub`] as the delivery target.
    pub async fn connect_redis(url: &str, hub: McpHub) -> DomainResult<Self> {
        let notifier = RedisMcpNotifier::connect(url).await?;
        notifier.spawn_subscriber(hub).await?;
        Ok(Self::Redis(Arc::new(notifier)))
    }
}

impl RunStatusNotifier for RunStatusNotifierImpl {
    async fn notify(&self, run: &Run) -> DomainResult<()> {
        match self {
            RunStatusNotifierImpl::Local(hub) => {
                hub.publish(run.id.0, mcp_run_status_notification(run));
                Ok(())
            }
            RunStatusNotifierImpl::Redis(n) => n.notify(run).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn associate_then_publish_reaches_subscriber() {
        let hub = McpHub::new();
        hub.associate(7, "sess-1".to_string());
        let mut rx = hub.subscribe("sess-1");
        hub.publish(7, "hello".to_string());
        assert_eq!(rx.recv().await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn subscribe_lazily_creates_unknown_session() {
        // An SSE request may land on a replica that never saw `initialize`; the
        // channel must be created on demand rather than rejected.
        let hub = McpHub::new();
        let mut rx = hub.subscribe("elsewhere");
        hub.dispatch("elsewhere", "hello".to_string());
        assert_eq!(rx.recv().await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn publish_unassociated_run_is_noop() {
        let hub = McpHub::new();
        hub.associate(1, "sess-a".to_string());
        let mut rx = hub.subscribe("sess-a");
        // Publishing for a run with no session must not deliver anything.
        hub.publish(999, "orphan".to_string());
        hub.publish(1, "real".to_string());
        assert_eq!(rx.recv().await.unwrap(), "real");
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let hub = McpHub::new();
        hub.associate(2, "sess-2".to_string());
        let mut a = hub.subscribe("sess-2");
        let mut b = hub.subscribe("sess-2");
        hub.publish(2, "broadcast".to_string());
        assert_eq!(a.recv().await.unwrap(), "broadcast");
        assert_eq!(b.recv().await.unwrap(), "broadcast");
    }
}
