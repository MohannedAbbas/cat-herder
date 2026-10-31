//! Cold-start latency metric (`PLAN.md` §16/§23): records trigger→running latency in a
//! Prometheus-style histogram, exposed by the control plane's `/metrics` endpoint so
//! the §23 SLO (p95 < 60s) can be measured.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Histogram bucket upper bounds, in seconds (a `+Inf` bucket is always appended).
const BUCKET_UPPERS_SECS: [f64; 11] = [0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0];

/// A lock-free, cumulative histogram of cold-start latencies. Buckets are cumulative
/// (each counts observations `<=` its upper bound), as Prometheus expects; the `+Inf`
/// bucket always counts every observation.
#[derive(Default)]
pub struct ColdStartHistogram {
    /// Cumulative counts, one per upper bound, plus the trailing `+Inf` bucket.
    buckets: [AtomicU64; BUCKET_UPPERS_SECS.len() + 1],
    /// Sum of observed latencies, in microseconds (avoids float atomics).
    sum_us: AtomicU64,
    /// Total number of observations.
    count: AtomicU64,
}

impl ColdStartHistogram {
    /// Record one observation of `seconds`.
    pub fn record(&self, seconds: f64) {
        let micros = (seconds.max(0.0) * 1_000_000.0).round() as u64;
        self.sum_us.fetch_add(micros, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        for (i, upper) in BUCKET_UPPERS_SECS.iter().enumerate() {
            if seconds <= *upper {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        // The `+Inf` bucket counts every observation.
        let inf = BUCKET_UPPERS_SECS.len();
        self.buckets[inf].fetch_add(1, Ordering::Relaxed);
    }

    /// The cumulative count of observations at or below each upper bound (including
    /// `+Inf`).
    pub fn bucket_counts(&self) -> Vec<u64> {
        self.buckets
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect()
    }

    /// Total observed latency, in seconds (sum / count when count > 0).
    pub fn sum_seconds(&self) -> f64 {
        self.sum_us.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Render in the Prometheus text exposition format.
    pub fn encode_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str("# TYPE cat_herder_cold_start_seconds histogram\n");
        for (upper, bucket) in BUCKET_UPPERS_SECS.iter().zip(self.buckets.iter()) {
            out.push_str(&format!(
                "cat_herder_cold_start_seconds_bucket{{le=\"{upper}\"}} {}\n",
                bucket.load(Ordering::Relaxed)
            ));
        }
        let inf = self.buckets[BUCKET_UPPERS_SECS.len()].load(Ordering::Relaxed);
        out.push_str(&format!(
            "cat_herder_cold_start_seconds_bucket{{le=\"+Inf\"}} {inf}\n"
        ));
        out.push_str(&format!(
            "cat_herder_cold_start_seconds_sum {}\n",
            self.sum_seconds()
        ));
        out.push_str(&format!(
            "cat_herder_cold_start_seconds_count {}\n",
            self.count()
        ));
        out
    }
}

/// The process-global cold-start histogram.
static COLD_START: OnceLock<ColdStartHistogram> = OnceLock::new();

/// The process-global cold-start histogram (initialized on first use).
pub fn cold_start() -> &'static ColdStartHistogram {
    COLD_START.get_or_init(ColdStartHistogram::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_records_cumulative_buckets_and_sum() {
        let h = ColdStartHistogram::default();
        h.record(1.0);
        h.record(20.0);
        h.record(90.0);

        let counts = h.bucket_counts();
        // +Inf counts everything; cumulative buckets are non-decreasing.
        assert_eq!(*counts.last().unwrap(), 3);
        // 1s and 20s are <= 30s; 90s is not.
        let le30 = BUCKET_UPPERS_SECS.iter().position(|&u| u == 30.0).unwrap();
        assert_eq!(counts[le30], 2);
        let le5 = BUCKET_UPPERS_SECS.iter().position(|&u| u == 5.0).unwrap();
        assert_eq!(counts[le5], 1);
        assert_eq!(h.count(), 3);
        assert!((h.sum_seconds() - 111.0).abs() < 0.001);
    }

    #[test]
    fn prometheus_encoding_is_well_formed() {
        let h = ColdStartHistogram::default();
        h.record(42.0);
        let text = h.encode_prometheus();
        assert!(text.contains("# TYPE cat_herder_cold_start_seconds histogram"));
        assert!(text.contains("cat_herder_cold_start_seconds_bucket{le=\"+Inf\"} 1"));
        assert!(text.contains("cat_herder_cold_start_seconds_count 1"));
        assert!(text.contains("cat_herder_cold_start_seconds_sum 42"));
    }
}
