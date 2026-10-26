//! Canonical event envelope (`PLAN.md` §9).
//!
//! One append-only, replayable log carries two projections: lifecycle events
//! (`run.*`, emitted by the control plane) and agent events (`agent.*`, normalized
//! from the harness). The control plane is the sole writer and assigns the per-run
//! monotonic `seq`.

use std::time::SystemTime;

use serde_json::Value;

use crate::entities::{Run, RunId};

/// The current envelope schema version. Bump only on a breaking envelope change.
pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// Well-known event type names. Namespaced: `run.*` (lifecycle projection) and
/// `agent.*` (agent projection). New types are additive; consumers must tolerate
/// unknown types (forward compatibility).
pub mod event_type {
    // Lifecycle (control-plane-emitted).
    pub const RUN_QUEUED: &str = "run.queued";
    pub const RUN_SCHEDULED: &str = "run.scheduled";
    pub const RUN_POD_PENDING: &str = "run.pod_pending";
    pub const RUN_RUNNING: &str = "run.running";
    pub const RUN_SUCCEEDED: &str = "run.succeeded";
    pub const RUN_FAILED: &str = "run.failed";
    pub const RUN_STUCK: &str = "run.stuck";
    pub const RUN_TIMED_OUT: &str = "run.timed_out";
    pub const RUN_CANCELLED: &str = "run.cancelled";
    pub const RUN_AWAITING_INPUT: &str = "run.awaiting_input";
    pub const RUN_BUDGET_PAUSED: &str = "run.budget_paused";
    pub const RUN_AWAITING_REVIEW: &str = "run.awaiting_review";
    pub const RUN_REVIEWED: &str = "run.reviewed";
    pub const RUN_REJECTED: &str = "run.rejected";

    // Agent (harness-sourced, normalized by the control plane).
    pub const AGENT_STARTED: &str = "agent.started";
    pub const AGENT_FINISHED: &str = "agent.finished";
    pub const AGENT_STUCK: &str = "agent.stuck";
}

/// Canonical event envelope. `seq` is assigned by the control plane (sole writer);
/// it is `0` until persisted by an [`crate::ports::EventRepository`].
#[derive(Debug, Clone)]
pub struct Event {
    pub run_id: RunId,
    pub seq: i64,
    pub event_type: String,
    pub schema_version: u32,
    pub occurred_at: SystemTime,
    pub actor: Option<String>,
    pub payload: Value,
}

impl Event {
    /// Build a new event (unpersisted, `seq = 0`).
    pub fn new(
        run_id: RunId,
        event_type: impl Into<String>,
        actor: Option<String>,
        payload: Value,
    ) -> Self {
        Event {
            run_id,
            seq: 0,
            event_type: event_type.into(),
            schema_version: ENVELOPE_SCHEMA_VERSION,
            occurred_at: SystemTime::now(),
            actor,
            payload,
        }
    }

    /// A lifecycle event carrying the run's current state as its payload.
    fn lifecycle(run: &Run, event_type: impl Into<String>) -> Self {
        Self::new(
            run.id,
            event_type,
            run.actor.clone(),
            serde_json::json!({ "state": run.state }),
        )
    }

    pub fn run_queued(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_QUEUED)
    }

    pub fn run_scheduled(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_SCHEDULED)
    }

    pub fn run_pod_pending(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_POD_PENDING)
    }

    pub fn run_running(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_RUNNING)
    }

    pub fn run_succeeded(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_SUCCEEDED)
    }

    pub fn run_failed(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_FAILED)
    }

    pub fn run_stuck(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_STUCK)
    }

    pub fn run_timed_out(run: &Run) -> Self {
        Self::lifecycle(run, event_type::RUN_TIMED_OUT)
    }

    /// The lifecycle event for a run's *current* state. Used by the reconciler and
    /// the terminal-transition paths so every state change lands in the log exactly
    /// once, with the state it represents.
    pub fn lifecycle_for(run: &Run) -> Self {
        use crate::entities::RunState::*;
        match run.state {
            Queued => Self::run_queued(run),
            Scheduled => Self::run_scheduled(run),
            PodPending => Self::run_pod_pending(run),
            Running => Self::run_running(run),
            Succeeded => Self::run_succeeded(run),
            Failed => Self::run_failed(run),
            Stuck => Self::run_stuck(run),
            TimedOut => Self::run_timed_out(run),
            // States without a wired event type yet fall back to a generic lifecycle
            // event carrying the state, so nothing is silently dropped from the log.
            _ => Self::lifecycle(run, "run.state"),
        }
    }
}
