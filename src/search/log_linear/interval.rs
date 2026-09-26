//! Outward binary64 intervals for preparing a pruning certificate.
//! No platform logarithm participates: ln uses range reduction and a positive
//! atanh series with an explicit geometric remainder (pruning-proof §18.8).
use std::ops::{Add, Div, Mul, Sub};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub(super) struct Interval {
    pub(super) lo: f64,
    pub(super) hi: f64,
}

impl Interval {
    const INVALID: Self = Self {
        lo: f64::NAN,
        hi: f64::NAN,
    };

    pub(super) fn point(value: f64) -> Self {
        Self {
            lo: value,
            hi: value,
        }
    }

    pub(super) fn finite(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }

    /// Monotonicity of ln reduces an interval argument to its endpoints.
    pub(super) fn ln(self) -> Self {
        if !self.finite() || self.lo <= 0.0 {
            return Self::INVALID;
        }
        Self {
            lo: ln_point(self.lo).lo,
            hi: ln_point(self.hi).hi,
        }
    }
}

impl Add for Interval {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        if !self.finite() || !other.finite() {
            return Self::INVALID;
        }
        if self.lo == 0.0 && self.hi == 0.0 {
            return other;
        }
        if other.lo == 0.0 && other.hi == 0.0 {
            return self;
        }
        Self {
            lo: (self.lo + other.lo).next_down(),
            hi: (self.hi + other.hi).next_up(),
        }
    }
}
impl Sub for Interval {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        if !self.finite() || !other.finite() {
            return Self::INVALID;
        }
        Self {
            lo: (self.lo - other.hi).next_down(),
            hi: (self.hi - other.lo).next_up(),
        }
    }
}
impl Mul for Interval {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        if !self.finite() || !other.finite() {
            return Self::INVALID;
        }
        if (self.lo == 0.0 && self.hi == 0.0) || (other.lo == 0.0 && other.hi == 0.0) {
            return Self::point(0.0);
        }
        let values = [
            self.lo * other.lo,
            self.lo * other.hi,
            self.hi * other.lo,
            self.hi * other.hi,
        ];
        Self {
            lo: values.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            hi: values
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max)
                .next_up(),
        }
    }
}
impl Div for Interval {
    type Output = Self;
    fn div(self, other: Self) -> Self {
        if !self.finite() || !other.finite() || (other.lo <= 0.0 && other.hi >= 0.0) {
            return Self::INVALID;
        }
        let values = [
            self.lo / other.lo,
            self.lo / other.hi,
            self.hi / other.lo,
            self.hi / other.hi,
        ];
        Self {
            lo: values.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            hi: values
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max)
                .next_up(),
        }
    }
}

/// Encloses ln(m) for 1 <= m <= 2. With z=(m-1)/(m+1), 0 <= z <= 1/3:
/// ln(m)=2*sum(z^(2j+1)/(2j+1), j=0..31)+R,
/// 0 <= R <= 2*z^65/(65*(1-z^2)). All endpoints are rounded outward.
fn ln_mantissa(m: f64) -> Interval {
    debug_assert!((1.0..=2.0).contains(&m));
    if m == 1.0 {
        return Interval::point(0.0);
    }
    let one = Interval::point(1.0);
    let two = Interval::point(2.0);
    let mut z = (Interval::point(m) - one) / (Interval::point(m) + one);
    z.lo = z.lo.max(0.0);
    let square = z * z;
    let mut power = z;
    let mut sum = Interval::point(0.0);
    for j in 0..32 {
        sum = sum + power / Interval::point(f64::from(2 * j + 1));
        power = power * square;
    }
    let remainder = two * power / (Interval::point(65.0) * (one - square));
    two * sum
        + Interval {
            lo: 0.0,
            hi: remainder.hi,
        }
}

fn ln_point(mut x: f64) -> Interval {
    if !x.is_finite() || x <= 0.0 {
        return Interval::INVALID;
    }
    let mut shift = 0;
    if x < f64::MIN_POSITIVE {
        // Exact power-of-two scaling, including the smallest subnormal.
        x *= 4_503_599_627_370_496.0;
        shift = 52;
    }
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - shift;
    let mantissa = f64::from_bits((bits & ((1_u64 << 52) - 1)) | (1023_u64 << 52));
    static LN_TWO: OnceLock<Interval> = OnceLock::new();
    let ln_two = *LN_TWO.get_or_init(|| ln_mantissa(2.0));
    ln_mantissa(mantissa) + Interval::point(f64::from(exponent)) * ln_two
}

/// Outward primitives for applying the already-prepared affine certificate.
#[inline(always)]
pub(crate) fn add_up(left: f64, right: f64) -> f64 {
    (left + right).next_up()
}
#[inline(always)]
pub(crate) fn mul_up(left: f64, right: f64) -> f64 {
    (left * right).next_up()
}
#[inline(always)]
pub(crate) fn div_up(left: f64, right: f64) -> f64 {
    (left / right).next_up()
}
#[inline(always)]
pub(crate) fn sub_down(left: f64, right: f64) -> f64 {
    (left - right).next_down()
}
#[inline]
pub(crate) fn sum_up(values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(0.0, add_up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_division_refuses_a_zero_crossing() {
        assert!(!(Interval::point(1.0) / Interval { lo: -1.0, hi: 1.0 }).finite());
    }

    #[test]
    // These are adjacent enclosing endpoints, deliberately not rounded constants.
    #[allow(clippy::approx_constant)]
    fn logarithm_encloses_reference_brackets() {
        for (x, lo, hi) in [
            (1.0, 0.0, 0.0),
            (2.0, 0.6931471805599453, 0.6931471805599454),
            (0.5, -0.6931471805599454, -0.6931471805599453),
            (10.0, 2.3025850929940455, 2.302585092994046),
            (100.0, 4.605170185988091, 4.605170185988092),
        ] {
            let value = Interval::point(x).ln();
            assert!(
                value.finite() && value.lo <= lo && value.hi >= hi,
                "{x}: {value:?}"
            );
        }
    }

    #[test]
    fn logarithm_is_finite_at_binary64_extremes() {
        for x in [
            f64::from_bits(1),
            f64::MIN_POSITIVE,
            1.0_f64.next_up(),
            f64::MAX,
        ] {
            let value = Interval::point(x).ln();
            assert!(value.finite(), "{x}: {value:?}");
            // Diagnostic comparison only; production does not trust libm ln.
            assert!(value.lo <= x.ln() && value.hi >= x.ln(), "{x}: {value:?}");
        }
    }
}
