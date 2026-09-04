//! 建池候选卡过滤与按角色裁剪。
//!
//! 两层判定：
//! - `prepared_*`：seed 阶段（`PreparedCardSeed`，尚未计算 power/skill）的快速过滤；
//! - 其余函数：`CardIntermediate` 阶段的 EP 预过滤、按角色 top-K 裁剪与硬约束保留。

use super::build::PreparedCardSeed;
use super::event_bonus::EventContext;
use super::gather::CardIntermediate;
use super::types::{self, attr_to_pool_index, parse_attr_code, parse_unit_code};

pub(super) fn prepared_ep_prefilter_keep(
    card: &PreparedCardSeed<'_>,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    if is_world_bloom || is_final_chapter {
        return card.master.card_rarity_type >= 3 || card.event_bonus.total_x10() > 0;
    }
    if card.master.card_rarity_type >= 4 {
        return true;
    }
    card.event_bonus.total_x10() > 0 && card.has_char_bonus && card.has_attr_bonus
}

pub(super) fn prepared_ep_prefilter_keep_with_params(
    card: &PreparedCardSeed<'_>,
    params: &types::BuildParams,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    if params.fixed_cards.contains(&card.master.id)
        || params.fixed_characters.contains(&card.master.character_id)
    {
        return true;
    }
    prepared_ep_prefilter_keep(card, is_world_bloom, is_final_chapter)
}

pub(super) fn prepared_keep_card(card: &PreparedCardSeed<'_>, params: &types::BuildParams) -> bool {
    let is_fixed_card = params.fixed_cards.contains(&card.master.id);
    if params.excluded_cards.contains(&card.master.id) {
        return false;
    }
    if !is_fixed_card {
        if let Some(unit) = params
            .unit_filter
            .as_deref()
            .and_then(parse_unit_code)
            .and_then(types::unit_to_pool_index)
            && card.unit_mask & (1u8 << unit) == 0
        {
            return false;
        }
        if let Some(attr) = params
            .attr_filter
            .as_deref()
            .and_then(parse_attr_code)
            .and_then(attr_to_pool_index)
            && card.attr != attr
        {
            return false;
        }
        if params.filter_other_unit
            && let Some(unit) = params
                .event_unit
                .as_deref()
                .and_then(parse_unit_code)
                .and_then(types::unit_to_pool_index)
            && card.unit_mask & (1u8 << unit) == 0
        {
            return false;
        }
    }
    params
        .challenge_live_character_id
        .is_none_or(|character_id| card.master.character_id == character_id)
}

pub(super) fn prepared_post_event_unit_filter(
    card: &PreparedCardSeed<'_>,
    params: &types::BuildParams,
    event_ctx: Option<&EventContext>,
) -> bool {
    if !params.filter_other_unit {
        return true;
    }
    let Some(unit) = event_ctx.and_then(|ctx| ctx.filter_unit) else {
        return true;
    };
    let Some(unit_index) = types::unit_to_pool_index(unit) else {
        return true;
    };
    let wanted = 1u8 << unit_index;
    let piapro = types::unit_to_pool_index(crate::types::Unit::Piapro)
        .map(|index| 1u8 << index)
        .unwrap_or(0);
    card.unit_mask & wanted != 0 || card.unit_mask == piapro
}

pub(super) fn ep_prefilter_keep(
    card: &CardIntermediate,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    // World Bloom / Final Chapter 跳过普通活动的双轴过滤，需要保留足够角色与属性覆盖。
    if is_world_bloom || is_final_chapter {
        return card.card_rarity_type >= 3 || card.event_bonus.total_x10() > 0;
    }
    // 4星+无条件保留
    if card.card_rarity_type >= 4 {
        return true;
    }
    // 3星：有 bonus + 双轴命中才保留
    if card.card_rarity_type == 3 {
        if card.event_bonus.total_x10() == 0 {
            return false;
        }
        return card.has_char_bonus && card.has_attr_bonus;
    }
    // 1-2星：有 bonus 且双轴同时命中才保留
    if card.event_bonus.total_x10() == 0 {
        return false;
    }
    card.has_char_bonus && card.has_attr_bonus
}

/// 逐角色把候选裁到 `per_char_keep` 张。
///
/// 与 [`trim_char_quota`] 的纯排序名额不同，这里保留规则是 (活动加成,
/// 综合力×技能) 的 **Pareto 前沿**：在这两者的任意权衡下有可能进最优解的
/// 卡，恰好就是前沿上的那一批——低加成高练度的「综合力专家」（WL turn-3
/// 的 336k cap / 异色差分场景）自然落在前沿末端，不需要单独的第二张榜单。
/// 幸存卡保持原相对顺序（掩码保留，不重排建池次序）。
///
/// 前沿不足额时按加成降序补齐；超额时沿前沿保留加成最高的一段。
/// 固定卡、固定角色与高加成卡（≥30%）豁免裁剪。
pub(super) fn per_character_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    per_char_keep: usize,
) {
    if cards.len() <= EP_PREFILTER_MIN_POOL || per_char_keep == 0 {
        return;
    }

    // 4 星无加成卡给虚拟加成 1：排在有加成卡之后，但优于低星。
    let bonus_key = |card: &CardIntermediate| -> u32 {
        let bonus = card.event_bonus.total_x10();
        if bonus == 0 && card.card_rarity_type >= 4 {
            1
        } else {
            bonus
        }
    };

    let mut keep = vec![false; cards.len()];
    let mut by_char: Vec<Vec<usize>> = vec![Vec::new(); 27];
    for (index, card) in cards.iter().enumerate() {
        if params.fixed_cards.contains(&card.game_card_id)
            || params
                .fixed_characters
                .contains(&(card.character_id as i32))
            || card.event_bonus.total_x10() >= 300
        {
            keep[index] = true;
            continue;
        }
        by_char[(card.character_id as usize).min(26)].push(index);
    }

    for candidates in &mut by_char {
        if candidates.len() <= per_char_keep {
            for &index in candidates.iter() {
                keep[index] = true;
            }
            continue;
        }
        // 加成降序、同加成时综合力降序；稳定排序让等价卡维持原始顺序。
        candidates.sort_by(|&left, &right| {
            bonus_key(&cards[right])
                .cmp(&bonus_key(&cards[left]))
                .then_with(|| {
                    cards[right]
                        .card_rarity_type
                        .cmp(&cards[left].card_rarity_type)
                })
                .then_with(|| power_skill_key(&cards[right]).cmp(&power_skill_key(&cards[left])))
        });

        // 加成已非增，因此「综合力严格高于此前所有卡」等价于不被任何卡支配。
        let mut kept = 0usize;
        let mut best_power = 0u64;
        let mut on_frontier = vec![false; candidates.len()];
        for (rank, &index) in candidates.iter().enumerate() {
            let power = power_skill_key(&cards[index]);
            if rank == 0 || power > best_power {
                best_power = best_power.max(power);
                on_frontier[rank] = true;
                keep[index] = true;
                kept += 1;
                if kept == per_char_keep {
                    break;
                }
            }
        }
        // 前沿不足额：按加成降序补齐剩余名额。
        for (rank, &index) in candidates.iter().enumerate() {
            if kept == per_char_keep {
                break;
            }
            if !on_frontier[rank] {
                keep[index] = true;
                kept += 1;
            }
        }
    }

    let mut index = 0usize;
    cards.retain(|_| {
        let result = keep[index];
        index += 1;
        result
    });
}

pub(super) fn ep_prefilter_keep_with_params(
    card: &CardIntermediate,
    params: &types::BuildParams,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    if (!params.fixed_cards.is_empty() && params.fixed_cards.contains(&card.game_card_id))
        || (!params.fixed_characters.is_empty()
            && params
                .fixed_characters
                .contains(&(card.character_id as i32)))
    {
        return true;
    }
    ep_prefilter_keep(card, is_world_bloom, is_final_chapter)
}

/// 综合力×技能排序键：score 类路径的「卡牌强度」统一度量。
fn power_skill_key(card: &CardIntermediate) -> u64 {
    card.power.power_max.max(0) as u64 * (256 + card.skill.skill_max as u64)
}

/// 每角色名额裁剪的唯一排序实现。
///
/// 历史上 general / target 两份函数各自复制了「排序 → 逐角色计数 → 保留」
/// 的机制，只有排序键与豁免规则不同；现在两者都走这里，由调用方注入：
/// - `order`：名额内的择优比较器（整体稳定排序，幸存卡保持排序序，
///   这决定了建池后的 CardIdx 次序，不得随意改动）；
/// - `exempt`：不占名额、无条件保留的卡（固定卡与固定角色由本函数统一豁免）。
fn trim_char_quota(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    order: impl FnMut(&CardIntermediate, &CardIntermediate) -> std::cmp::Ordering,
    exempt: impl Fn(&CardIntermediate) -> bool,
    per_char_keep: usize,
) {
    cards.sort_by(order);
    let mut counts = [0u8; 27];
    cards.retain(|card| {
        if params.fixed_cards.contains(&card.game_card_id)
            || params
                .fixed_characters
                .contains(&(card.character_id as i32))
            || exempt(card)
        {
            return true;
        }
        let ch = (card.character_id as usize).min(26);
        if (counts[ch] as usize) < per_char_keep {
            counts[ch] += 1;
            true
        } else {
            false
        }
    });
}

/// 通用活动的按角色裁剪（容量压力挡板）。
pub(super) fn general_per_character_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    per_char_keep: usize,
) {
    // 活动点数对加成是乘性敏感的：同角色内按纯战力保留会把高加成卡挤到
    // 名额之外直接裁掉（实测 jp1302 + 活动 215 裁掉 62.5% 活动卡，
    // Top-1 EP 落后 4~207，130 个马拉松/欢乐事件受影响）。因此有加成的卡
    // 一律豁免；战力序名额只用于 0 加成填充卡的择优，容量压力交给
    // dominance_trim（对 Top-1 无损）。
    trim_char_quota(
        cards,
        params,
        |a, b| power_skill_key(b).cmp(&power_skill_key(a)),
        |card| card.event_bonus.total_x10() > 0,
        per_char_keep,
    );
}

/// power/skill 目标的按角色裁剪：名额按目标键（minimize 时反向）择优。
pub(super) fn target_per_character_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
) {
    // minimize（最弱组卡，仅 Power）时保留每角色最弱的若干张，否则保留最强。
    let minimize = params.minimize && matches!(params.target, crate::types::ScoreTarget::Power);
    trim_char_quota(
        cards,
        params,
        |a, b| {
            let (a_key, b_key) = match params.target {
                crate::types::ScoreTarget::Power => {
                    (a.power.power_max.max(0) as u64, b.power.power_max.max(0) as u64)
                }
                crate::types::ScoreTarget::Skill => {
                    (a.skill.skill_max as u64, b.skill.skill_max as u64)
                }
                _ => (power_skill_key(a), power_skill_key(b)),
            };
            // minimize 时按 power 升序（最弱优先保留），否则降序。次级键同向翻转。
            let primary = if minimize {
                a_key.cmp(&b_key)
            } else {
                b_key.cmp(&a_key)
            };
            let rarity = if minimize {
                a.card_rarity_type.cmp(&b.card_rarity_type)
            } else {
                b.card_rarity_type.cmp(&a.card_rarity_type)
            };
            primary
                .then(rarity)
                .then_with(|| a.game_card_id.cmp(&b.game_card_id))
        },
        |_| false,
        GENERAL_PER_CHAR_KEEP,
    );
}

pub(super) fn keep_card(card: &CardIntermediate, params: &types::BuildParams) -> bool {
    let is_fixed_card = params.fixed_cards.contains(&card.game_card_id);
    if params.excluded_cards.contains(&card.game_card_id) {
        return false;
    }

    if !is_fixed_card {
        if let Some(unit) = params
            .unit_filter
            .as_deref()
            .and_then(parse_unit_code)
            .and_then(types::unit_to_pool_index)
        {
            let wanted = 1u8 << unit;
            if card.unit_mask_raw & wanted == 0 {
                return false;
            }
        }

        if let Some(attr) = params
            .attr_filter
            .as_deref()
            .and_then(parse_attr_code)
            .and_then(attr_to_pool_index)
            && card.attr != attr
        {
            return false;
        }

        if params.filter_other_unit
            && let Some(unit) = params
                .event_unit
                .as_deref()
                .and_then(parse_unit_code)
                .and_then(types::unit_to_pool_index)
        {
            let wanted = 1u8 << unit;
            if card.unit_mask_raw & wanted == 0 {
                return false;
            }
        }
    }

    if let Some(challenge_char_id) = params.challenge_live_character_id
        && card.character_id != challenge_char_id as u8
    {
        return false;
    }

    true
}

pub(super) const EP_PREFILTER_MIN_POOL: usize = 50;
pub(super) const PER_CHAR_KEEP: usize = 6;
/// WL 章节 / 终章的单角色名额：336k cap 与异色差分让低加成高练度卡也可能进最优解，
/// 名额比常规活动宽，具体保留哪几张交给 Pareto 前沿决定。
pub(super) const WORLD_BLOOM_PER_CHAR_KEEP: usize = 14;
pub(super) const FINAL_CHAPTER_PER_CHAR_KEEP: usize = 16;
pub(super) const GENERAL_TRIM_THRESHOLD: usize = 400;
pub(super) const GENERAL_PER_CHAR_KEEP: usize = 10;
pub(super) const CHALLENGE_ALL_PER_CHAR_KEEP: usize = 19;
