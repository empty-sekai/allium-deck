use crate::pool::{CardIdx, CardPool};
use crate::types::{ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::evaluate::{card_proxy_bonus, leaf_evaluate_checked};
use super::types::DeckResult;

const FINAL_CHAPTER_WARM_START_LEADERS: usize = 20;
const SCORE_EVENT_SOLO_WARM_START_PREFIX: usize = 16;
const FINAL_CHAPTER_EXACT_LEADERS: usize = 8;
const FINAL_CHAPTER_EXACT_PREFIX: usize = 24;
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
    if matches!(ctx.target, ScoreTarget::Score)
        && ctx.has_event()
        && matches!(
            ctx.effective_live_type(),
            crate::types::LiveType::Solo | crate::types::LiveType::Auto
        )
        && !ctx.is_final_chapter
    {
        best = warm_start_score_event_solo(pool, ctx, SCORE_EVENT_SOLO_WARM_START_PREFIX);
    }
    if ctx.is_final_chapter {
        best = warm_start_final_chapter(pool, ctx).or(best);
    }
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

fn warm_start_final_chapter(pool: &CardPool, ctx: &SearchContext) -> Option<DeckResult> {
    let leaders = sorted_final_chapter_leaders(pool, ctx)
        .into_iter()
        .take(FINAL_CHAPTER_EXACT_LEADERS)
        .collect::<Vec<_>>();
    let mut best = None;

    for leader in leaders {
        let members = sorted_final_chapter_members(pool, ctx, leader)
            .into_iter()
            .take(FINAL_CHAPTER_EXACT_PREFIX)
            .collect::<Vec<_>>();
        if members.len() + 1 < DECK_SIZE {
            continue;
        }
        let mut deck = [leader; DECK_SIZE];
        deck[0] = leader;
        warm_start_final_chapter_recurse(
            pool,
            ctx,
            leader,
            &members,
            1,
            0,
            1u32 << pool.char_id(leader),
            &mut deck,
            &mut best,
        );
    }

    let best = best?;
    let improved = one_swap_improve(pool, ctx, best.cards, Some(best.cards[0]));
    let score = leaf_evaluate_checked(pool, ctx, &improved)?;
    Some(DeckResult::new(improved, score))
}

fn warm_start_final_chapter_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    leader: CardIdx,
    members: &[CardIdx],
    depth: usize,
    start: usize,
    used_chars: u32,
    deck: &mut [CardIdx; DECK_SIZE],
    best: &mut Option<DeckResult>,
) {
    if depth == DECK_SIZE {
        if let Some(score) = leaf_evaluate_checked(pool, ctx, deck) {
            promote_best(best, DeckResult::new(*deck, score));
        }
        return;
    }

    let mut idx = start;
    while idx < members.len() {
        let card = members[idx];
        idx += 1;
        if card == leader {
            continue;
        }
        let char_id = pool.char_id(card);
        if ctx.enforce_char_uniqueness && used_chars & (1u32 << char_id) != 0 {
            continue;
        }
        deck[depth] = card;
        warm_start_final_chapter_recurse(
            pool,
            ctx,
            leader,
            members,
            depth + 1,
            idx,
            used_chars | (1u32 << char_id),
            deck,
            best,
        );
    }
}

fn warm_start_score_event_solo(
    pool: &CardPool,
    ctx: &SearchContext,
    prefix_len: usize,
) -> Option<DeckResult> {
    let mut cards = pool
        .indices()
        .map(|card| (score_event_solo_key(pool, ctx, card), card))
        .collect::<Vec<_>>();
    cards.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.raw().cmp(&right.1.raw()))
    });
    let prefix = cards
        .into_iter()
        .take(prefix_len)
        .map(|(_, card)| card)
        .collect::<Vec<_>>();
    if prefix.len() < DECK_SIZE {
        return None;
    }

    let mut deck = [prefix[0]; DECK_SIZE];
    let mut best = None;
    warm_start_prefix_recurse(pool, ctx, &prefix, 0, 0, 0, &mut deck, &mut best);
    let best = best?;
    let improved = one_swap_improve(pool, ctx, best.cards, None);
    let score = leaf_evaluate_checked(pool, ctx, &improved)?;
    Some(DeckResult::new(improved, score))
}

fn warm_start_prefix_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    prefix: &[CardIdx],
    depth: usize,
    start: usize,
    used_chars: u32,
    deck: &mut [CardIdx; DECK_SIZE],
    best: &mut Option<DeckResult>,
) {
    if depth == DECK_SIZE {
        if let Some(score) = leaf_evaluate_checked(pool, ctx, deck) {
            promote_best(best, DeckResult::new(*deck, score));
        }
        return;
    }

    let mut idx = start;
    while idx < prefix.len() {
        let card = prefix[idx];
        idx += 1;
        let char_id = pool.char_id(card);
        if ctx.enforce_char_uniqueness && used_chars & (1u32 << char_id) != 0 {
            continue;
        }
        if !slot_matches(pool, ctx, depth, card) {
            continue;
        }
        deck[depth] = card;
        warm_start_prefix_recurse(
            pool,
            ctx,
            prefix,
            depth + 1,
            idx,
            used_chars | (1u32 << char_id),
            deck,
            best,
        );
    }
}

fn top_final_chapter_leaders(
    pool: &CardPool,
    ctx: &SearchContext,
) -> [Option<CardIdx>; FINAL_CHAPTER_WARM_START_LEADERS] {
    let mut leaders = [None; FINAL_CHAPTER_WARM_START_LEADERS];
    for candidate in sorted_final_chapter_leaders(pool, ctx) {
        if !slot_matches(pool, ctx, 0, candidate) {
            continue;
        }
        let Some(slot) = leaders.iter().position(|leader| leader.is_none()) else {
            break;
        };
        leaders[slot] = Some(candidate);
        if slot + 1 >= FINAL_CHAPTER_WARM_START_LEADERS {
            break;
        }
    }
    leaders
}

pub(crate) fn sorted_final_chapter_leaders(pool: &CardPool, ctx: &SearchContext) -> Vec<CardIdx> {
    let mut leaders = pool.indices().collect::<Vec<_>>();
    leaders.sort_unstable_by(|left, right| {
        leader_key(pool, ctx, *right)
            .cmp(&leader_key(pool, ctx, *left))
            .then_with(|| left.raw().cmp(&right.raw()))
    });
    leaders
}

fn leader_key(pool: &CardPool, ctx: &SearchContext, leader: CardIdx) -> u64 {
    let power = pool.power_max(leader) as u64;
    let skill = pool.skill_max(leader) as u64;
    let bonus = card_proxy_bonus(pool, ctx, leader, true) as u64;
    power * (256 + skill) * (100 + bonus)
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
            if ctx.enforce_char_uniqueness && used_chars & (1u32 << pool.char_id(card)) != 0 {
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

                if ctx.enforce_char_uniqueness {
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
            ScoreTarget::Bonus => pool.event_bonus(card).total_rate(),
            ScoreTarget::Score | ScoreTarget::Mysekai => {
                if matches!(ctx.target, ScoreTarget::Score) && !ctx.has_event() {
                    let power = pool.power_max(card) as f64;
                    let skill = pool.skill_max(card) as f64;
                    return power * (256.0 + skill);
                }
                let bonus = card_proxy_bonus(pool, ctx, card, false);
                ctx.w_power * pool.power_max(card) as f64 + ctx.w_bonus * bonus as f64
            }
        },
    }
}

#[inline(always)]
fn score_event_solo_key(pool: &CardPool, ctx: &SearchContext, card: CardIdx) -> u64 {
    let power = pool.power_max(card) as u64;
    let skill = pool.skill_max(card) as u64;
    let bonus = card_proxy_bonus(pool, ctx, card, false) as u64;
    power * (256 + skill) * (100 + bonus)
}

fn sorted_final_chapter_members(
    pool: &CardPool,
    ctx: &SearchContext,
    leader: CardIdx,
) -> Vec<CardIdx> {
    let leader_char = pool.char_id(leader);
    let mut cards = pool
        .indices()
        .filter(|card| *card != leader && pool.char_id(*card) != leader_char)
        .map(|card| (score_event_solo_key(pool, ctx, card), card))
        .collect::<Vec<_>>();
    cards.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.raw().cmp(&right.1.raw()))
    });
    cards.into_iter().map(|(_, card)| card).collect()
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
