//! Subset-sum reachability for bonus-target searches.
//!
//! Bucket targets that no combination of [`DECK_SIZE`] cards can hit are
//! provably unsatisfiable, yet the DFS used to keep exploring them until the
//! deadline (empty buckets have no live threshold, so nothing was pruned).
//! This precomputes, for every suffix of the card order and every pick count,
//! the set of achievable total bonus sums in 0.1% units (`total_x10`), so the
//! tracker can prune a bucket as soon as its target interval is unreachable
//! from the remaining cards.

use super::DECK_SIZE;
use crate::pool::{CardIdx, CardPool};

/// Word length of one bitset (one bit per achievable `total_x10` sum).
pub struct BonusReach {
    words: usize,
    /// `levels[pos][r]`: bitset of achievable sums (x10 units) picking `r`
    /// cards from `cards[pos..]`; `pos` in `0..=n`.
    levels: Vec<[Vec<u64>; DECK_SIZE + 1]>,
    max_sum: u32,
}

impl BonusReach {
    pub fn build(pool: &CardPool) -> Self {
        let n = pool.count();
        let bonuses: Vec<u32> = (0..n)
            .map(|idx| pool.event_bonus(CardIdx::new(idx as u16)).total_x10() as u32)
            .collect();
        let mut sorted = bonuses.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        let max_sum: u32 = sorted.iter().take(DECK_SIZE).sum();
        let words = (max_sum as usize) / 64 + 2;
        let mut levels: Vec<[Vec<u64>; DECK_SIZE + 1]> =
            vec![std::array::from_fn(|_| vec![0u64; words]); n + 1];
        // Empty suffix: only zero picks with sum zero.
        levels[n][0][0] = 1;
        for pos in (0..n).rev() {
            let shift = bonuses[pos] as usize;
            let (wshift, bshift) = (shift / 64, shift % 64);
            for r in 0..=DECK_SIZE {
                // Skip picking card `pos`.
                levels[pos][r] = levels[pos + 1][r].clone();
                if r == 0 {
                    continue;
                }
                // Pick card `pos`: shift the (r-1)-suffix sums by its bonus.
                let src = levels[pos + 1][r - 1].clone();
                let dst = &mut levels[pos][r];
                for (i, word) in dst.iter_mut().enumerate() {
                    let mut value = *word;
                    if bshift == 0 {
                        if i >= wshift {
                            value |= src[i - wshift];
                        }
                    } else {
                        if i > wshift {
                            value |= src[i - wshift - 1] >> (64 - bshift);
                        }
                        if i >= wshift {
                            value |= src[i - wshift] << bshift;
                        }
                    }
                    *word = value;
                }
            }
        }
        Self {
            words,
            levels,
            max_sum,
        }
    }

    /// Whether picking `r` cards from `cards[pos..]` can reach a total bonus
    /// sum inside the inclusive `[lo, hi]` range (x10 units).
    pub fn any_in_range(&self, pos: usize, r: usize, lo: u32, hi: u32) -> bool {
        if r > DECK_SIZE || pos >= self.levels.len() {
            return false;
        }
        let reach = &self.levels[pos][r];
        let hi = (hi as usize).min(self.max_sum as usize);
        let lo = (lo as usize).min(hi);
        let (word_lo, bit_lo) = (lo / 64, lo % 64);
        let (word_hi, bit_hi) = (hi / 64, hi % 64);
        for (word, &bits) in reach.iter().enumerate().take(word_hi + 1).skip(word_lo) {
            let mut mask = u64::MAX;
            if word == word_lo && bit_lo != 0 {
                mask &= !0u64 << bit_lo;
            }
            if word == word_hi && bit_hi != 63 {
                mask &= (1u64 << (bit_hi + 1)) - 1;
            }
            if bits & mask != 0 {
                return true;
            }
        }
        let _ = self.words;
        false
    }
}
