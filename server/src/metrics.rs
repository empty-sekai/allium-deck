//! Prometheus text-format metrics, hand-rolled.
//!
//! The service exports a small, fixed set of series, so a registry crate would add a
//! dependency without removing work. Histogram buckets span sub-millisecond to ten
//! seconds because that is the real spread of this workload: a routine multi/score
//! request lands in microseconds, while World Bloom and final-chapter searches run for
//! hundreds of milliseconds.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::pool::PoolMetrics;

/// Upper bounds, in seconds, of the histogram buckets.
const BOUNDS: [f64; 12] = [
    0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Endpoints that carry their own timing series.
pub const ENDPOINTS: [&str; 6] = [
    "recommend",
    "challenge_all",
    "world_bloom_support_cards",
    "music_recommend",
    "live_exact_score",
    "area_items_recommend",
];

/// Response classes counted per endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    BadRequest,
    Overloaded,
    Timeout,
    Error,
}

impl Outcome {
    const ALL: [Outcome; 5] = [
        Outcome::Ok,
        Outcome::BadRequest,
        Outcome::Overloaded,
        Outcome::Timeout,
        Outcome::Error,
    ];

    fn label(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::BadRequest => "bad_request",
            Outcome::Overloaded => "overloaded",
            Outcome::Timeout => "timeout",
            Outcome::Error => "error",
        }
    }

    fn index(self) -> usize {
        match self {
            Outcome::Ok => 0,
            Outcome::BadRequest => 1,
            Outcome::Overloaded => 2,
            Outcome::Timeout => 3,
            Outcome::Error => 4,
        }
    }
}

#[derive(Default)]
struct Histogram {
    buckets: [AtomicU64; BOUNDS.len()],
    /// Accumulated in microseconds to keep the running total on integer atomics.
    sum_micros: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    fn observe(&self, seconds: f64) {
        for (index, bound) in BOUNDS.iter().enumerate() {
            if seconds <= *bound {
                self.buckets[index].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.sum_micros
            .fetch_add((seconds * 1_000_000.0) as u64, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn render(&self, out: &mut String, name: &str, endpoint: &str) {
        for (index, bound) in BOUNDS.iter().enumerate() {
            let value = self.buckets[index].load(Ordering::Relaxed);
            let _ = writeln!(
                out,
                "{name}_bucket{{endpoint=\"{endpoint}\",le=\"{bound}\"}} {value}"
            );
        }
        let count = self.count.load(Ordering::Relaxed);
        let sum = self.sum_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        let _ = writeln!(
            out,
            "{name}_bucket{{endpoint=\"{endpoint}\",le=\"+Inf\"}} {count}"
        );
        let _ = writeln!(out, "{name}_sum{{endpoint=\"{endpoint}\"}} {sum}");
        let _ = writeln!(out, "{name}_count{{endpoint=\"{endpoint}\"}} {count}");
    }
}

#[derive(Default)]
struct EndpointMetrics {
    requests: [AtomicU64; 5],
    duration: Histogram,
    queue_wait: Histogram,
    build_pool: Histogram,
    search: Histogram,
}

/// All exported series.
pub struct Metrics {
    endpoints: Vec<EndpointMetrics>,
    search_timeouts: AtomicU64,
    reloads: AtomicU64,
    pool: Arc<PoolMetrics>,
}

impl Metrics {
    pub fn new(pool: Arc<PoolMetrics>) -> Self {
        Self {
            endpoints: ENDPOINTS
                .iter()
                .map(|_| EndpointMetrics::default())
                .collect(),
            search_timeouts: AtomicU64::new(0),
            reloads: AtomicU64::new(0),
            pool,
        }
    }

    fn endpoint(&self, name: &str) -> Option<&EndpointMetrics> {
        let index = ENDPOINTS.iter().position(|known| *known == name)?;
        self.endpoints.get(index)
    }

    /// Records one finished request.
    pub fn record(&self, endpoint: &str, outcome: Outcome, seconds: f64) {
        let Some(metrics) = self.endpoint(endpoint) else {
            return;
        };
        metrics.requests[outcome.index()].fetch_add(1, Ordering::Relaxed);
        metrics.duration.observe(seconds);
    }

    /// Records the per-stage split of a request that reached the engine.
    ///
    /// This split is what tells a deployment where its time actually goes — queueing,
    /// pool building, or the search itself — rather than only the total.
    pub fn record_stages(
        &self,
        endpoint: &str,
        queue_wait_seconds: f64,
        build_pool_seconds: f64,
        search_seconds: f64,
    ) {
        let Some(metrics) = self.endpoint(endpoint) else {
            return;
        };
        metrics.queue_wait.observe(queue_wait_seconds);
        metrics.build_pool.observe(build_pool_seconds);
        metrics.search.observe(search_seconds);
    }

    pub fn record_search_timeout(&self) {
        self.search_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_reload(&self) {
        self.reloads.fetch_add(1, Ordering::Relaxed);
    }

    /// Renders the Prometheus text exposition format.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(8 * 1024);

        out.push_str("# HELP alliumdeck_requests_total Requests by endpoint and outcome.\n");
        out.push_str("# TYPE alliumdeck_requests_total counter\n");
        for (index, endpoint) in ENDPOINTS.iter().enumerate() {
            let Some(metrics) = self.endpoints.get(index) else {
                continue;
            };
            for outcome in Outcome::ALL {
                let value = metrics.requests[outcome.index()].load(Ordering::Relaxed);
                let _ = writeln!(
                    out,
                    "alliumdeck_requests_total{{endpoint=\"{endpoint}\",outcome=\"{}\"}} {value}",
                    outcome.label()
                );
            }
        }

        for (name, help, select) in [
            (
                "alliumdeck_request_duration_seconds",
                "Wall time from accepting a request to writing its response.",
                0usize,
            ),
            (
                "alliumdeck_queue_wait_seconds",
                "Time a request spent queued before a search thread claimed it.",
                1,
            ),
            (
                "alliumdeck_build_pool_seconds",
                "Time spent building the candidate pool.",
                2,
            ),
            (
                "alliumdeck_search_seconds",
                "Time spent in the search itself.",
                3,
            ),
        ] {
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} histogram");
            for (index, endpoint) in ENDPOINTS.iter().enumerate() {
                let Some(metrics) = self.endpoints.get(index) else {
                    continue;
                };
                let histogram = match select {
                    0 => &metrics.duration,
                    1 => &metrics.queue_wait,
                    2 => &metrics.build_pool,
                    _ => &metrics.search,
                };
                histogram.render(&mut out, name, endpoint);
            }
        }

        let _ = writeln!(
            out,
            "# HELP alliumdeck_pool_queued Jobs waiting for a search thread.\n\
             # TYPE alliumdeck_pool_queued gauge\n\
             alliumdeck_pool_queued {}",
            self.pool.queued.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "# HELP alliumdeck_pool_running Jobs currently executing.\n\
             # TYPE alliumdeck_pool_running gauge\n\
             alliumdeck_pool_running {}",
            self.pool.running.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "# HELP alliumdeck_pool_rejected_total Requests refused by the queue.\n\
             # TYPE alliumdeck_pool_rejected_total counter\n\
             alliumdeck_pool_rejected_total{{reason=\"queue_full\"}} {}\n\
             alliumdeck_pool_rejected_total{{reason=\"queue_timeout\"}} {}",
            self.pool.rejected_full.load(Ordering::Relaxed),
            self.pool.rejected_timeout.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "# HELP alliumdeck_worker_panics_total Jobs that panicked.\n\
             # TYPE alliumdeck_worker_panics_total counter\n\
             alliumdeck_worker_panics_total {}",
            self.pool.panics.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "# HELP alliumdeck_search_timeouts_total Searches that hit their deadline and \
             returned the best decks found so far.\n\
             # TYPE alliumdeck_search_timeouts_total counter\n\
             alliumdeck_search_timeouts_total {}",
            self.search_timeouts.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "# HELP alliumdeck_masterdata_reloads_total Successful masterdata reloads.\n\
             # TYPE alliumdeck_masterdata_reloads_total counter\n\
             alliumdeck_masterdata_reloads_total {}",
            self.reloads.load(Ordering::Relaxed)
        );

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_cumulative_and_counted_once() {
        let metrics = Metrics::new(Arc::new(PoolMetrics::default()));
        metrics.record("recommend", Outcome::Ok, 0.003);
        let text = metrics.render();

        // 0.003s falls above the 0.001 bound and at or below every bound from 0.005 up.
        assert!(text.contains(
            "alliumdeck_request_duration_seconds_bucket{endpoint=\"recommend\",le=\"0.001\"} 0"
        ));
        assert!(text.contains(
            "alliumdeck_request_duration_seconds_bucket{endpoint=\"recommend\",le=\"0.005\"} 1"
        ));
        assert!(text.contains(
            "alliumdeck_request_duration_seconds_bucket{endpoint=\"recommend\",le=\"+Inf\"} 1"
        ));
        assert!(
            text.contains("alliumdeck_request_duration_seconds_count{endpoint=\"recommend\"} 1")
        );
        assert!(
            text.contains("alliumdeck_requests_total{endpoint=\"recommend\",outcome=\"ok\"} 1")
        );
    }

    #[test]
    fn an_observation_past_the_last_bound_still_counts() {
        let metrics = Metrics::new(Arc::new(PoolMetrics::default()));
        metrics.record("recommend", Outcome::Ok, 30.0);
        let text = metrics.render();
        assert!(text.contains(
            "alliumdeck_request_duration_seconds_bucket{endpoint=\"recommend\",le=\"10\"} 0"
        ));
        assert!(text.contains(
            "alliumdeck_request_duration_seconds_bucket{endpoint=\"recommend\",le=\"+Inf\"} 1"
        ));
    }

    #[test]
    fn unknown_endpoints_are_ignored_rather_than_panicking() {
        let metrics = Metrics::new(Arc::new(PoolMetrics::default()));
        metrics.record("not_an_endpoint", Outcome::Ok, 1.0);
        assert!(!metrics.render().contains("not_an_endpoint"));
    }
}
