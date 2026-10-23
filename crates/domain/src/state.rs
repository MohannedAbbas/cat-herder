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
        for from in [Queued, Scheduled, PodPending, Running, AwaitingInput, BudgetPaused] {
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
        for s in [Running, AwaitingInput, BudgetPaused, Succeeded, AwaitingReview] {
            assert!(!is_terminal(s), "{s:?} should not be terminal");
        }
    }
}
