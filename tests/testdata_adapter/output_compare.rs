use std::collections::BTreeSet;

use allium_deck::pool::{CardIdx, CardPool, DiffSkill, RefSkill, SkillSlot, UnitCountSkill};
use allium_deck::search::{DeckResult, SearchContext, leaf_evaluate};
use allium_deck::types::{DECK_SIZE, ScoreTarget, SkillReferenceStrategy};
use serde::Serialize;

use super::legacy_types::{LegacyOutput, LegacyOutputCard};

/// 回归比对分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CompareCategory {
    Pass,
    Better,
    Timeout,
    Empty,
    Bug,
}

/// 单 case 比对结果。
#[derive(Debug, Clone, Serialize)]
pub struct CompareResult {
    pub passed: bool,
    pub category: CompareCategory,
    pub detail: String,
}

/// 单 case 写入 summary 的记录。
#[derive(Debug, Clone, Serialize)]
pub struct CaseSummary {
    pub name: String,
    pub combo: String,
    pub category: CompareCategory,
    pub passed: bool,
    pub detail: String,
}

/// 针对不同 target 比对搜索结果与 C++ reference_output。
pub fn compare(
    target: ScoreTarget,
    result: &[DeckResult],
    pool: &CardPool,
    ctx: &SearchContext,
    expected: &[LegacyOutput],
    verify_output: bool,
    timeout_ms: u64,
) -> CompareResult {
    if !verify_output {
        return CompareResult {
            passed: true,
            category: CompareCategory::Pass,
            detail: "manifest 标记 verify_output=false，已跳过输出比对".to_string(),
        };
    }

    if expected.is_empty() && result.is_empty() {
        return CompareResult {
            passed: true,
            category: CompareCategory::Empty,
            detail: "allium 与 C++ reference_output 均为空结果".to_string(),
        };
    }

    if expected.is_empty() {
        return CompareResult {
            passed: false,
            category: CompareCategory::Bug,
            detail: "C++ reference_output 为空，但 allium 返回了结果".to_string(),
        };
    }

    let Some(actual) = result.first() else {
        return CompareResult {
            passed: timeout_ms <= 5_000,
            category: classify_failure(timeout_ms, false),
            detail: "搜索结果为空，但 reference_output 非空".to_string(),
        };
    };

    let expected_top = &expected[0];
    let card_set_matches = compare_card_set(actual.cards, pool, &expected_top.cards);

    match target {
        ScoreTarget::Power => {
            let actual_power = actual.score as i64;
            let expected_power = expected_top.total_power as i64;
            let mut result = compare_at_least_i64(
                actual_power,
                expected_power,
                timeout_ms,
                card_set_matches,
                "total_power",
            );
            if !result.passed || matches!(result.category, CompareCategory::Better) {
                result.detail.push_str(&format!(
                    "; actual_cards={}; expected_cards={}; expected_deck_actual={}; expected_card_power={}",
                    format_actual_cards(actual.cards, pool),
                    format_expected_cards(&expected_top.cards, pool),
                    evaluate_expected_cards(&expected_top.cards, pool, ctx),
                    format_expected_card_power(&expected_top.cards, pool),
                ));
            }
            result
        }
        ScoreTarget::Score => {
            let actual_event_point = (actual.score >> 32) as i64;
            let actual_live_score = (actual.score & 0xffff_ffff) as i64;
            let expected_event_point = expected_top.score as i64;
            let expected_live_score = expected_top.live_score as i64;
            if ctx.has_event() {
                let mut result = compare_at_least_i64(
                    actual_event_point,
                    expected_event_point,
                    timeout_ms,
                    card_set_matches,
                    "event_point",
                );
                if !result.passed || matches!(result.category, CompareCategory::Better) {
                    result.detail.push_str(&format!(
                        "; actual_cards={}; expected_cards={}; expected_deck_actual={}",
                        format_actual_cards(actual.cards, pool),
                        format_expected_cards(&expected_top.cards, pool),
                        evaluate_expected_cards(&expected_top.cards, pool, ctx),
                    ));
                }
                result
            } else {
                let mut result = compare_at_least_i64(
                    actual_live_score,
                    expected_live_score,
                    timeout_ms,
                    card_set_matches,
                    "live_score",
                );
                if !result.passed || matches!(result.category, CompareCategory::Better) {
                    result.detail.push_str(&format!(
                        "; actual_cards={}; expected_cards={}",
                        format_actual_cards(actual.cards, pool),
                        format_expected_cards(&expected_top.cards, pool),
                    ));
                }
                result
            }
        }
        ScoreTarget::Skill => {
            let actual_skill = precise_multi_live_score_up(actual.cards, pool, ctx);
            compare_at_least_f64(
                actual_skill,
                expected_top.multi_live_score_up,
                timeout_ms,
                card_set_matches,
                "multi_live_score_up",
            )
        }
        ScoreTarget::Bonus | ScoreTarget::Mysekai => CompareResult {
            passed: false,
            category: CompareCategory::Bug,
            detail: "manifest 不应出现 Bonus/Mysekai target".to_string(),
        },
    }
}

fn compare_at_least_i64(
    actual: i64,
    expected: i64,
    timeout_ms: u64,
    card_set_matches: bool,
    field: &str,
) -> CompareResult {
    if actual >= expected {
        let category = if actual > expected + 1 {
            CompareCategory::Better
        } else {
            CompareCategory::Pass
        };
        let card_note = if card_set_matches {
            "卡组集合匹配"
        } else {
            "卡组集合不同"
        };
        CompareResult {
            passed: true,
            category,
            detail: format!("{field}: actual={actual} expected={expected}; {card_note}"),
        }
    } else {
        CompareResult {
            passed: timeout_ms <= 5_000,
            category: classify_failure(timeout_ms, card_set_matches),
            detail: format!("{field} 严格更差: actual={actual} expected={expected}"),
        }
    }
}

fn compare_at_least_f64(
    actual: f64,
    expected: f64,
    timeout_ms: u64,
    card_set_matches: bool,
    field: &str,
) -> CompareResult {
    if actual + 1e-9 >= expected {
        let category = if actual > expected + 1.0 {
            CompareCategory::Better
        } else {
            CompareCategory::Pass
        };
        let card_note = if card_set_matches {
            "卡组集合匹配"
        } else {
            "卡组集合不同"
        };
        CompareResult {
            passed: true,
            category,
            detail: format!("{field}: actual={actual:.4} expected={expected:.4}; {card_note}"),
        }
    } else {
        CompareResult {
            passed: timeout_ms <= 5_000,
            category: classify_failure(timeout_ms, card_set_matches),
            detail: format!("{field} 严格更差: actual={actual:.4} expected={expected:.4}"),
        }
    }
}

fn classify_failure(timeout_ms: u64, card_set_matches: bool) -> CompareCategory {
    if timeout_ms <= 5_000 {
        CompareCategory::Timeout
    } else {
        let _ = card_set_matches;
        CompareCategory::Bug
    }
}

fn compare_card_set(actual: [CardIdx; 5], pool: &CardPool, expected: &[LegacyOutputCard]) -> bool {
    let actual_set = actual
        .into_iter()
        .map(|card| pool.game_id(card) as i32)
        .collect::<BTreeSet<_>>();
    let expected_set = expected
        .iter()
        .map(|card| card.card_id)
        .collect::<BTreeSet<_>>();
    actual_set == expected_set
}

fn format_actual_cards(actual: [CardIdx; 5], pool: &CardPool) -> String {
    actual
        .into_iter()
        .map(|card| {
            let bonus = pool.event_bonus_exact(card);
            format!(
                "{}({}+{})",
                pool.game_id(card),
                bonus.base_ceil(),
                bonus.limited_ceil()
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_expected_cards(expected: &[LegacyOutputCard], pool: &CardPool) -> String {
    expected
        .iter()
        .map(
            |card| match find_card_by_game_id(pool, card.card_id as u16) {
                Some(idx) => {
                    let bonus = pool.event_bonus_exact(idx);
                    format!(
                        "{}({}+{})",
                        card.card_id,
                        bonus.base_ceil(),
                        bonus.limited_ceil()
                    )
                }
                None => format!("{}(missing)", card.card_id),
            },
        )
        .collect::<Vec<_>>()
        .join(",")
}

fn find_card_by_game_id(pool: &CardPool, game_id: u16) -> Option<CardIdx> {
    pool.indices().find(|&card| pool.game_id(card) == game_id)
}

fn evaluate_expected_cards(
    expected: &[LegacyOutputCard],
    pool: &CardPool,
    ctx: &SearchContext,
) -> String {
    if expected.len() != DECK_SIZE {
        return "n/a".to_string();
    }
    let mut indices = Vec::with_capacity(DECK_SIZE);
    for card in expected {
        let Some(idx) = find_card_by_game_id(pool, card.card_id as u16) else {
            return "missing".to_string();
        };
        indices.push(idx);
    }
    let Ok(deck) = indices.try_into() else {
        return "n/a".to_string();
    };
    leaf_evaluate(pool, ctx, &deck).to_string()
}

fn format_expected_card_power(expected: &[LegacyOutputCard], pool: &CardPool) -> String {
    expected
        .iter()
        .filter_map(|card| {
            let idx = find_card_by_game_id(pool, card.card_id as u16)?;
            Some(format!("{}:{}", card.card_id, pool.power_max(idx) * 4))
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn precise_support_bonus(ctx: &SearchContext, deck_game_ids: &[u16; 5]) -> u32 {
    let mut total = 0u32;
    let mut picked = 0u8;
    for &(game_id, bonus) in &ctx.support_deck.cards {
        if picked >= ctx.support_deck.count {
            break;
        }
        if deck_game_ids.contains(&game_id) {
            continue;
        }
        total += bonus as u32;
        picked += 1;
    }
    total
}

fn precise_multi_live_score_up(deck: [CardIdx; 5], pool: &CardPool, ctx: &SearchContext) -> f64 {
    let unit_counts = count_units(deck, pool);
    let mut skills = [0.0_f64; DECK_SIZE];
    for (index, &card) in deck.iter().enumerate() {
        let slot = pool.skill(card);
        skills[index] = match slot.skill_type {
            0 => slot.value as f64,
            1 => resolve_unit_count_score(slot, pool.special().unit_count(), &unit_counts) as f64,
            2 => resolve_diff_score(
                slot,
                pool.special().diff(),
                different_units(deck, pool, index),
            )
            .min(u32::from(pool.skill_max(card))) as f64,
            3 => {
                let base = pool.skill_min(card) as f64;
                match resolve_ref_score(slot, pool.special().ref_skills()) {
                    Some((rate, max)) if rate != 0 && max != 0 => {
                        // Share of each other member's static skill maximum, unrounded.
                        let mut shares = [0.0; DECK_SIZE - 1];
                        let mut len = 0usize;
                        for (other, &target) in deck.iter().enumerate() {
                            if other != index {
                                shares[len] = (f64::from(pool.skill_reference(target))
                                    * f64::from(rate)
                                    / 100.0)
                                    .min(f64::from(max));
                                len += 1;
                            }
                        }
                        base + choose_reference_score(&shares, len, ctx.skill_reference_strategy)
                    }
                    _ => base,
                }
            }
            _ => 0.0,
        };
    }

    let order = leader_order(deck, &skills, ctx.effective_best_skill_as_leader());
    let mut total = skills[order[0]];
    for &position in &order[1..] {
        total += skills[position] * 0.2;
    }
    total
}

fn choose_reference_score(
    scores: &[f64; DECK_SIZE - 1],
    len: usize,
    strategy: SkillReferenceStrategy,
) -> f64 {
    match strategy {
        SkillReferenceStrategy::Max => scores[..len].iter().copied().fold(0.0, f64::max),
        SkillReferenceStrategy::Min => scores[..len]
            .iter()
            .copied()
            .reduce(f64::min)
            .unwrap_or(0.0),
        SkillReferenceStrategy::Average => {
            if len == 0 {
                0.0
            } else {
                scores[..len].iter().sum::<f64>() / len as f64
            }
        }
    }
}

fn leader_order(deck: [CardIdx; 5], skills: &[f64; 5], best_as_leader: bool) -> [usize; 5] {
    let mut order = [0usize, 1, 2, 3, 4];
    if best_as_leader {
        let mut best = 0usize;
        let mut idx = 1usize;
        while idx < DECK_SIZE {
            if skills[idx] > skills[best]
                || (skills[idx] == skills[best] && deck[idx].raw() < deck[best].raw())
            {
                best = idx;
            }
            idx += 1;
        }
        order.swap(0, best);
    } else {
        let mut left = 2usize;
        while left < DECK_SIZE {
            let mut cursor = left;
            while cursor > 1 {
                if deck[order[cursor - 1]].raw() <= deck[order[cursor]].raw() {
                    break;
                }
                order.swap(cursor - 1, cursor);
                cursor -= 1;
            }
            left += 1;
        }
    }
    order
}

fn count_units(deck: [CardIdx; 5], pool: &CardPool) -> [u8; 6] {
    let mut counts = [0u8; 6];
    for card in deck {
        let mask = pool.unit_mask_raw(card);
        let mut unit = 0usize;
        while unit < 6 {
            if mask & (1u8 << unit) != 0 {
                counts[unit] += 1;
            }
            unit += 1;
        }
    }
    counts
}

fn resolve_unit_count_score(
    slot: SkillSlot,
    table: &[UnitCountSkill],
    unit_counts: &[u8; 6],
) -> u32 {
    let Some(entry) = table.get(slot.value.saturating_sub(1) as usize) else {
        return 0;
    };
    let member_count = unit_counts[entry.unit as usize].clamp(1, 5) as usize;
    entry.score_up[member_count - 1] as u32
}

/// Distinct units of the other members that differ from the member's own unit.
/// A Virtual Singer card with a support unit counts as that unit.
fn different_units(deck: [CardIdx; 5], pool: &CardPool, index: usize) -> u32 {
    const PIAPRO: u8 = 1 << 5;
    let unit_of = |card: CardIdx| {
        let mask = pool.unit_mask_raw(card);
        let non_piapro = mask & !PIAPRO;
        if non_piapro == 0 {
            mask
        } else {
            1 << non_piapro.trailing_zeros()
        }
    };
    let own = unit_of(deck[index]);
    let mut others = 0u8;
    for (position, &card) in deck.iter().enumerate() {
        let unit = unit_of(card);
        if position != index && unit != own {
            others |= unit;
        }
    }
    others.count_ones()
}

fn resolve_diff_score(slot: SkillSlot, table: &[DiffSkill], units: u32) -> u32 {
    let Some(entry) = table.get(slot.value.saturating_sub(1) as usize) else {
        return 0;
    };
    u32::from(entry.base)
        + u32::from(entry.increment) * units.min(u32::from(DiffSkill::MAX_COUNTED_UNITS))
}

fn resolve_ref_score(slot: SkillSlot, table: &[RefSkill]) -> Option<(u8, u8)> {
    table
        .get(slot.value.saturating_sub(1) as usize)
        .map(|entry| (entry.rate, entry.max))
}
