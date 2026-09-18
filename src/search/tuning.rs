//! Tuning is read at preparation boundaries, never inside a node loop.
//! Knobs can change bounds and traversal, but cannot trim the feasible input.
#[derive(Clone, Copy, Debug)]
pub(super) struct SearchTuning {
    pub bounds: bool,
    pub dominance: bool,
    pub simple_bound: bool,
    pub correlated_bound: bool,
    /// None selects the validated adaptive one/two-plane policy.
    pub correlated_planes: Option<usize>,
    pub correlated_trace: bool,
    pub warm_neighbors: bool,
    pub warm_candidate_limit: Option<usize>,
    pub world_bloom_attr_matching: bool,
    pub final_attr_dp: bool,
}

impl Default for SearchTuning {
    fn default() -> Self {
        Self {
            bounds: true,
            dominance: true,
            simple_bound: true,
            correlated_bound: true,
            correlated_planes: None,
            correlated_trace: false,
            warm_neighbors: true,
            warm_candidate_limit: None,
            world_bloom_attr_matching: true,
            final_attr_dp: true,
        }
    }
}

impl SearchTuning {
    pub fn load() -> Self {
        #[cfg(test)]
        if let Some(value) = OVERRIDE.with(|slot| slot.get()) {
            return value;
        }
        let plane_setting = std::env::var("ALLIUM_CORRELATED_PLANES").ok();
        Self {
            bounds: enabled("ALLIUM_SEARCH_BOUND"),
            dominance: enabled("ALLIUM_DOMINANCE"),
            simple_bound: enabled("ALLIUM_SIMPLE_EXACT_BOUND"),
            correlated_bound: enabled("ALLIUM_CORRELATED_BOUND"),
            correlated_planes: plane_setting.and_then(|value| {
                if value.eq_ignore_ascii_case("auto") {
                    None
                } else {
                    Some(value.parse::<usize>().unwrap_or(1).clamp(1, 4))
                }
            }),
            correlated_trace: std::env::var_os("ALLIUM_CORRELATED_TRACE").is_some(),
            warm_neighbors: enabled("ALLIUM_TOPK_WARM_NEIGHBORS"),
            warm_candidate_limit: std::env::var("ALLIUM_TOPK_WARM_CANDIDATES")
                .ok()
                .and_then(|value| value.parse().ok()),
            world_bloom_attr_matching: enabled("ALLIUM_WL_ATTR_MATCH"),
            final_attr_dp: enabled("ALLIUM_FINAL_ATTR_DP"),
        }
    }
}

fn enabled(name: &str) -> bool {
    !std::env::var_os(name).is_some_and(|value| value == "0")
}

#[cfg(test)]
std::thread_local! {
    static OVERRIDE: std::cell::Cell<Option<SearchTuning>> = const { std::cell::Cell::new(None) };
}

/// Thread-local and panic-safe: tests never mutate the process environment.
#[cfg(test)]
pub(super) fn with_tuning<R>(tuning: SearchTuning, operation: impl FnOnce() -> R) -> R {
    struct Restore(Option<SearchTuning>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|value| value.set(self.0));
        }
    }
    let guard = Restore(OVERRIDE.with(|value| value.replace(Some(tuning))));
    let result = operation();
    drop(guard);
    result
}
