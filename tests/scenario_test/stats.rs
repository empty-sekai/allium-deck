use super::verify::{CaseRun, CaseStatus};

#[derive(Clone, Debug)]
pub struct ScenarioSummary {
    pub passed: usize,
    pub better: usize,
    pub bug: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub p99_ms: f64,
    pub avg_pool: usize,
    pub avg_leaves: u64,
}

pub fn summarize(case_runs: &[CaseRun]) -> ScenarioSummary {
    let mut timings = case_runs.iter().map(|case| case.elapsed_ms).collect::<Vec<_>>();
    timings.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let len = case_runs.len().max(1);
    let total_ms = case_runs.iter().map(|case| case.elapsed_ms).sum::<f64>();
    let total_pool = case_runs.iter().map(|case| case.pool_size).sum::<usize>();
    let total_leaves = case_runs
        .iter()
        .map(|case| case.stats.leaf_nodes)
        .sum::<u64>();
    let p99_index = ((timings.len().saturating_sub(1)) * 99) / 100;

    ScenarioSummary {
        passed: case_runs
            .iter()
            .filter(|case| matches!(case.status, CaseStatus::Passed))
            .count(),
        better: case_runs
            .iter()
            .filter(|case| matches!(case.status, CaseStatus::Better))
            .count(),
        bug: case_runs
            .iter()
            .filter(|case| matches!(case.status, CaseStatus::Bug))
            .count(),
        avg_ms: total_ms / len as f64,
        max_ms: timings.last().copied().unwrap_or(0.0),
        p99_ms: timings.get(p99_index).copied().unwrap_or(0.0),
        avg_pool: total_pool / len,
        avg_leaves: total_leaves / len as u64,
    }
}

pub fn print_summary(name: &str, case_runs: &[CaseRun]) {
    let summary = summarize(case_runs);
    eprintln!(
        "=== {name} ({} cases) ===\n  passed: {}  better: {}  bug: {}\n  avg: {:.3}ms  max: {:.3}ms  p99: {:.3}ms\n  avg_pool: {}  avg_leaves: {}",
        case_runs.len(),
        summary.passed,
        summary.better,
        summary.bug,
        summary.avg_ms,
        summary.max_ms,
        summary.p99_ms,
        summary.avg_pool,
        summary.avg_leaves,
    );
}
