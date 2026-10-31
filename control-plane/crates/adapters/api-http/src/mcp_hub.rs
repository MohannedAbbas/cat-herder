//! MCP server-to-client push hub (Streamable HTTP).
//!
//! The MCP server is otherwise stateless, but Streamable HTTP allows the server to
//! push messages to the client over a long-lived SSE stream (the `GET /mcp` request).
//! This hub ties the two together:
//!
//! * `run_id -> session_id` — the association created when `run_agent` creates a run
//!   for a session.
//! * `session_id -> broadcast::Sender<String>` — the outbound channel per session;
//!   the SSE handler subscribes to it and forwards each message as an SSE event.
//!
//! When a run reaches a terminal state the control plane calls [`McpHub::publish`],
//! which looks up the run's session and fans the notification out to any connected
//! client (a no-op when the run has no associated session, preserving the stateless
//! POST-only flow).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

/// How many messages a subscriber may fall behind before it observes a lag.
const BROADCAST_CAPACITY: usize = 64;

#[derive(Default)]
struct Inner {
    /// run id -> session id.
    run_to_session: HashMap<u64, String>,
    /// session id -> outbound channel.
    sessions: HashMap<String, broadcast::Sender<String>>,
}

/// Maps runs to MCP sessions and fans out notifications to connected SSE clients.
/// Cloneable handle over shared state (matches [`crate::AppState`]).
#[derive(Clone, Default)]
pub struct McpHub {
    inner: Arc<Mutex<Inner>>,
}

impl McpHub {
    /// A fresh, empty hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Associate a run with the MCP session that created it, so a later terminal
    /// notification can be routed to that session's stream. Creates the session's
    /// outbound channel on first use.
    pub fn associate(&self, run_id: u64, session_id: String) {
        let mut inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        inner
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| broadcast::channel(BROADCAST_CAPACITY).0);
        inner.run_to_session.insert(run_id, session_id);
    }

    /// Register a session (creating its outbound channel) without associating a run.
    /// Called on `initialize` so a client can open the SSE stream immediately, before
    /// any run has been created for it.
    pub fn register_session(&self, session_id: String) {
        let mut inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        inner
            .sessions
            .entry(session_id)
            .or_insert_with(|| broadcast::channel(BROADCAST_CAPACITY).0);
    }

    /// Subscribe to a session's outbound stream, or `None` if the session is unknown.
    /// Returns `None` so the SSE handler can return a `400` for a bogus/expired
    /// session id (matching the stateless POST flow, which never requires one).
    pub fn subscribe(&self, session_id: &str) -> Option<broadcast::Receiver<String>> {
        let inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        inner.sessions.get(session_id).map(|tx| tx.subscribe())
    }

    /// Publish a pre-serialized JSON-RPC message for `run_id` to its session's stream.
    /// A no-op when the run has not been associated with any session (or when no
    /// client is currently subscribed — the message is simply dropped).
    pub fn publish(&self, run_id: u64, json: String) {
        let inner = self.inner.lock().expect("mcp-hub mutex poisoned");
        if let Some(session_id) = inner.run_to_session.get(&run_id)
            && let Some(tx) = inner.sessions.get(session_id)
        {
            // A send error just means no live subscribers.
            let _ = tx.send(json);
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
        let mut rx = hub.subscribe("sess-1").expect("session exists");
        hub.publish(7, "hello".to_string());
        assert_eq!(rx.recv().await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn subscribe_unknown_session_is_none() {
        let hub = McpHub::new();
        assert!(hub.subscribe("nobody").is_none());
    }

    #[tokio::test]
    async fn publish_unassociated_run_is_noop() {
        let hub = McpHub::new();
        hub.associate(1, "sess-a".to_string());
        let mut rx = hub.subscribe("sess-a").unwrap();
        // Publishing for a run with no session must not deliver anything.
        hub.publish(999, "orphan".to_string());
        hub.publish(1, "real".to_string());
        assert_eq!(rx.recv().await.unwrap(), "real");
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let hub = McpHub::new();
        hub.associate(2, "sess-2".to_string());
        let mut a = hub.subscribe("sess-2").unwrap();
        let mut b = hub.subscribe("sess-2").unwrap();
        hub.publish(2, "broadcast".to_string());
        assert_eq!(a.recv().await.unwrap(), "broadcast");
        assert_eq!(b.recv().await.unwrap(), "broadcast");
    }
}
