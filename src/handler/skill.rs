use crate::pool::{DiffSkill, RefSkill, SkillSlot, UnitCountSkill};
use crate::types::SkillInfo;

use super::index::{PoolIndexes, PreparedSkillEffectKind};
use super::types::{GameData, MasterCard, UserCard, unit_to_pool_index};
use super::{BuildError, capacity};

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

pub(crate) fn is_bfes_skill_pair(left: &SkillResult, right: &SkillResult) -> bool {
    left.full.skill_id != right.full.skill_id
        && (left.ref_skill.is_some()
            || right.ref_skill.is_some()
            || left.full.has_ref
            || right.full.has_ref
            || left.diff.is_some()
            || right.diff.is_some()
            || left.unit_count.is_some()
            || right.unit_count.is_some())
}

/// 卡牌技能状态选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkillState {
    /// 使用卡牌原始技能。
    BeforeTraining,
    /// 使用特训后技能；没有特训后技能时回落到原始技能。
    AfterTraining,
}

/// 构建单卡技能预计算结果。
///
/// `_game` 保留以兼容既有调用方；技能/效果查表已走 `idx` 索引（P3）。
pub(crate) fn build_skill(
    user_card: &UserCard,
    master: &MasterCard,
    _game: &GameData<'_>,
    idx: &PoolIndexes,
    character_rank: i32,
    skill_limit: Option<u32>,
    skill_state: SkillState,
) -> Result<SkillResult, BuildError> {
    let skill_id = match skill_state {
        SkillState::AfterTraining => master.special_training_skill_id.unwrap_or(master.skill_id),
        SkillState::BeforeTraining => master.skill_id,
    };
    let skill = idx.skill(skill_id, user_card.skill_level);
    let Some(skill) = skill else {
        return Ok(SkillResult::default());
    };

    let effects = idx.skill_effects(skill_id, skill.level).iter();

    let mut base_score_up = 0i64;
    let mut life_recovery = 0i64;
    let mut character_rank_bonus = 0i64;
    let mut unit_count_unit = None;
    let mut unit_count_values = [0u8; 5];
    let mut diff = None;
    let mut ref_rate = 0i32;
    let mut ref_max = 0i32;

    for effect in effects {
        match effect.kind {
            PreparedSkillEffectKind::ScoreUp => {
                base_score_up = base_score_up.max(i64::from(effect.value));
            }
            PreparedSkillEffectKind::LifeRecovery => life_recovery += i64::from(effect.value),
            PreparedSkillEffectKind::CharacterRank => {
                if effect
                    .activate_character_rank
                    .is_some_and(|rank| rank <= character_rank)
                {
                    character_rank_bonus = character_rank_bonus.max(i64::from(effect.value));
                }
            }
            PreparedSkillEffectKind::UnitCount => {
                unit_count_unit = effect.unit;
                if let Some(count) = effect.unit_member_count
                    && (1..=5).contains(&count)
                {
                    unit_count_values[(count - 1) as usize] =
                        capacity::score(i64::from(effect.value), skill_limit, "skill score")?;
                }
            }
            PreparedSkillEffectKind::Diff => {
                diff = Some(DiffSkill {
                    base: capacity::score(i64::from(effect.value), skill_limit, "skill score")?,
                    increment: capacity::score(
                        i64::from(effect.additional_value.unwrap_or(0)),
                        skill_limit,
                        "different-unit increment",
                    )?,
                });
            }
            PreparedSkillEffectKind::Reference => {
                ref_rate = effect.value;
                ref_max = effect.additional_value.unwrap_or(0);
            }
            _ => {}
        }
    }

    base_score_up += character_rank_bonus;
    let base_clamped = capacity::score(base_score_up, skill_limit, "base skill score")?;
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
        return Ok(result);
    }

    if let Some(diff) = diff {
        result.slot = SkillSlot {
            skill_type: 2,
            value: 0,
        };
        result.diff = Some(diff);
        result.skill_min = diff.base;
        result.skill_max = capacity::score(
            i64::from(diff.base) + i64::from(diff.increment) * 2,
            skill_limit,
            "different-unit upper bound",
        )?;
        result.full.base_score_up = result.skill_max as f64;
        return Ok(result);
    }

    if ref_rate > 0 && ref_max > 0 {
        let ref_max_clamped = match skill_limit {
            Some(limit) => {
                let headroom = limit.saturating_sub(base_clamped as u32);
                capacity::score(
                    i64::from(ref_max),
                    Some(headroom),
                    "reference skill addition",
                )?
            }
            None => capacity::score(i64::from(ref_max), None, "reference skill addition")?,
        };
        result.slot = SkillSlot {
            skill_type: 3,
            value: 0,
        };
        result.ref_skill = Some(RefSkill {
            rate: capacity::score(i64::from(ref_rate), None, "reference skill rate")?,
            max: ref_max_clamped,
        });
        result.skill_min = base_clamped;
        result.skill_max = capacity::score(
            i64::from(base_clamped) + i64::from(ref_max_clamped),
            None,
            "reference skill upper bound",
        )?;
        result.full.ref_max = ref_max_clamped as f64;
        return Ok(result);
    }

    result.slot = SkillSlot {
        skill_type: 0,
        value: base_clamped,
    };
    Ok(result)
}
