//! Lifecycle state machine — the single source of truth for valid transitions
//! (`PLAN.md` §8).

use crate::entities::RunState;
use RunState::*;

/// Whether `from -> to` is a valid lifecycle transition.
pub fn can_transition(from: RunState, to: RunState) -> bool {
    matches!(
        (from, to),
        // queuing / scheduling
        (Queued, Scheduled)
            | (Queued, Cancelled)
            | (Scheduled, PodPending)
            | (Scheduled, Cancelled)
            | (PodPending, Running)
            | (PodPending, Cancelled)
            // running
            | (Running, AwaitingInput)
            | (Running, BudgetPaused)
            | (Running, Succeeded)
            | (Running, Failed)
            | (Running, TimedOut)
            | (Running, Cancelled)
            | (Running, Stuck)
            // human-in-the-loop
            | (AwaitingInput, Running)
            | (AwaitingInput, Cancelled)
            | (AwaitingInput, TimedOut)
            // budget
            | (BudgetPaused, Running)
            | (BudgetPaused, Cancelled)
            // result review
            | (Succeeded, AwaitingReview)
            | (AwaitingReview, Reviewed)
            | (AwaitingReview, Rejected)
    )
}

/// Leaf states — no outgoing transitions (excluding `Succeeded`, which may still
/// proceed to `AwaitingReview`).
pub fn is_terminal(state: RunState) -> bool {
    matches!(
        state,
        Reviewed | Rejected | Failed | TimedOut | Cancelled | Stuck
    )
}

/// Error returned when an invalid lifecycle transition is attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionError {
    pub from: RunState,
    pub to: RunState,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid state transition {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for TransitionError {}

impl crate::entities::Run {
    /// Attempt to transition this run to `to`, enforcing the lifecycle state machine.
    /// Sets `started_at` on entry to `Running` and `finished_at` on completion
    /// (`Succeeded` or any terminal state).
    pub fn transition(&mut self, to: RunState) -> Result<(), TransitionError> {
        if !can_transition(self.state, to) {
            return Err(TransitionError {
                from: self.state,
                to,
            });
        }
        self.state = to;
        if to == Running {
            self.started_at = Some(std::time::SystemTime::now());
        }
        if to == Succeeded || is_terminal(to) {
            self.finished_at = Some(std::time::SystemTime::now());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [RunState; 14] = [
        Queued,
        Scheduled,
        PodPending,
        Running,
        AwaitingInput,
        BudgetPaused,
        Succeeded,
        AwaitingReview,
        Reviewed,
        Rejected,
        Failed,
        TimedOut,
        Cancelled,
        Stuck,
    ];

    #[test]
    fn happy_path_is_valid() {
        assert!(can_transition(Queued, Scheduled));
        assert!(can_transition(Scheduled, PodPending));
        assert!(can_transition(PodPending, Running));
        assert!(can_transition(Running, Succeeded));
    }

    #[test]
    fn hitl_round_trip_is_valid() {
        assert!(can_transition(Running, AwaitingInput));
        assert!(can_transition(AwaitingInput, Running));
        assert!(can_transition(AwaitingInput, Cancelled));
    }

    #[test]
    fn budget_and_review_flows_are_valid() {
        assert!(can_transition(Running, BudgetPaused));
        assert!(can_transition(BudgetPaused, Running));
        assert!(can_transition(Succeeded, AwaitingReview));
        assert!(can_transition(AwaitingReview, Reviewed));
        assert!(can_transition(AwaitingReview, Rejected));
    }

    #[test]
    fn cancellation_from_any_nonterminal_is_valid() {
        for from in [
            Queued,
            Scheduled,
            PodPending,
            Running,
            AwaitingInput,
            BudgetPaused,
        ] {
            assert!(can_transition(from, Cancelled), "{from:?} -> Cancelled");
        }
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        for state in ALL {
            if !is_terminal(state) {
                continue;
            }
            for next in ALL {
                assert!(
                    !can_transition(state, next),
                    "{state:?} -> {next:?} should be invalid"
                );
            }
        }
    }

    #[test]
    fn backwards_transitions_are_invalid() {
        assert!(!can_transition(Running, Queued));
        assert!(!can_transition(Running, Scheduled));
        assert!(!can_transition(Succeeded, Running));
        assert!(!can_transition(Reviewed, Running));
        assert!(!can_transition(Failed, Running));
    }

    #[test]
    fn is_terminal_marks_leaf_states() {
        for s in [Reviewed, Rejected, Failed, TimedOut, Cancelled, Stuck] {
            assert!(is_terminal(s), "{s:?} should be terminal");
        }
        for s in [
            Running,
            AwaitingInput,
            BudgetPaused,
            Succeeded,
            AwaitingReview,
        ] {
            assert!(!is_terminal(s), "{s:?} should not be terminal");
        }
    }

    #[test]
    fn transition_to_valid_state_succeeds_and_mutates() {
        let mut run = crate::entities::Run {
            id: crate::entities::RunId(1),
            kind: crate::entities::RunKind::Agent,
            state: Queued,
            task_spec: "test".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: std::time::SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        assert!(run.transition(Scheduled).is_ok());
        assert_eq!(run.state, Scheduled);
    }

    #[test]
    fn transition_to_invalid_state_errors_without_mutating() {
        let mut run = crate::entities::Run {
            id: crate::entities::RunId(1),
            kind: crate::entities::RunKind::Agent,
            state: Queued,
            task_spec: "test".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: std::time::SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        let err = run.transition(Running).unwrap_err();
        assert_eq!(err.from, Queued);
        assert_eq!(err.to, Running);
        assert_eq!(run.state, Queued); // unchanged
    }

    #[test]
    fn happy_path_walk_reaches_succeeded() {
        let mut run = crate::entities::Run {
            id: crate::entities::RunId(1),
            kind: crate::entities::RunKind::Agent,
            state: Queued,
            task_spec: "test".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: std::time::SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        for next in [Scheduled, PodPending, Running, Succeeded] {
            run.transition(next).expect("valid transition");
        }
        assert_eq!(run.state, Succeeded);
    }
}
