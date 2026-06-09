use crate::pool::{DiffSkill, RefSkill, SkillSlot, UnitCountSkill};
use crate::types::SkillInfo;

use super::index::PoolIndexes;
use super::types::{parse_unit_code, unit_to_pool_index, GameData, MasterCard, UserCard};

/// 技能预计算结果。
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct SkillResult {
    /// PoolBuilder 主表槽位。
    pub slot: SkillSlot,
    /// 组分侧表项。
    pub unit_count: Option<UnitCountSkill>,
    /// 异团侧表项。
    pub diff: Option<DiffSkill>,
    /// 吸分侧表项。
    pub ref_skill: Option<RefSkill>,
    /// 技能下界。
    pub skill_min: u8,
    /// 技能上界。
    pub skill_max: u8,
    /// 全精度技能信息。
    pub full: SkillInfo,
}

/// 卡牌技能状态选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkillState {
    /// 使用卡牌原始技能。
    BeforeTraining,
    /// 使用特训后技能；没有特训后技能时回落到原始技能。
    AfterTraining,
}

fn clamp_score(value: i32, limit: Option<u32>) -> u8 {
    let value = value.max(0) as u32;
    let capped = match limit {
        Some(limit) => value.min(limit),
        None => value,
    };
    capped.min(u8::MAX as u32) as u8
}

/// 构建单卡技能预计算结果。
///
/// `_game` 保留以兼容既有调用方；技能/效果查表已走 `idx` 索引（P3）。
pub(crate) fn build_skill(
    user_card: &UserCard,
    master: &MasterCard,
    _game: &GameData<'_>,
    idx: &PoolIndexes<'_>,
    character_rank: i32,
    skill_limit: Option<u32>,
    skill_state: SkillState,
) -> SkillResult {
    let skill_id = match skill_state {
        SkillState::AfterTraining => master.special_training_skill_id.unwrap_or(master.skill_id),
        SkillState::BeforeTraining => master.skill_id,
    };
    let skill = idx.skill(skill_id, user_card.skill_level);
    let Some(skill) = skill else {
        return SkillResult::default();
    };

    let effects = idx.skill_effects(skill_id, skill.level).iter().copied();

    let mut base_score_up = 0i32;
    let mut life_recovery = 0i32;
    let mut character_rank_bonus = 0i32;
    let mut unit_count_unit = None;
    let mut unit_count_values = [0u8; 5];
    let mut diff = None;
    let mut ref_rate = 0i32;
    let mut ref_max = 0i32;

    for effect in effects {
        match effect.effect_type.trim().to_ascii_lowercase().as_str() {
            "score_up" | "score_up_condition_life" | "score_up_keep" => {
                base_score_up = base_score_up.max(effect.value);
            }
            "life_recovery" => life_recovery += effect.value,
            "score_up_character_rank" => {
                if effect
                    .activate_character_rank
                    .is_some_and(|rank| rank <= character_rank)
                {
                    character_rank_bonus = character_rank_bonus.max(effect.value);
                }
            }
            "score_up_unit_count" => {
                unit_count_unit = effect.unit.as_deref().and_then(parse_unit_code);
                if let Some(count) = effect.unit_member_count {
                    if (1..=5).contains(&count) {
                        unit_count_values[(count - 1) as usize] =
                            clamp_score(effect.value, skill_limit);
                    }
                }
            }
            "score_up_diff" => {
                diff = Some(DiffSkill {
                    base: clamp_score(effect.value, skill_limit),
                    increment: clamp_score(effect.additional_value.unwrap_or(0), None),
                });
            }
            "score_up_reference" => {
                ref_rate = effect.value;
                ref_max = effect.additional_value.unwrap_or(0);
            }
            _ => {}
        }
    }

    base_score_up += character_rank_bonus;
    let base_clamped = clamp_score(base_score_up, skill_limit);
    let mut result = SkillResult {
        full: SkillInfo {
            skill_id,
            is_after_training: matches!(skill_state, SkillState::AfterTraining),
            base_score_up: base_clamped as f64,
            life_recovery: life_recovery.max(0) as f64,
            has_ref: ref_rate > 0 && ref_max > 0,
            ref_rate: ref_rate.max(0) as f64,
            ref_max: 0.0,
        },
        skill_min: base_clamped,
        skill_max: base_clamped,
        ..SkillResult::default()
    };

    if let Some(unit) = unit_count_unit.and_then(unit_to_pool_index) {
        for value in &mut unit_count_values {
            if *value == 0 {
                *value = base_clamped;
            }
        }
        result.slot = SkillSlot {
            skill_type: 1,
            value: 0,
        };
        result.unit_count = Some(UnitCountSkill {
            unit,
            score_up: unit_count_values,
        });
        result.skill_min = *unit_count_values.iter().min().unwrap_or(&0);
        result.skill_max = *unit_count_values.iter().max().unwrap_or(&0);
        result.full.base_score_up = result.skill_max as f64;
        return result;
    }

    if let Some(diff) = diff {
        result.slot = SkillSlot {
            skill_type: 2,
            value: 0,
        };
        result.diff = Some(diff);
        result.skill_min = diff.base;
        result.skill_max = clamp_score(diff.base as i32 + diff.increment as i32 * 2, skill_limit);
        result.full.base_score_up = result.skill_max as f64;
        return result;
    }

    if ref_rate > 0 && ref_max > 0 {
        let ref_max_clamped = match skill_limit {
            Some(limit) => {
                let headroom = limit.saturating_sub(base_clamped as u32);
                (ref_max.max(0) as u32).min(headroom).min(u8::MAX as u32) as u8
            }
            None => ref_max.max(0).min(u8::MAX as i32) as u8,
        };
        result.slot = SkillSlot {
            skill_type: 3,
            value: 0,
        };
        result.ref_skill = Some(RefSkill {
            rate: ref_rate.max(0).min(u8::MAX as i32) as u8,
            max: ref_max_clamped,
        });
        result.skill_min = base_clamped;
        result.skill_max = base_clamped.saturating_add(ref_max_clamped);
        result.full.ref_max = ref_max_clamped as f64;
        return result;
    }

    result.slot = SkillSlot {
        skill_type: 0,
        value: base_clamped,
    };
    result
}
