//! Per-run live log spool + broadcast (`NEXT_STEPS #18`, `PLAN.md` §16).
//!
//! The harness streams redacted log lines (LLM output, tool calls, shell
//! stdout/stderr) over the gRPC control channel. The control plane stamps each line's
//! arrival time and keeps the last [`RING_CAPACITY`] lines per run in a bounded ring
//! buffer, so `GET /v1/runs/{id}/logs` can replay the recent tail before following the
//! live broadcast.
//!
//! This is an in-memory, best-effort tail (not the durable event log): it lives in the
//! control-plane process and is intentionally bounded. Lines are redacted *again* on
//! write (see the HTTP handler) as defense-in-depth.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use cat_herder_domain::entities::RunId;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// How many lines to keep per run in the replay buffer.
pub const RING_CAPACITY: usize = 2000;

/// How many live messages a subscriber may fall behind before it observes a lag.
const BROADCAST_CAPACITY: usize = 256;

/// One spooled log line, as surfaced by `GET /v1/runs/{id}/logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLogLine {
    /// Source label: "llm" | "tool" | "stdout" | "stderr" | "system".
    pub stream: String,
    pub line: String,
    /// Arrival time (epoch milliseconds), stamped by the control plane on write.
    pub occurred_at: i64,
}

/// A per-run entry: the bounded ring buffer and its live broadcast channel.
struct RunEntry {
    ring: VecDeque<RunLogLine>,
    tx: broadcast::Sender<RunLogLine>,
}

impl RunEntry {
    fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            ring: VecDeque::with_capacity(RING_CAPACITY),
            tx,
        }
    }
}

/// In-memory spool of harness log lines, shared across the gRPC ingest path and the
/// HTTP read path. Cloneable handle over shared state (matches [`crate::AppState`]).
#[derive(Clone, Default)]
pub struct RunLogs {
    inner: Arc<Mutex<HashMap<RunId, RunEntry>>>,
}

impl RunLogs {
    /// A fresh, empty spool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a log line for `run_id`, stamping its arrival time, then fan it out to
    /// live subscribers. The ring keeps only the most recent [`RING_CAPACITY`] lines.
    pub fn push(&self, run_id: RunId, stream: impl Into<String>, line: impl Into<String>) {
        let entry = RunLogLine {
            stream: stream.into(),
            line: line.into(),
            occurred_at: now_millis(),
        };
        let mut map = self.inner.lock().expect("run-logs mutex poisoned");
        let run = map.entry(run_id).or_insert_with(RunEntry::new);
        if run.ring.len() >= RING_CAPACITY {
            run.ring.pop_front();
        }
        run.ring.push_back(entry.clone());
        // A send error just means no live subscribers; the ring still holds the line.
        let _ = run.tx.send(entry);
    }

    /// A snapshot of the buffered lines for `run_id` (oldest first).
    pub fn buffered(&self, run_id: RunId) -> Vec<RunLogLine> {
        let map = self.inner.lock().expect("run-logs mutex poisoned");
        map.get(&run_id)
            .map(|e| e.ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Subscribe to live lines for `run_id`. Subscribe *before* calling
    /// [`Self::buffered`] to avoid dropping a line published in between.
    pub fn subscribe(&self, run_id: RunId) -> broadcast::Receiver<RunLogLine> {
        let mut map = self.inner.lock().expect("run-logs mutex poisoned");
        let run = map.entry(run_id).or_insert_with(RunEntry::new);
        run.tx.subscribe()
    }
}

/// Current wall-clock time as epoch milliseconds (the spool's arrival stamp).
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn push_spools_and_replays_in_order() {
        let logs = RunLogs::new();
        logs.push(RunId(1), "stdout", "hello");
        logs.push(RunId(1), "llm", "world");

        let buffered = logs.buffered(RunId(1));
        assert_eq!(buffered.len(), 2);
        assert_eq!(buffered[0].stream, "stdout");
        assert_eq!(buffered[0].line, "hello");
        assert_eq!(buffered[1].line, "world");
        assert!(buffered[0].occurred_at > 0);

        // A different run's buffer is independent.
        assert!(logs.buffered(RunId(2)).is_empty());
    }

    #[tokio::test]
    async fn ring_buffer_is_bounded() {
        let logs = RunLogs::new();
        for i in 0..(RING_CAPACITY + 50) {
            logs.push(RunId(1), "stdout", format!("line {i}"));
        }
        let buffered = logs.buffered(RunId(1));
        assert_eq!(buffered.len(), RING_CAPACITY);
        // The oldest 50 fell off; the newest line is retained.
        assert_eq!(buffered[0].line, format!("line {}", 50));
        assert_eq!(
            buffered.last().unwrap().line,
            format!("line {}", RING_CAPACITY + 49)
        );
    }

    #[tokio::test]
    async fn subscribers_receive_live_lines() {
        let logs = RunLogs::new();
        let mut rx = logs.subscribe(RunId(7));
        logs.push(RunId(7), "tool", "executing");

        let got = rx.recv().await.expect("live line delivered");
        assert_eq!(got.stream, "tool");
        assert_eq!(got.line, "executing");
    }
}
