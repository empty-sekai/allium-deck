//! One cooperative deadline shared by every phase of an operation.
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::Instant;
#[cfg(target_arch = "wasm32")]
pub(crate) use web_time::Instant;

use super::SearchParams;

pub(crate) struct SearchBudget {
    deadline: Option<Instant>,
    checks: u16,
    pub(crate) hit: bool,
}

impl SearchBudget {
    pub(crate) fn new(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            checks: 1023,
            hit: false,
        }
    }

    pub(crate) fn from_params(params: &SearchParams) -> Self {
        Self::new(
            (params.timeout_ms != 0)
                .then(|| Instant::now() + Duration::from_millis(params.timeout_ms)),
        )
    }

    /// Phase/job boundary. No clock access at all when the budget is unlimited.
    #[inline]
    pub(crate) fn expired(&mut self) -> bool {
        if self.hit {
            return true;
        }
        let Some(deadline) = self.deadline else {
            return false;
        };
        self.hit = Instant::now() >= deadline;
        self.hit
    }

    /// Hot loops sample once per 1024 checkpoints; expiry remains sticky.
    #[inline(always)]
    pub(crate) fn expired_sampled(&mut self) -> bool {
        self.expired_sampled_with(Instant::now)
    }

    #[inline(always)]
    pub(crate) fn expired_sampled_with(&mut self, now: impl FnOnce() -> Instant) -> bool {
        if self.hit {
            return true;
        }
        let Some(deadline) = self.deadline else {
            return false;
        };
        self.checks = self.checks.wrapping_add(1);
        if self.checks & 1023 == 0 {
            self.hit = now() >= deadline;
        }
        self.hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_is_sticky_and_unlimited_never_reads_clock() {
        let start = Instant::now();
        let end = start + Duration::from_secs(1);
        let mut budget = SearchBudget::new(Some(end));
        assert!(!budget.expired_sampled_with(|| start));
        for _ in 0..1023 {
            assert!(!budget.expired_sampled_with(|| panic!("unsampled clock read")));
        }
        assert!(budget.expired_sampled_with(|| end));
        for _ in 0..2048 {
            assert!(budget.expired_sampled_with(|| panic!("expired clock read")));
        }
        let mut unlimited = SearchBudget::new(None);
        for _ in 0..2048 {
            assert!(!unlimited.expired_sampled_with(|| panic!("unlimited clock read")));
        }
    }
}
