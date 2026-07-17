#[inline]
pub(crate) fn avx512_candidate_mask_available() -> bool {
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
#[inline(always)]
pub(crate) unsafe fn unused_character_mask_16(char_ids: *const u8, used_chars: u32) -> u16 {
    #[cfg(target_arch = "x86_64")]
    {
        unused_character_mask_16_avx512(char_ids, used_chars)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (char_ids, used_chars);
        0
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn unused_character_mask_16_avx512(char_ids: *const u8, used_chars: u32) -> u16 {
    use std::arch::x86_64::*;

    let packed = _mm_loadu_si128(char_ids.cast::<__m128i>());
    let characters = _mm512_cvtepu8_epi32(packed);
    let character_bits = _mm512_sllv_epi32(_mm512_set1_epi32(1), characters);
    let conflicts = _mm512_and_si512(character_bits, _mm512_set1_epi32(used_chars as i32));
    _mm512_cmpeq_epi32_mask(conflicts, _mm512_setzero_si512()) as u16
}

#[cfg(test)]
mod tests {
    use super::{avx512_candidate_mask_available, unused_character_mask_16};

    #[test]
    fn avx512_mask_rejects_used_characters() {
        if !avx512_candidate_mask_available() {
            return;
        }
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
}
