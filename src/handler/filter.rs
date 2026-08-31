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
            && card.unit_mask & (1u8 << unit) == 0 {
                return false;
            }
        if let Some(attr) = params
            .attr_filter
            .as_deref()
            .and_then(parse_attr_code)
            .and_then(attr_to_pool_index)
            && card.attr != attr {
                return false;
            }
        if params.filter_other_unit
            && let Some(unit) = params
                .event_unit
                .as_deref()
                .and_then(parse_unit_code)
                .and_then(types::unit_to_pool_index)
                && card.unit_mask & (1u8 << unit) == 0 {
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

pub(super) fn per_character_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    per_char_keep: usize,
    power_specialist_keep: usize,
) {
    if cards.len() <= EP_PREFILTER_MIN_POOL {
        return;
    }
    debug_assert!(per_char_keep + power_specialist_keep <= FINAL_CHAPTER_PER_CHAR_KEEP);

    let compare = |a: &CardIntermediate, b: &CardIntermediate| {
        let a_bonus = a.event_bonus.total_x10();
        let b_bonus = b.event_bonus.total_x10();
        // 4星无bonus给虚拟bonus=1，排在有bonus卡之后但优于低星
        let a_effective_bonus = if a_bonus == 0 && a.card_rarity_type >= 4 {
            1
        } else {
            a_bonus
        };
        let b_effective_bonus = if b_bonus == 0 && b.card_rarity_type >= 4 {
            1
        } else {
            b_bonus
        };
        let a_key = a.power.power_max.max(0) as u64 * (256 + a.skill.skill_max as u64);
        let b_key = b.power.power_max.max(0) as u64 * (256 + b.skill.skill_max as u64);
        b_effective_bonus
            .cmp(&a_effective_bonus)
            .then_with(|| b.card_rarity_type.cmp(&a.card_rarity_type))
            .then_with(|| b_key.cmp(&a_key))
    };

    // Only the best K non-exempt cards per character are needed. Sorting the
    // entire vector moves large CardIntermediate values O(n log n) times.
    // Maintain 27 tiny, stable top-K index lists instead; equal keys stay in
    // their original order, matching stable-sort selection semantics.
    //
    // power_specialist_keep > 0 时额外维护一张「综合力榜」（综合力×(256+技能)
    // 降序）：低加成高练度卡在 WL turn-3（336k cap / 差分异色）场景同样能进
    // 最优解，纯加成榜会漏解。加成榜淘汰的卡
    // 立即获得一次综合力榜入榜尝试；两榜独立淘汰，角色保留上限 =
    // per_char_keep + power_specialist_keep（调用方保证合计 ≤
    // FINAL_CHAPTER_PER_CHAR_KEEP，27 角色总数 ≤ 512 mask 容量）。
    let mut selected = [[usize::MAX; FINAL_CHAPTER_PER_CHAR_KEEP]; 27];
    let mut counts = [0usize; 27];
    let mut p_selected = [[usize::MAX; 8]; 27];
    let mut p_counts = [0usize; 27];
    let mut keep = vec![false; cards.len()];
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
        let ch = (card.character_id as usize).min(26);
        let count = counts[ch];
        let mut insert = 0usize;
        while insert < count
            && compare(card, &cards[selected[ch][insert]]) != std::cmp::Ordering::Less
        {
            insert += 1;
        }
        let evicted = if insert < per_char_keep && count == per_char_keep {
            let evicted_index = selected[ch][per_char_keep - 1];
            keep[evicted_index] = false;
            Some(evicted_index)
        } else {
            None
        };
        if insert < per_char_keep {
            let new_count = (count + 1).min(per_char_keep);
            let mut slot = new_count - 1;
            while slot > insert {
                selected[ch][slot] = selected[ch][slot - 1];
                slot -= 1;
            }
            selected[ch][insert] = index;
            counts[ch] = new_count;
            keep[index] = true;
        }
        if let Some(evicted_index) = evicted {
            // 刚被加成榜挤出的卡立即尝试综合力榜（本趟不会再扫到它）。
            let evicted_card = &cards[evicted_index];
            let key = evicted_card.power.power_max.max(0) as u64
                * (256 + evicted_card.skill.skill_max as u64);
            let mut p_insert = 0usize;
            while p_insert < p_counts[ch] {
                let kept = &cards[p_selected[ch][p_insert]];
                let kept_key = kept.power.power_max.max(0) as u64
                    * (256 + kept.skill.skill_max as u64);
                if key > kept_key {
                    break;
                }
                p_insert += 1;
            }
            if p_insert < power_specialist_keep {
                let p_new_count = (p_counts[ch] + 1).min(power_specialist_keep);
                if p_counts[ch] == power_specialist_keep {
                    keep[p_selected[ch][power_specialist_keep - 1]] = false;
                }
                let mut p_slot = p_new_count - 1;
                while p_slot > p_insert {
                    p_selected[ch][p_slot] = p_selected[ch][p_slot - 1];
                    p_slot -= 1;
                }
                p_selected[ch][p_insert] = evicted_index;
                p_counts[ch] = p_new_count;
                keep[evicted_index] = true;
            }
        }
        // 综合力榜尝试：加成榜落选者（含刚被挤出的）都可入榜。
        if keep[index] {
            continue;
        }
        if power_specialist_keep > 0 {
            let key = card.power.power_max.max(0) as u64
                * (256 + card.skill.skill_max as u64);
            let mut p_insert = 0usize;
            while p_insert < p_counts[ch] {
                let kept = &cards[p_selected[ch][p_insert]];
                let kept_key = kept.power.power_max.max(0) as u64
                    * (256 + kept.skill.skill_max as u64);
                if key > kept_key {
                    break;
                }
                p_insert += 1;
            }
            if p_insert < power_specialist_keep {
                let p_new_count = (p_counts[ch] + 1).min(power_specialist_keep);
                if p_counts[ch] == power_specialist_keep {
                    keep[p_selected[ch][power_specialist_keep - 1]] = false;
                }
                let mut p_slot = p_new_count - 1;
                while p_slot > p_insert {
                    p_selected[ch][p_slot] = p_selected[ch][p_slot - 1];
                    p_slot -= 1;
                }
                p_selected[ch][p_insert] = index;
                p_counts[ch] = p_new_count;
                keep[index] = true;
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

pub(super) fn general_per_character_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    per_char_keep: usize,
) {
    cards.sort_by(|a, b| {
        let a_key = a.power.power_max.max(0) as u64 * (256 + a.skill.skill_max as u64);
        let b_key = b.power.power_max.max(0) as u64 * (256 + b.skill.skill_max as u64);
        b_key.cmp(&a_key)
    });
    let mut counts = [0u8; 27];
    cards.retain(|card| {
        if params.fixed_cards.contains(&card.game_card_id) {
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

pub(super) fn target_per_character_trim(cards: &mut Vec<CardIntermediate>, params: &types::BuildParams) {
    // minimize（最弱组卡，仅 Power）时保留每角色最弱的若干张，否则保留最强。
    let minimize = params.minimize && matches!(params.target, crate::types::ScoreTarget::Power);
    cards.sort_by(|a, b| {
        let (a_key, b_key) = match params.target {
            crate::types::ScoreTarget::Power => (
                a.power.power_max.max(0) as u64,
                b.power.power_max.max(0) as u64,
            ),
            crate::types::ScoreTarget::Skill => {
                (a.skill.skill_max as u64, b.skill.skill_max as u64)
            }
            _ => {
                let a_key = a.power.power_max.max(0) as u64 * (256 + a.skill.skill_max as u64);
                let b_key = b.power.power_max.max(0) as u64 * (256 + b.skill.skill_max as u64);
                (a_key, b_key)
            }
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
    });

    let mut counts = [0u8; 27];
    cards.retain(|card| {
        if params.fixed_cards.contains(&card.game_card_id)
            || params
                .fixed_characters
                .contains(&(card.character_id as i32))
        {
            return true;
        }
        let ch = (card.character_id as usize).min(26);
        if (counts[ch] as usize) < GENERAL_PER_CHAR_KEEP {
            counts[ch] += 1;
            true
        } else {
            false
        }
    });
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
            && card.attr != attr {
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
        && card.character_id != challenge_char_id as u8 {
            return false;
        }

    true
}

pub(super) const EP_PREFILTER_MIN_POOL: usize = 50;
pub(super) const PER_CHAR_KEEP: usize = 6;
pub(super) const FINAL_CHAPTER_PER_CHAR_KEEP: usize = 16;
pub(super) const GENERAL_TRIM_THRESHOLD: usize = 400;
pub(super) const GENERAL_PER_CHAR_KEEP: usize = 10;
pub(super) const CHALLENGE_ALL_PER_CHAR_KEEP: usize = 19;

