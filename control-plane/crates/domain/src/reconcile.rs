//! Reconcile logic: map execution-runtime status onto lifecycle transitions.

use crate::entities::RunState;

/// Execution-runtime status, as reported by the [`crate::ports::AgentRuntime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// The execution exists but is not ready yet.
    Pending,
    /// The execution is ready / running.
    Ready,
    /// The backing pod finished successfully (terminal phase, exit 0).
    Succeeded,
    /// The backing pod failed (crash / liveness-killed / OOM).
    Failed,
    /// The sandbox's `shutdownTime` hard deadline was reached.
    Expired,
}

/// Determine the lifecycle transitions to apply given the current state and the
/// runtime status (`None` = the execution is gone). Returns the (possibly
/// multi-step) sequence of states to walk through; empty means no change.
pub fn reconcile_run(state: RunState, status: Option<RuntimeStatus>) -> Vec<RunState> {
    use RunState::*;
    use RuntimeStatus as RS;

    match (state, status) {
        // Becoming ready advances pending -> running.
        (PodPending, Some(RS::Ready)) => vec![Running],
        // A pending run whose sandbox already reached a terminal outcome is
        // fast-forwarded through Running to that terminal state.
        (PodPending, Some(RS::Succeeded)) => vec![Running, Succeeded],
        (PodPending, Some(RS::Failed)) => vec![Running, Stuck],
        (PodPending, Some(RS::Expired)) => vec![Running, TimedOut],
        // Running outcomes.
        (Running, Some(RS::Succeeded)) => vec![Succeeded],
        (Running, Some(RS::Failed)) => vec![Stuck],
        (Running, Some(RS::Expired)) => vec![TimedOut],
        // A missing sandbox means the execution is gone. The `finished` event is the
        // normal terminal signal, so this is only a fallback (treat as clean).
        (Running, None) => vec![Succeeded],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_pending_becomes_running_when_ready() {
        assert_eq!(
            reconcile_run(RunState::PodPending, Some(RuntimeStatus::Ready)),
            vec![RunState::Running]
        );
    }

    #[test]
    fn running_becomes_succeeded_when_gone() {
        assert_eq!(reconcile_run(RunState::Running, None), vec![RunState::Succeeded]);
    }

    #[test]
    fn running_becomes_stuck_when_pod_failed() {
        assert_eq!(
            reconcile_run(RunState::Running, Some(RuntimeStatus::Failed)),
            vec![RunState::Stuck]
        );
    }

    #[test]
    fn running_becomes_timed_out_when_expired() {
        assert_eq!(
            reconcile_run(RunState::Running, Some(RuntimeStatus::Expired)),
            vec![RunState::TimedOut]
        );
    }

    #[test]
    fn pending_terminal_outcomes_fast_forward_through_running() {
        assert_eq!(
            reconcile_run(RunState::PodPending, Some(RuntimeStatus::Failed)),
            vec![RunState::Running, RunState::Stuck]
        );
        assert_eq!(
            reconcile_run(RunState::PodPending, Some(RuntimeStatus::Expired)),
            vec![RunState::Running, RunState::TimedOut]
        );
        assert_eq!(
            reconcile_run(RunState::PodPending, Some(RuntimeStatus::Succeeded)),
            vec![RunState::Running, RunState::Succeeded]
        );
    }

    #[test]
    fn no_change_for_stable_or_irrelevant_states() {
        assert!(reconcile_run(RunState::PodPending, Some(RuntimeStatus::Pending)).is_empty());
        assert!(reconcile_run(RunState::PodPending, None).is_empty());
        assert!(reconcile_run(RunState::Running, Some(RuntimeStatus::Ready)).is_empty());
        assert!(reconcile_run(RunState::Running, Some(RuntimeStatus::Pending)).is_empty());
        assert!(reconcile_run(RunState::Queued, Some(RuntimeStatus::Ready)).is_empty());
        assert!(reconcile_run(RunState::Succeeded, None).is_empty());
        assert!(reconcile_run(RunState::Stuck, Some(RuntimeStatus::Failed)).is_empty());
    }
}
