use crate::pool::{CardPool, EventBonusHot, PoolBuilder, SkillSlot};
use crate::types::{DefaultImage, LiveType, PowerDetail, ScoreTarget, SkillInfo};

use super::power::PowerResult;
use super::skill::{is_bfes_skill_pair, SkillResult};

/// gather 前的卡级中间态。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CardIntermediate {
    /// 原始 game card id。
    pub game_card_id: i32,
    /// 稀有度类型。
    pub card_rarity_type: i32,
    /// 角色 ID。
    pub character_id: u8,
    /// 属性 ID。
    pub attr: u8,
    /// 原始 unit mask。
    pub unit_mask_raw: u8,
    /// 默认立绘。
    pub default_image: DefaultImage,
    /// master rank。
    pub master_rank: i32,
    /// 技能等级。
    pub skill_level: i32,
    /// power 全精度结果。
    pub power: PowerResult,
    /// skill 全精度结果。
    pub skill: SkillResult,
    /// 热路径活动 bonus。
    pub event_bonus: EventBonusHot,
    /// 是否命中角色 bonus 轴。
    pub has_char_bonus: bool,
    /// 是否命中属性 bonus 轴。
    pub has_attr_bonus: bool,
    /// 终章 leader honor bonus。
    pub leader_honor_bonus: u16,
    /// 终章 leader limit bonus。
    pub leader_limit_bonus: u16,
    /// Score/Mysekai 排序键。
    pub ep_sort_key: i64,
}

/// 排序后保留的全精度卡信息。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FullPrecisionCard {
    /// 原始 game card id。
    pub game_card_id: u16,
    /// 稀有度类型。
    pub card_rarity_type: i32,
    /// 角色 ID。
    pub character_id: u8,
    /// 属性 ID。
    pub attr: u8,
    /// 原始 unit mask。
    pub unit_mask_raw: u8,
    /// 默认立绘。
    pub default_image: DefaultImage,
    /// master rank。
    pub master_rank: i32,
    /// 技能等级。
    pub skill_level: i32,
    /// 全精度 power 结果。
    pub power: [[PowerDetail; 4]; 6],
    /// 全精度 skill 结果。
    pub skill: SkillInfo,
    /// 热路径活动 bonus。
    pub event_bonus: EventBonusHot,
    /// 精确 power 下界。
    pub power_min_exact: i32,
    /// 精确 power 上界。
    pub power_max_exact: i32,
    /// 精确 skill 下界。
    pub skill_min_exact: u8,
    /// 精确 skill 上界。
    pub skill_max_exact: u8,
    /// leader honor bonus。
    pub leader_honor_bonus: u16,
    /// leader limit bonus。
    pub leader_limit_bonus: u16,
}

fn encode_power(card: &CardIntermediate) -> ([u16; 8], u32) {
    let mut units = Vec::new();
    for bit in 0..6u8 {
        if card.unit_mask_raw & (1u8 << bit) != 0 {
            units.push(bit as usize);
        }
    }
    let primary = units.first().copied().unwrap_or(0);
    let secondary = units.get(1).copied().unwrap_or(primary);
    let mut values = [0u16; 8];
    let mut packed = 0u32;

    for member_key in 0..4usize {
        let primary_value = card.power.resolved[primary][member_key].total.max(0) as u32;
        values[member_key] = primary_value as u16;
        packed |= ((primary_value >> 16) & 0b11) << (member_key * 2);

        let secondary_value = card.power.resolved[secondary][member_key].total.max(0) as u32;
        let slot = 4 + member_key;
        values[slot] = secondary_value as u16;
        packed |= ((secondary_value >> 16) & 0b11) << (slot * 2);
    }

    for unit in units {
        let slot = if unit == secondary { 1u32 } else { 0u32 };
        packed |= slot << (16 + unit);
    }

    (values, packed)
}

fn compare_cards(
    left: &CardIntermediate,
    right: &CardIntermediate,
    target: ScoreTarget,
    has_event: bool,
    effective_live_type: LiveType,
) -> std::cmp::Ordering {
    match target {
        ScoreTarget::Skill => right
            .skill
            .skill_max
            .cmp(&left.skill.skill_max)
            .then_with(|| right.skill.skill_min.cmp(&left.skill.skill_min))
            .then_with(|| right.game_card_id.cmp(&left.game_card_id)),
        ScoreTarget::Power => right
            .power
            .power_max
            .cmp(&left.power.power_max)
            .then_with(|| right.power.power_min.cmp(&left.power.power_min))
            .then_with(|| right.game_card_id.cmp(&left.game_card_id)),
        ScoreTarget::Score
            if has_event && matches!(effective_live_type, LiveType::Solo | LiveType::Auto) =>
        {
            let left_bonus = left.event_bonus.total_x2();
            let right_bonus = right.event_bonus.total_x2();
            let left_key = score_noevent_sort_key(left);
            let right_key = score_noevent_sort_key(right);
            right_bonus
                .cmp(&left_bonus)
                .then_with(|| right_key.cmp(&left_key))
                .then_with(|| right.power.power_max.cmp(&left.power.power_max))
                .then_with(|| right.skill.skill_max.cmp(&left.skill.skill_max))
                .then_with(|| right.game_card_id.cmp(&left.game_card_id))
        }
        // 有 event: 按 bonus 降序（ep 乘积结构下 bonus 敏感度更高）
        // 无 event: bonus 不参与 ep → 回退 power 排序
        _ if has_event => {
            let left_bonus = left.event_bonus.total_x2();
            let right_bonus = right.event_bonus.total_x2();
            let left_key = score_noevent_sort_key(left);
            let right_key = score_noevent_sort_key(right);
            right_bonus
                .cmp(&left_bonus)
                .then_with(|| right_key.cmp(&left_key))
                .then_with(|| right.power.power_max.cmp(&left.power.power_max))
                .then_with(|| right.skill.skill_max.cmp(&left.skill.skill_max))
                .then_with(|| right.game_card_id.cmp(&left.game_card_id))
        }
        ScoreTarget::Score => {
            let left_key = score_noevent_sort_key(left);
            let right_key = score_noevent_sort_key(right);
            right_key
                .cmp(&left_key)
                .then_with(|| right.power.power_max.cmp(&left.power.power_max))
                .then_with(|| right.skill.skill_max.cmp(&left.skill.skill_max))
                .then_with(|| right.power.power_min.cmp(&left.power.power_min))
                .then_with(|| right.skill.skill_min.cmp(&left.skill.skill_min))
                .then_with(|| right.game_card_id.cmp(&left.game_card_id))
        }
        _ => right
            .power
            .power_max
            .cmp(&left.power.power_max)
            .then_with(|| right.power.power_min.cmp(&left.power.power_min))
            .then_with(|| right.game_card_id.cmp(&left.game_card_id)),
    }
}

fn fixed_slot_rank(
    card: &CardIntermediate,
    fixed_card_ids: &[u16],
    fixed_character_ids: &[u8],
) -> Option<usize> {
    let game_card_id = card.game_card_id.max(0).min(u16::MAX as i32) as u16;
    if let Some(pos) = fixed_card_ids.iter().position(|id| *id == game_card_id) {
        return Some(pos);
    }
    fixed_character_ids
        .iter()
        .position(|id| *id == card.character_id)
        .map(|pos| fixed_card_ids.len() + pos)
}

fn compare_cards_with_fixed_slots(
    left: &CardIntermediate,
    right: &CardIntermediate,
    target: ScoreTarget,
    has_event: bool,
    effective_live_type: LiveType,
    fixed_card_ids: &[u16],
    fixed_character_ids: &[u8],
) -> std::cmp::Ordering {
    match (
        fixed_slot_rank(left, fixed_card_ids, fixed_character_ids),
        fixed_slot_rank(right, fixed_card_ids, fixed_character_ids),
    ) {
        (Some(left_rank), Some(right_rank)) if left_rank != right_rank => {
            return left_rank.cmp(&right_rank);
        }
        (Some(_), Some(_))
            if left.game_card_id == right.game_card_id
                && is_bfes_skill_pair(&left.skill, &right.skill) =>
        {
            let left_trained = matches!(left.default_image, DefaultImage::SpecialTraining);
            let right_trained = matches!(right.default_image, DefaultImage::SpecialTraining);
            if left_trained != right_trained {
                return right_trained.cmp(&left_trained);
            }
        }
        (Some(_), None) => return std::cmp::Ordering::Less,
        (None, Some(_)) => return std::cmp::Ordering::Greater,
        _ => {}
    }
    compare_cards(left, right, target, has_event, effective_live_type)
}

#[inline(always)]
fn score_noevent_sort_key(card: &CardIntermediate) -> u64 {
    card.power.power_max.max(0) as u64 * (256 + card.skill.skill_max as u64)
}

/// 排序并灌装 `CardPool`。
pub(crate) fn sort_and_gather(
    mut cards: Vec<CardIntermediate>,
    target: ScoreTarget,
    has_event: bool,
    effective_live_type: LiveType,
    fixed_card_ids: &[u16],
    fixed_character_ids: &[u8],
) -> (CardPool, Vec<FullPrecisionCard>) {
    if fixed_card_ids.is_empty() && fixed_character_ids.is_empty() {
        cards.sort_by(|left, right| {
            compare_cards(left, right, target, has_event, effective_live_type)
        });
    } else {
        cards.sort_by(|left, right| {
            compare_cards_with_fixed_slots(
                left,
                right,
                target,
                has_event,
                effective_live_type,
                fixed_card_ids,
                fixed_character_ids,
            )
        });
    }

    let mut builder = PoolBuilder::new(cards.len() as u16);
    let mut unit_count_idx = 0u8;
    let mut diff_idx = 0u8;
    let mut ref_idx = 0u8;
    let mut full = Vec::with_capacity(cards.len());

    for (dense, card) in cards.into_iter().enumerate() {
        let dense = dense as u16;
        let (power_values, power_lut) = encode_power(&card);
        let mut slot = card.skill.slot;
        if let Some(skill) = card.skill.unit_count {
            unit_count_idx = unit_count_idx.saturating_add(1);
            builder.add_unit_count_skill(skill);
            slot = SkillSlot {
                skill_type: 1,
                value: unit_count_idx,
            };
        } else if let Some(skill) = card.skill.diff {
            diff_idx = diff_idx.saturating_add(1);
            builder.add_diff_skill(skill);
            slot = SkillSlot {
                skill_type: 2,
                value: diff_idx,
            };
        } else if let Some(skill) = card.skill.ref_skill {
            ref_idx = ref_idx.saturating_add(1);
            builder.add_ref_skill(skill);
            slot = SkillSlot {
                skill_type: 3,
                value: ref_idx,
            };
        }

        builder.set_power_values(dense, power_values);
        builder.set_power_lut(dense, power_lut);
        builder.set_power_max(dense, card.power.power_max.max(0) as u32);
        builder.set_skill(dense, slot);
        builder.set_skill_min(dense, card.skill.skill_min);
        builder.set_skill_max(dense, card.skill.skill_max);
        builder.set_event_bonus(dense, card.event_bonus);

        builder.set_char_id(dense, card.character_id);
        builder.set_attr(dense, card.attr);
        builder.set_unit_mask(dense, card.unit_mask_raw);
        builder.set_game_id(dense, card.game_card_id.max(0).min(u16::MAX as i32) as u16);
        builder.mark_char(card.character_id, dense);
        builder.mark_attr(card.attr, dense);
        for bit in 0..6u8 {
            if card.unit_mask_raw & (1u8 << bit) != 0 {
                builder.mark_unit(bit, dense);
            }
        }

        full.push(FullPrecisionCard {
            game_card_id: card.game_card_id.max(0).min(u16::MAX as i32) as u16,
            card_rarity_type: card.card_rarity_type,
            character_id: card.character_id,
            attr: card.attr,
            unit_mask_raw: card.unit_mask_raw,
            default_image: card.default_image,
            master_rank: card.master_rank,
            skill_level: card.skill_level,
            power: card.power.resolved,
            skill: card.skill.full,
            event_bonus: card.event_bonus,
            power_min_exact: card.power.power_min,
            power_max_exact: card.power.power_max,
            skill_min_exact: card.skill.skill_min,
            skill_max_exact: card.skill.skill_max,
            leader_honor_bonus: card.leader_honor_bonus,
            leader_limit_bonus: card.leader_limit_bonus,
        });
    }

    (builder.freeze(), full)
}
