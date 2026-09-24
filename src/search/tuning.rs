//! Switches for cross-checking pruning mechanisms against each other.
//! Production searches always run the default; tests override it per thread.
//! Knobs can change bounds and traversal, but cannot trim the feasible input.
#[derive(Clone, Copy, Debug)]
pub(super) struct SearchTuning {
    pub bounds: bool,
    pub dominance: bool,
    pub world_bloom_attr_matching: bool,
    pub final_attr_dp: bool,
    pub final_seeds: bool,
    /// Exact bonus tiers build their attribute-limited views and tier
    /// certificates from the first node instead of after a node count.
    pub eager_bonus_tiers: bool,
}

impl Default for SearchTuning {
    fn default() -> Self {
        Self {
            bounds: true,
            dominance: true,
            world_bloom_attr_matching: true,
            final_attr_dp: true,
            final_seeds: true,
            eager_bonus_tiers: false,
        }
    }
}

impl SearchTuning {
    /// Read at preparation boundaries, never inside a node loop.
    #[inline(always)]
    pub fn load() -> Self {
        #[cfg(test)]
        if let Some(value) = OVERRIDE.with(|slot| slot.get()) {
            return value;
        }
        Self::default()
    }
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
