//! Task-completion scoring (`PLAN.md` §17): SWE-bench-style pass/fail of an agent's
//! produced diff/patch against held-out tests. Pure — the actual test *runner* (apply
//! the patch, execute the test command in a sandbox) is the evaluation harness's
//! follow-on; this module is the scoring core it feeds.

/// The outcome of running one held-out test against the agent's produced patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
}

/// An evaluation of one task: the tests that must go from failing to passing (the
/// task's acceptance tests) and the tests that must keep passing (no regression).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Eval {
    pub fail_to_pass: Vec<TestResult>,
    pub pass_to_pass: Vec<TestResult>,
}

impl Eval {
    /// Whether the task is "resolved": every fail-to-pass test now passes **and** every
    /// pass-to-pass test still passes. A task is resolved only when both halves hold.
    pub fn resolved(&self) -> bool {
        self.fail_to_pass.iter().all(|t| t.passed)
            && self.pass_to_pass.iter().all(|t| t.passed)
    }

    /// The pass rate across all tests (0.0–1.0). Zero tests counts as `1.0` (nothing to
    /// regress), matching SWE-bench's per-instance denominator.
    pub fn pass_rate(&self) -> f64 {
        let total = self.fail_to_pass.len() + self.pass_to_pass.len();
        if total == 0 {
            return 1.0;
        }
        let passed = self
            .fail_to_pass
            .iter()
            .chain(self.pass_to_pass.iter())
            .filter(|t| t.passed)
            .count();
        passed as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(name: &str, passed: bool) -> TestResult {
        TestResult {
            name: name.to_string(),
            passed,
        }
    }

    #[test]
    fn resolved_requires_both_halves() {
        let eval = Eval {
            fail_to_pass: vec![res("test_a", true)],
            pass_to_pass: vec![res("test_b", true)],
        };
        assert!(eval.resolved());

        // A failing pass-to-pass (regression) fails the whole task.
        let regressed = Eval {
            fail_to_pass: vec![res("test_a", true)],
            pass_to_pass: vec![res("test_b", false)],
        };
        assert!(!regressed.resolved());

        // A still-failing fail-to-pass fails the task.
        let incomplete = Eval {
            fail_to_pass: vec![res("test_a", false)],
            pass_to_pass: vec![res("test_b", true)],
        };
        assert!(!incomplete.resolved());
    }

    #[test]
    fn pass_rate_counts_all_tests() {
        let eval = Eval {
            fail_to_pass: vec![res("a", true), res("b", false)],
            pass_to_pass: vec![res("c", true)],
        };
        assert!((eval.pass_rate() - 2.0 / 3.0).abs() < 1e-9);

        let empty = Eval::default();
        assert_eq!(empty.pass_rate(), 1.0);
    }
}
