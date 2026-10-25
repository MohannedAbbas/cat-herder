//! Reconcile logic: map execution-runtime status onto lifecycle transitions.

use crate::entities::RunState;

/// Execution-runtime status, as reported by the [`crate::ports::AgentRuntime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// The execution exists but is not ready yet.
    Pending,
    /// The execution is ready / running.
    Ready,
}

/// Determine the next run state given its current state and the runtime status
/// (`None` = the execution is gone). Returns `None` when no transition should happen.
pub fn reconcile_run(state: RunState, status: Option<RuntimeStatus>) -> Option<RunState> {
    use RunState::*;
    use RuntimeStatus::*;

    match (state, status) {
        (PodPending, Some(Ready)) => Some(Running),
        (Running, None) => Some(Succeeded),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_pending_becomes_running_when_ready() {
        assert_eq!(
            reconcile_run(RunState::PodPending, Some(RuntimeStatus::Ready)),
            Some(RunState::Running)
        );
    }

    #[test]
    fn running_becomes_succeeded_when_gone() {
        assert_eq!(reconcile_run(RunState::Running, None), Some(RunState::Succeeded));
    }

    #[test]
    fn no_change_for_stable_or_irrelevant_states() {
        assert_eq!(reconcile_run(RunState::PodPending, Some(RuntimeStatus::Pending)), None);
        assert_eq!(reconcile_run(RunState::PodPending, None), None);
        assert_eq!(reconcile_run(RunState::Running, Some(RuntimeStatus::Ready)), None);
        assert_eq!(reconcile_run(RunState::Queued, Some(RuntimeStatus::Ready)), None);
        assert_eq!(reconcile_run(RunState::Succeeded, None), None);
    }
}
