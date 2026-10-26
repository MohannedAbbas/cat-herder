//! Sharding: partition reconcile work by a stable hash of the run id.
//!
//! The orchestrator is a *sharded* reconciler (`PLAN.md` §6): reconcile work is
//! partitioned by `run_id` hash across N leaders, each owning a shard via a lease.
//! A run's shard is a pure function of its id, so every replica (and a restarting
//! replica) agrees on which shard owns which run without coordination.

use crate::entities::RunId;

/// Which shard (in `0..shard_count`) a run belongs to.
///
/// Uses a SplitMix64 finalizer so ids that are assigned sequentially still spread
/// uniformly across shards (a plain `% shard_count` would do that too, but a real
/// mix keeps the mapping stable and uniform even if ids become non-sequential or
/// come from an external source). Deterministic across processes and restarts.
pub fn shard_for(run_id: RunId, shard_count: u32) -> u32 {
    if shard_count <= 1 {
        return 0;
    }
    let mut z = run_id.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z % shard_count as u64) as u32
}

/// Whether `shard` owns `run_id` under `shard_count`.
pub fn owns(run_id: RunId, shard_count: u32, shard: u32) -> bool {
    shard_for(run_id, shard_count) == shard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_is_stable_and_in_range() {
        let count = 16;
        for id in 0..10_000u64 {
            let shard = shard_for(RunId(id), count);
            assert!(shard < count, "shard {shard} out of range for id {id}");
            // Deterministic: recomputing gives the same answer.
            assert_eq!(shard, shard_for(RunId(id), count));
        }
    }

    #[test]
    fn single_shard_owns_everything() {
        assert_eq!(shard_for(RunId(42), 1), 0);
        assert!(owns(RunId(42), 1, 0));
    }

    #[test]
    fn ids_spread_across_shards() {
        // Over a contiguous range the shards must all be represented (no degenerate
        // clustering onto one shard).
        let count = 16;
        let mut seen = [false; 16];
        for id in 0..256u64 {
            seen[shard_for(RunId(id), count) as usize] = true;
        }
        assert!(seen.iter().all(|s| *s), "not all shards represented");
    }

    #[test]
    fn owns_matches_shard_for() {
        let (count, shard) = (8, 3);
        let id = RunId(1001);
        assert_eq!(owns(id, count, shard), shard_for(id, count) == shard);
        assert!(!owns(RunId(1002), 8, 3) || shard_for(RunId(1002), 8) == 3);
    }
}
