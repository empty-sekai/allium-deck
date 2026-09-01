#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimdBackend {
    Scalar,
    Avx512,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PowerCommon16 {
    pub(crate) character_bonus: [i32; 16],
    pub(crate) fixture_bonus: [i32; 16],
    pub(crate) gate_bonus: [i32; 16],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PowerAreaItem {
    pub(crate) unit: u8,
    pub(crate) attr: u8,
    pub(crate) character_id: i32,
    pub(crate) power_rate: f64,
    pub(crate) power_all_match_rate: f64,
}

impl PowerAreaItem {
    pub(crate) const ANY: u8 = u8::MAX;
    pub(crate) const ANY_CHARACTER: i32 = -1;
}

impl PowerCommon16 {
    const EMPTY: Self = Self {
        character_bonus: [0; 16],
        fixture_bonus: [0; 16],
        gate_bonus: [0; 16],
    };
}

impl SimdBackend {
    #[inline]
    pub(crate) fn detect() -> Self {
        static BACKEND: OnceLock<SimdBackend> = OnceLock::new();
        *BACKEND.get_or_init(|| {
            if avx512_available_uncached() {
                Self::Avx512
            } else {
                Self::Scalar
            }
        })
    }

    #[inline(always)]
    pub(crate) unsafe fn power_common_16(
        self,
        base_dims: &[[i32; 16]; 3],
        base_sum: &[i32; 16],
        character_rates: &[f32; 16],
        fixture_rates: &[f64; 16],
        gate_rates: &[f64; 16],
        valid_lanes: usize,
    ) -> PowerCommon16 {
        unsafe {
            let mut result = match self {
                Self::Scalar => power_common_16_scalar(
                    base_dims,
                    base_sum,
                    character_rates,
                    fixture_rates,
                    gate_rates,
                    valid_lanes,
                ),
                Self::Avx512 => power_common_16_avx512_unchecked(
                    base_dims,
                    base_sum,
                    character_rates,
                    fixture_rates,
                    gate_rates,
                ),
            };
            let mut lane = valid_lanes;
            while lane < 16 {
                result.character_bonus[lane] = 0;
                result.fixture_bonus[lane] = 0;
                result.gate_bonus[lane] = 0;
                lane += 1;
            }
            result
        }
    }

    #[inline(always)]
    pub(crate) unsafe fn power_area_single_unit_16(
        self,
        base_dims: &[[i32; 16]; 3],
        target_units: &[u8; 16],
        attrs: &[u8; 16],
        character_ids: &[i32; 16],
        items: &[PowerAreaItem],
        member_key: usize,
        active_lanes: u16,
    ) -> [i32; 16] {
        unsafe {
            match self {
                Self::Scalar => power_area_single_unit_16_scalar(
                    base_dims,
                    target_units,
                    attrs,
                    character_ids,
                    items,
                    member_key,
                    active_lanes,
                ),
                Self::Avx512 => power_area_single_unit_16_avx512_unchecked(
                    base_dims,
                    target_units,
                    attrs,
                    character_ids,
                    items,
                    member_key,
                    active_lanes,
                ),
            }
        }
    }
}

#[inline]
pub(crate) fn avx512_available() -> bool {
    matches!(SimdBackend::detect(), SimdBackend::Avx512)
}

#[inline]
fn avx512_available_uncached() -> bool {
    if std::env::var_os("ALLIUM_DECK_FORCE_SCALAR").is_some() {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Returns one bit per candidate whose character is not present in `used_chars`.
///
/// The caller guarantees that `char_ids` contains at least 16 entries.
#[cfg(test)]
#[inline(always)]
pub(crate) unsafe fn unused_character_mask_16(char_ids: *const u8, used_chars: u32) -> u16 {
    // 安全性由调用方保证：`char_ids` 至少 16 项，直接转交给下游实现。
    unsafe {
        if avx512_available() {
            return unused_character_mask_16_avx512_unchecked(char_ids, used_chars);
        }
        unused_character_mask_16_scalar(char_ids, used_chars)
    }
}

/// Returns one bit per upper bound that is strictly above `threshold`.
///
/// The caller guarantees that `upper_bounds` contains at least 16 entries.
#[cfg(test)]
#[inline(always)]
pub(crate) unsafe fn upper_bound_mask_16(upper_bounds: *const u64, threshold: u64) -> u16 {
    // 安全性由调用方保证：`upper_bounds` 至少 16 项，直接转交给下游实现。
    unsafe {
        if avx512_available() {
            return upper_bound_mask_16_avx512_unchecked(upper_bounds, threshold);
        }
        upper_bound_mask_16_scalar(upper_bounds, threshold)
    }
}

#[cfg(any(test, not(target_arch = "x86_64")))]
#[inline(always)]
unsafe fn unused_character_mask_16_scalar(char_ids: *const u8, used_chars: u32) -> u16 {
    unsafe {
        let mut mask = 0u16;
        let mut lane = 0usize;
        while lane < 16 {
            let character = *char_ids.add(lane);
            mask |= (((used_chars & (1u32 << character)) == 0) as u16) << lane;
            lane += 1;
        }
        mask
    }
}

#[cfg(any(test, not(target_arch = "x86_64")))]
#[inline(always)]
unsafe fn upper_bound_mask_16_scalar(upper_bounds: *const u64, threshold: u64) -> u16 {
    unsafe {
        let mut mask = 0u16;
        let mut lane = 0usize;
        while lane < 16 {
            mask |= ((*upper_bounds.add(lane) > threshold) as u16) << lane;
            lane += 1;
        }
        mask
    }
}

#[inline(always)]
fn power_common_16_scalar(
    base_dims: &[[i32; 16]; 3],
    base_sum: &[i32; 16],
    character_rates: &[f32; 16],
    fixture_rates: &[f64; 16],
    gate_rates: &[f64; 16],
    valid_lanes: usize,
) -> PowerCommon16 {
    let mut result = PowerCommon16::EMPTY;
    let mut lane = 0usize;
    while lane < valid_lanes {
        let rate = character_rates[lane] * 0.01_f32;
        let mut character_bonus = 0i32;
        let mut dim = 0usize;
        while dim < 3 {
            character_bonus += (rate * base_dims[dim][lane] as f32).floor() as i32;
            dim += 1;
        }
        result.character_bonus[lane] = character_bonus;
        result.fixture_bonus[lane] =
            ((base_sum[lane] as f64 * fixture_rates[lane]) * 0.001_f64).floor() as i32;
        result.gate_bonus[lane] =
            ((base_sum[lane] as f64 * gate_rates[lane]) * 0.01_f64).floor() as i32;
        lane += 1;
    }
    result
}

#[inline(always)]
fn power_area_single_unit_16_scalar(
    base_dims: &[[i32; 16]; 3],
    target_units: &[u8; 16],
    attrs: &[u8; 16],
    character_ids: &[i32; 16],
    items: &[PowerAreaItem],
    member_key: usize,
    active_lanes: u16,
) -> [i32; 16] {
    let mut result = [0i32; 16];
    let mut lanes = active_lanes;
    while lanes != 0 {
        let lane = lanes.trailing_zeros() as usize;
        lanes &= lanes - 1;
        let mut acc = [0.0_f64; 3];
        for item in items {
            if item.unit != PowerAreaItem::ANY && item.unit != target_units[lane] {
                continue;
            }
            if item.attr != PowerAreaItem::ANY && item.attr != attrs[lane] {
                continue;
            }
            if item.character_id != PowerAreaItem::ANY_CHARACTER
                && item.character_id != character_ids[lane]
            {
                continue;
            }
            let all_match = (item.unit != PowerAreaItem::ANY && member_key >= 2)
                || (item.attr != PowerAreaItem::ANY && member_key % 2 == 1);
            let rate = if all_match {
                item.power_all_match_rate
            } else {
                item.power_rate
            };
            let factor = rate * 0.01_f64;
            let mut dim = 0usize;
            while dim < 3 {
                acc[dim] += factor * base_dims[dim][lane] as f64;
                dim += 1;
            }
        }
        result[lane] = acc[0].floor() as i32 + acc[1].floor() as i32 + acc[2].floor() as i32;
    }
    result
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn power_area_single_unit_16_avx512_unchecked(
    base_dims: &[[i32; 16]; 3],
    target_units: &[u8; 16],
    attrs: &[u8; 16],
    character_ids: &[i32; 16],
    items: &[PowerAreaItem],
    member_key: usize,
    active_lanes: u16,
) -> [i32; 16] {
    unsafe {
        use std::arch::x86_64::*;

        let packed_units = _mm_loadu_si128(target_units.as_ptr().cast::<__m128i>());
        let packed_attrs = _mm_loadu_si128(attrs.as_ptr().cast::<__m128i>());
        let units = _mm512_cvtepu8_epi32(packed_units);
        let attributes = _mm512_cvtepu8_epi32(packed_attrs);
        let characters = _mm512_loadu_si512(character_ids.as_ptr().cast::<__m512i>());
        let mut acc = [[_mm512_setzero_pd(); 2]; 3];

        for item in items {
            let mut lanes = active_lanes;
            if item.unit != PowerAreaItem::ANY {
                lanes &= _mm512_cmpeq_epi32_mask(units, _mm512_set1_epi32(item.unit as i32)) as u16;
            }
            if item.attr != PowerAreaItem::ANY {
                lanes &=
                    _mm512_cmpeq_epi32_mask(attributes, _mm512_set1_epi32(item.attr as i32)) as u16;
            }
            if item.character_id != PowerAreaItem::ANY_CHARACTER {
                lanes &= _mm512_cmpeq_epi32_mask(characters, _mm512_set1_epi32(item.character_id))
                    as u16;
            }
            if lanes == 0 {
                continue;
            }
            let all_match = (item.unit != PowerAreaItem::ANY && member_key >= 2)
                || (item.attr != PowerAreaItem::ANY && member_key % 2 == 1);
            let rate = if all_match {
                item.power_all_match_rate
            } else {
                item.power_rate
            } * 0.01_f64;
            let rate = _mm512_set1_pd(rate);
            let mut dim = 0usize;
            while dim < 3 {
                let mut half = 0usize;
                while half < 2 {
                    let offset = half * 8;
                    let mask = ((lanes >> offset) & 0xff) as u8;
                    if mask != 0 {
                        let base = _mm512_cvtepi32_pd(_mm256_loadu_si256(
                            base_dims[dim].as_ptr().add(offset).cast::<__m256i>(),
                        ));
                        acc[dim][half] = _mm512_mask_add_pd(
                            acc[dim][half],
                            mask,
                            acc[dim][half],
                            _mm512_mul_pd(rate, base),
                        );
                    }
                    half += 1;
                }
                dim += 1;
            }
        }

        let mut result = [0i32; 16];
        let mut half = 0usize;
        while half < 2 {
            let offset = half * 8;
            let mut sum = _mm256_setzero_si256();
            let mut dim = 0usize;
            while dim < 3 {
                sum = _mm256_add_epi32(
                    sum,
                    _mm512_cvttpd_epi32(_mm512_roundscale_pd::<0x09>(acc[dim][half])),
                );
                dim += 1;
            }
            _mm256_storeu_si256(result.as_mut_ptr().add(offset).cast::<__m256i>(), sum);
            half += 1;
        }
        result
    }
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn power_area_single_unit_16_avx512_unchecked(
    base_dims: &[[i32; 16]; 3],
    target_units: &[u8; 16],
    attrs: &[u8; 16],
    character_ids: &[i32; 16],
    items: &[PowerAreaItem],
    member_key: usize,
    active_lanes: u16,
) -> [i32; 16] {
    power_area_single_unit_16_scalar(
        base_dims,
        target_units,
        attrs,
        character_ids,
        items,
        member_key,
        active_lanes,
    )
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn power_common_16_avx512_unchecked(
    base_dims: &[[i32; 16]; 3],
    base_sum: &[i32; 16],
    character_rates: &[f32; 16],
    fixture_rates: &[f64; 16],
    gate_rates: &[f64; 16],
) -> PowerCommon16 {
    unsafe {
        use std::arch::x86_64::*;

        let rates = _mm512_mul_ps(
            _mm512_loadu_ps(character_rates.as_ptr()),
            _mm512_set1_ps(0.01_f32),
        );
        let mut character_bonus = _mm512_setzero_si512();
        let mut dim = 0usize;
        while dim < 3 {
            let base = _mm512_loadu_si512(base_dims[dim].as_ptr().cast::<__m512i>());
            let scaled = _mm512_mul_ps(rates, _mm512_cvtepi32_ps(base));
            character_bonus = _mm512_add_epi32(
                character_bonus,
                _mm512_cvttps_epi32(_mm512_roundscale_ps::<0x09>(scaled)),
            );
            dim += 1;
        }

        let mut result = PowerCommon16::EMPTY;
        _mm512_storeu_si512(
            result.character_bonus.as_mut_ptr().cast::<__m512i>(),
            character_bonus,
        );
        let mut half = 0usize;
        while half < 2 {
            let offset = half * 8;
            let sums = _mm512_cvtepi32_pd(_mm256_loadu_si256(
                base_sum.as_ptr().add(offset).cast::<__m256i>(),
            ));
            let fixture = _mm512_mul_pd(
                _mm512_mul_pd(sums, _mm512_loadu_pd(fixture_rates.as_ptr().add(offset))),
                _mm512_set1_pd(0.001_f64),
            );
            let gate = _mm512_mul_pd(
                _mm512_mul_pd(sums, _mm512_loadu_pd(gate_rates.as_ptr().add(offset))),
                _mm512_set1_pd(0.01_f64),
            );
            _mm256_storeu_si256(
                result
                    .fixture_bonus
                    .as_mut_ptr()
                    .add(offset)
                    .cast::<__m256i>(),
                _mm512_cvttpd_epi32(_mm512_roundscale_pd::<0x09>(fixture)),
            );
            _mm256_storeu_si256(
                result.gate_bonus.as_mut_ptr().add(offset).cast::<__m256i>(),
                _mm512_cvttpd_epi32(_mm512_roundscale_pd::<0x09>(gate)),
            );
            half += 1;
        }
        result
    }
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn power_common_16_avx512_unchecked(
    base_dims: &[[i32; 16]; 3],
    base_sum: &[i32; 16],
    character_rates: &[f32; 16],
    fixture_rates: &[f64; 16],
    gate_rates: &[f64; 16],
) -> PowerCommon16 {
    power_common_16_scalar(
        base_dims,
        base_sum,
        character_rates,
        fixture_rates,
        gate_rates,
        16,
    )
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
pub(crate) unsafe fn unused_character_mask_16_avx512_unchecked(
    char_ids: *const u8,
    used_chars: u32,
) -> u16 {
    unsafe {
        use std::arch::x86_64::*;

        let packed = _mm_loadu_si128(char_ids.cast::<__m128i>());
        let characters = _mm512_cvtepu8_epi32(packed);
        let character_bits = _mm512_sllv_epi32(_mm512_set1_epi32(1), characters);
        let conflicts = _mm512_and_si512(character_bits, _mm512_set1_epi32(used_chars as i32));
        _mm512_cmpeq_epi32_mask(conflicts, _mm512_setzero_si512()) as u16
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub(crate) unsafe fn upper_bound_mask_16_avx512_unchecked(
    upper_bounds: *const u64,
    threshold: u64,
) -> u16 {
    unsafe {
        use std::arch::x86_64::*;

        let threshold = _mm512_set1_epi64(threshold as i64);
        let lower = _mm512_loadu_si512(upper_bounds.cast::<__m512i>());
        let upper = _mm512_loadu_si512(upper_bounds.add(8).cast::<__m512i>());
        let lower_mask = _mm512_cmp_epu64_mask::<6>(lower, threshold) as u16;
        let upper_mask = _mm512_cmp_epu64_mask::<6>(upper, threshold) as u16;
        lower_mask | (upper_mask << 8)
    }
}
use std::sync::OnceLock;

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
pub(crate) unsafe fn unused_character_mask_16_avx512_unchecked(
    char_ids: *const u8,
    used_chars: u32,
) -> u16 {
    unsafe { unused_character_mask_16_scalar(char_ids, used_chars) }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
pub(crate) unsafe fn upper_bound_mask_16_avx512_unchecked(
    upper_bounds: *const u64,
    threshold: u64,
) -> u16 {
    unsafe { upper_bound_mask_16_scalar(upper_bounds, threshold) }
}

#[cfg(test)]
mod tests {
    use super::{PowerAreaItem, SimdBackend, unused_character_mask_16, upper_bound_mask_16};

    #[test]
    fn dispatched_mask_rejects_used_characters() {
        let characters = [0u8, 1, 7, 2, 7, 3, 4, 9, 5, 6, 9, 8, 10, 11, 12, 13];
        let used = (1u32 << 1) | (1u32 << 7) | (1u32 << 9) | (1u32 << 12);
        let actual = unsafe { unused_character_mask_16(characters.as_ptr(), used) };
        let expected = characters
            .into_iter()
            .enumerate()
            .fold(0u16, |mask, (lane, character)| {
                mask | (((used & (1u32 << character) == 0) as u16) << lane)
            });
        assert_eq!(actual, expected);
    }

    #[test]
    fn dispatched_mask_rejects_bounds_at_or_below_threshold() {
        let upper_bounds = [
            0,
            1,
            9,
            10,
            11,
            19,
            20,
            21,
            99,
            100,
            101,
            u32::MAX as u64,
            u32::MAX as u64 + 1,
            u64::MAX - 1,
            u64::MAX,
            10,
        ];
        let actual = unsafe { upper_bound_mask_16(upper_bounds.as_ptr(), 10) };
        let expected = upper_bounds
            .into_iter()
            .enumerate()
            .fold(0u16, |mask, (lane, upper)| {
                mask | (((upper > 10) as u16) << lane)
            });
        assert_eq!(actual, expected);
    }

    #[test]
    fn power_common_dispatch_matches_scalar_exactly() {
        let mut base_dims = [[0i32; 16]; 3];
        let mut base_sum = [0i32; 16];
        let mut character_rates = [0f32; 16];
        let mut fixture_rates = [0f64; 16];
        let mut gate_rates = [0f64; 16];
        for lane in 0..16 {
            base_dims[0][lane] = 10_001 + lane as i32 * 17;
            base_dims[1][lane] = 11_003 + lane as i32 * 19;
            base_dims[2][lane] = 12_007 + lane as i32 * 23;
            base_sum[lane] = base_dims[0][lane] + base_dims[1][lane] + base_dims[2][lane];
            character_rates[lane] = 0.5 + lane as f32 * 0.75;
            fixture_rates[lane] = (lane * 7) as f64;
            gate_rates[lane] = lane as f64 * 0.125;
        }
        let scalar = unsafe {
            SimdBackend::Scalar.power_common_16(
                &base_dims,
                &base_sum,
                &character_rates,
                &fixture_rates,
                &gate_rates,
                13,
            )
        };
        let dispatched = unsafe {
            SimdBackend::detect().power_common_16(
                &base_dims,
                &base_sum,
                &character_rates,
                &fixture_rates,
                &gate_rates,
                13,
            )
        };
        assert_eq!(dispatched, scalar);
    }

    #[test]
    fn power_area_dispatch_matches_scalar_exactly() {
        let mut base_dims = [[0i32; 16]; 3];
        let mut target_units = [0u8; 16];
        let mut attrs = [0u8; 16];
        let mut character_ids = [0i32; 16];
        for lane in 0..16 {
            base_dims[0][lane] = 20_003 + lane as i32 * 11;
            base_dims[1][lane] = 21_007 + lane as i32 * 13;
            base_dims[2][lane] = 22_009 + lane as i32 * 17;
            target_units[lane] = (lane % 6) as u8;
            attrs[lane] = (lane % 5) as u8;
            character_ids[lane] = (lane % 7 + 1) as i32;
        }
        let items = [
            PowerAreaItem {
                unit: PowerAreaItem::ANY,
                attr: PowerAreaItem::ANY,
                character_id: PowerAreaItem::ANY_CHARACTER,
                power_rate: 0.3,
                power_all_match_rate: 0.7,
            },
            PowerAreaItem {
                unit: 2,
                attr: PowerAreaItem::ANY,
                character_id: PowerAreaItem::ANY_CHARACTER,
                power_rate: 1.25,
                power_all_match_rate: 2.75,
            },
            PowerAreaItem {
                unit: PowerAreaItem::ANY,
                attr: 3,
                character_id: 4,
                power_rate: 0.85,
                power_all_match_rate: 1.65,
            },
        ];
        let active_lanes = 0b0111_1111_1111_1101;
        for member_key in 0..4 {
            let scalar = unsafe {
                SimdBackend::Scalar.power_area_single_unit_16(
                    &base_dims,
                    &target_units,
                    &attrs,
                    &character_ids,
                    &items,
                    member_key,
                    active_lanes,
                )
            };
            let dispatched = unsafe {
                SimdBackend::detect().power_area_single_unit_16(
                    &base_dims,
                    &target_units,
                    &attrs,
                    &character_ids,
                    &items,
                    member_key,
                    active_lanes,
                )
            };
            assert_eq!(dispatched, scalar);
        }
    }
}
