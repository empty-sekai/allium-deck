use std::collections::BTreeSet;
use std::time::Instant;

use allium_deck::pool::CardIdx;
use allium_deck::search::{leaf_evaluate, search_instrumented, DeckResult, SearchStats};
use allium_deck::types::DECK_SIZE;

use crate::testdata_adapter::output_compare::{compare, CompareCategory};

use super::framework::{PreparedCase, VerificationKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseStatus {
    Passed,
    Better,
    Bug,
}

#[derive(Clone, Debug)]
pub struct CaseRun {
    pub case_name: String,
    pub status: CaseStatus,
    pub detail: String,
    pub elapsed_ms: f64,
    pub pool_size: usize,
    pub stats: SearchStats,
}

pub fn run_case(case: &PreparedCase) -> CaseRun {
    let t0 = Instant::now();
    let (results, stats) = search_instrumented(&case.pool, &case.ctx, &case.search_params);
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let mut status = CaseStatus::Passed;
    let mut detail = match &case.verify {
        VerificationKind::Reference(expected) => {
            let result = compare(
                case.ctx.target,
                &results,
                &case.pool,
                &case.ctx,
                expected,
                true,
                case.timeout_ms,
            );
            match result.category {
                CompareCategory::Pass | CompareCategory::Empty => CaseStatus::Passed,
                CompareCategory::Better => CaseStatus::Better,
                _ => CaseStatus::Bug,
            };
            status = match result.category {
                CompareCategory::Pass | CompareCategory::Empty => CaseStatus::Passed,
                CompareCategory::Better => CaseStatus::Better,
                _ => CaseStatus::Bug,
            };
            result.detail
        }
        VerificationKind::Soundness => match verify_soundness(case, &results) {
            Ok(()) => "soundness 断言通过".to_string(),
            Err(err) => {
                status = CaseStatus::Bug;
                err
            }
        },
    };

    if elapsed_ms >= case.perf_limit_ms {
        detail.push_str(&format!(
            "; 性能超限 {:.3}ms >= {:.0}ms",
            elapsed_ms, case.perf_limit_ms
        ));
    }

    CaseRun {
        case_name: case.case_name.clone(),
        status,
        detail,
        elapsed_ms,
        pool_size: case.pool.count(),
        stats,
    }
}

fn verify_soundness(case: &PreparedCase, results: &[DeckResult]) -> Result<(), String> {
    if results.is_empty() {
        return Err(format!("{}: 搜索结果为空", case.case_name));
    }
    for window in results.windows(2) {
        if window[0].score < window[1].score {
            return Err(format!("{}: 结果未按 score 降序", case.case_name));
        }
    }
    let mut deck_set = BTreeSet::new();
    for result in results {
        if !deck_set.insert(result.cards) {
            return Err(format!("{}: 结果存在重复 deck", case.case_name));
        }
        let mut chars = BTreeSet::new();
        for (slot, card) in result.cards.iter().enumerate() {
            if !chars.insert(case.pool.char_id(*card)) {
                return Err(format!("{}: deck 角色不唯一", case.case_name));
            }
            if let Some(game_id) = case.ctx.fixed_card_at(slot) {
                if case.pool.game_id(*card) != game_id {
                    return Err(format!("{}: fixed card 约束失效", case.case_name));
                }
            }
            if let Some(character_id) = case.ctx.fixed_character_at(slot) {
                if case.pool.char_id(*card) != character_id {
                    return Err(format!("{}: fixed character 约束失效", case.case_name));
                }
            }
        }
        if chars.len() != DECK_SIZE {
            return Err(format!("{}: deck 角色不唯一", case.case_name));
        }
    }

    if case.pool.count() <= 15 && !has_fixed_slot(case) {
        let brute = brute_force_best(case);
        if results[0].score != brute {
            return Err(format!(
                "{}: DFS 结果与暴力枚举不一致 actual={} brute={}",
                case.case_name, results[0].score, brute
            ));
        }
    }
    Ok(())
}

fn has_fixed_slot(case: &PreparedCase) -> bool {
    (0..DECK_SIZE).any(|slot| {
        case.ctx.fixed_card_at(slot).is_some() || case.ctx.fixed_character_at(slot).is_some()
    })
}

fn brute_force_best(case: &PreparedCase) -> u64 {
    let mut best = 0u64;
    let first = case.pool.card_idx(0).unwrap_or_else(|| panic!("空 pool"));
    let mut deck = [first; DECK_SIZE];
    brute_recurse(case, 0, 0, 0, &mut deck, &mut best);
    best
}

fn brute_recurse(
    case: &PreparedCase,
    depth: usize,
    start: usize,
    used_chars: u32,
    deck: &mut [CardIdx; DECK_SIZE],
    best: &mut u64,
) {
    if depth == DECK_SIZE {
        let score = leaf_evaluate(&case.pool, &case.ctx, deck);
        *best = (*best).max(score);
        return;
    }

    let mut dense = start;
    while dense < case.pool.count() {
        let card = case
            .pool
            .card_idx(dense as u16)
            .unwrap_or_else(|| panic!("dense idx 越界: {dense}"));
        dense += 1;
        let character_id = case.pool.char_id(card);
        if used_chars & (1u32 << character_id) != 0 {
            continue;
        }
        if let Some(game_id) = case.ctx.fixed_card_at(depth) {
            if case.pool.game_id(card) != game_id {
                continue;
            }
        }
        if let Some(fixed_character_id) = case.ctx.fixed_character_at(depth) {
            if character_id != fixed_character_id {
                continue;
            }
        }
        deck[depth] = card;
        brute_recurse(
            case,
            depth + 1,
            dense,
            used_chars | (1u32 << character_id),
            deck,
            best,
        );
    }
}
