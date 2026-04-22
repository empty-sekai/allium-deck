use crate::pool::{CardIdx, CardPool};
use crate::types::{ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::evaluate::{card_proxy_bonus, leaf_evaluate_checked};
use super::types::DeckResult;

const FINAL_CHAPTER_WARM_START_LEADERS: usize = 20;

#[derive(Clone, Copy)]
enum Strategy {
    Power,
    Skill,
    Target,
}

/// 生成热启动下界。
pub fn warm_start(pool: &CardPool, ctx: &SearchContext) -> u64 {
    warm_start_best(pool, ctx)
        .map(|result| result.score)
        .unwrap_or(0)
}

pub(crate) fn warm_start_best(pool: &CardPool, ctx: &SearchContext) -> Option<DeckResult> {
    let mut best: Option<DeckResult> = None;
    let final_chapter_leaders = if ctx.is_final_chapter {
        top_final_chapter_leaders(pool, ctx)
    } else {
        [None; FINAL_CHAPTER_WARM_START_LEADERS]
    };
    for strategy in [Strategy::Power, Strategy::Skill, Strategy::Target] {
        if ctx.is_final_chapter {
            for leader in final_chapter_leaders.into_iter().flatten() {
                let Some(deck) = greedy_select(pool, ctx, strategy, Some(leader)) else {
                    continue;
                };
                let improved = one_swap_improve(pool, ctx, deck, Some(leader));
                if let Some(score) = leaf_evaluate_checked(pool, ctx, &improved) {
                    promote_best(&mut best, DeckResult::new(improved, score));
                }
            }
        } else if let Some(deck) = greedy_select(pool, ctx, strategy, None) {
            let improved = one_swap_improve(pool, ctx, deck, None);
            if let Some(score) = leaf_evaluate_checked(pool, ctx, &improved) {
                promote_best(&mut best, DeckResult::new(improved, score));
            }
        }
    }
    best
}

fn top_final_chapter_leaders(
    pool: &CardPool,
    ctx: &SearchContext,
) -> [Option<CardIdx>; FINAL_CHAPTER_WARM_START_LEADERS] {
    let mut leaders = [None; FINAL_CHAPTER_WARM_START_LEADERS];
    for candidate in pool.indices() {
        if !slot_matches(pool, ctx, 0, candidate) {
            continue;
        }
        let mut pos = 0usize;
        while pos < FINAL_CHAPTER_WARM_START_LEADERS {
            let should_insert = match leaders[pos] {
                Some(current) => leader_better(pool, candidate, current),
                None => true,
            };
            if should_insert {
                let mut shift = FINAL_CHAPTER_WARM_START_LEADERS - 1;
                while shift > pos {
                    leaders[shift] = leaders[shift - 1];
                    shift -= 1;
                }
                leaders[pos] = Some(candidate);
                break;
            }
            pos += 1;
        }
    }
    leaders
}

fn leader_better(pool: &CardPool, candidate: CardIdx, current: CardIdx) -> bool {
    let candidate_power = pool.power_max(candidate);
    let current_power = pool.power_max(current);
    candidate_power > current_power
        || (candidate_power == current_power && candidate.raw() < current.raw())
}

fn greedy_select(
    pool: &CardPool,
    ctx: &SearchContext,
    strategy: Strategy,
    fixed_leader: Option<CardIdx>,
) -> Option<[CardIdx; 5]> {
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    let mut used_chars = 0u32;
    let mut filled = 0usize;
    if let Some(leader) = fixed_leader {
        deck[0] = leader;
        used_chars |= 1u32 << pool.char_id(leader);
        filled = 1;
    }

    while filled < DECK_SIZE {
        let mut candidate = None;
        let mut candidate_score = f64::NEG_INFINITY;
        for card in pool.indices() {
            if used_chars & (1u32 << pool.char_id(card)) != 0 {
                continue;
            }
            if fixed_leader.is_some_and(|leader| leader == card) {
                continue;
            }
            if !slot_matches(pool, ctx, filled, card) {
                continue;
            }
            let score = strategy_score(pool, ctx, strategy, card);
            if score > candidate_score
                || (score == candidate_score
                    && candidate.is_some_and(|best_card: CardIdx| card.raw() < best_card.raw()))
            {
                candidate = Some(card);
                candidate_score = score;
            }
        }

        let next = candidate?;
        deck[filled] = next;
        used_chars |= 1u32 << pool.char_id(next);
        filled += 1;
    }

    Some(deck)
}

fn one_swap_improve(
    pool: &CardPool,
    ctx: &SearchContext,
    mut deck: [CardIdx; 5],
    fixed_leader: Option<CardIdx>,
) -> [CardIdx; 5] {
    let Some(mut best_score) = leaf_evaluate_checked(pool, ctx, &deck) else {
        return deck;
    };
    let start_slot = if fixed_leader.is_some() { 1 } else { 0 };

    loop {
        let mut improved = false;
        let mut slot = start_slot;
        while slot < DECK_SIZE {
            if ctx.is_fixed_slot(slot) {
                slot += 1;
                continue;
            }
            let original = unsafe { *deck.get_unchecked(slot) };
            let mut best_card = original;
            let mut best_slot_score = best_score;
            for candidate in pool.indices() {
                if candidate == original || fixed_leader.is_some_and(|leader| leader == candidate) {
                    continue;
                }
                if !slot_matches(pool, ctx, slot, candidate) {
                    continue;
                }

                let mut conflict = false;
                let cand_char = pool.char_id(candidate);
                let mut idx = 0usize;
                while idx < DECK_SIZE {
                    if idx != slot {
                        let current = unsafe { *deck.get_unchecked(idx) };
                        if current == candidate || pool.char_id(current) == cand_char {
                            conflict = true;
                            break;
                        }
                    }
                    idx += 1;
                }
                if conflict {
                    continue;
                }

                unsafe {
                    *deck.get_unchecked_mut(slot) = candidate;
                }
                let Some(score) = leaf_evaluate_checked(pool, ctx, &deck) else {
                    unsafe {
                        *deck.get_unchecked_mut(slot) = original;
                    }
                    continue;
                };
                if score > best_slot_score {
                    best_slot_score = score;
                    best_card = candidate;
                }
                unsafe {
                    *deck.get_unchecked_mut(slot) = original;
                }
            }

            if best_card != original {
                unsafe {
                    *deck.get_unchecked_mut(slot) = best_card;
                }
                best_score = best_slot_score;
                improved = true;
            }
            slot += 1;
        }

        if !improved {
            break;
        }
    }

    deck
}

fn strategy_score(pool: &CardPool, ctx: &SearchContext, strategy: Strategy, card: CardIdx) -> f64 {
    match strategy {
        Strategy::Power => pool.power_max(card) as f64,
        Strategy::Skill => pool.skill_max(card) as f64,
        Strategy::Target => match ctx.target {
            ScoreTarget::Power => pool.power_max(card) as f64,
            ScoreTarget::Skill => pool.skill_max(card) as f64,
            ScoreTarget::Bonus => card_proxy_bonus(pool, ctx, card, false) as f64,
            ScoreTarget::Score | ScoreTarget::Mysekai => {
                let bonus = card_proxy_bonus(pool, ctx, card, false);
                ctx.w_power * pool.power_max(card) as f64 + ctx.w_bonus * bonus as f64
            }
        },
    }
}

fn promote_best(best: &mut Option<DeckResult>, candidate: DeckResult) {
    let replace = match best {
        Some(current) => candidate.score > current.score,
        None => true,
    };
    if replace {
        *best = Some(candidate);
    }
}

#[inline(always)]
fn slot_matches(pool: &CardPool, ctx: &SearchContext, slot: usize, card: CardIdx) -> bool {
    if let Some(game_id) = ctx.fixed_card_at(slot) {
        if pool.game_id(card) != game_id {
            return false;
        }
    }
    if let Some(character_id) = ctx.fixed_character_at(slot) {
        if pool.char_id(card) != character_id {
            return false;
        }
    }
    true
}
