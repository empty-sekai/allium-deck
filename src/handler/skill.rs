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
    /// Value another member's reference skill reads from this skill: its
    /// static maximum at the skill level, see `static_reference_value`.
    pub reference_value: u16,
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
    let mut reference = StaticMaximum::default();

    for effect in effects {
        match effect.kind {
            PreparedSkillEffectKind::ScoreUp => {
                base_score_up = base_score_up.max(i64::from(effect.value));
            }
            PreparedSkillEffectKind::LifeRecovery => life_recovery += i64::from(effect.value),
            PreparedSkillEffectKind::CharacterRank => {
                if let Some(rank) = effect.activate_character_rank {
                    if rank <= character_rank {
                        character_rank_bonus = character_rank_bonus.max(i64::from(effect.value));
                    }
                    reference.character_rank_row(rank, effect.value);
                }
            }
            PreparedSkillEffectKind::UnitCount => {
                reference.unit_count_row(effect.value);
                unit_count_unit = effect.unit;
                if let Some(count) = effect.unit_member_count
                    && (1..=5).contains(&count)
                {
                    unit_count_values[(count - 1) as usize] =
                        capacity::score(i64::from(effect.value), skill_limit, "skill score")?;
                }
            }
            PreparedSkillEffectKind::Diff => {
                reference.different_unit_increment = effect.additional_value.unwrap_or(0);
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
                reference.reference_max = ref_max;
            }
            _ => {}
        }
    }

    let reference_value = static_reference_value(base_score_up, &reference)?;
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
        reference_value,
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
            i64::from(diff.base)
                + i64::from(diff.increment) * i64::from(DiffSkill::MAX_COUNTED_UNITS),
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

/// Conditional parts of one skill at one level, each at its largest row.
#[derive(Debug, Default)]
struct StaticMaximum {
    /// Character-rank row with the highest rank threshold: `(rank, value)`.
    character_rank: Option<(i32, i32)>,
    /// Largest unit-count row; the row already includes the base score-up.
    unit_count: Option<i32>,
    /// Per-unit increment of a different-unit skill.
    different_unit_increment: i32,
    /// Largest addition of a reference skill.
    reference_max: i32,
}

impl StaticMaximum {
    fn character_rank_row(&mut self, rank: i32, value: i32) {
        // The first row wins a tie on the threshold.
        if self.character_rank.is_none_or(|(best, _)| rank > best) {
            self.character_rank = Some((rank, value));
        }
    }

    fn unit_count_row(&mut self, value: i32) {
        self.unit_count = Some(self.unit_count.map_or(value, |best| best.max(value)));
    }
}

/// Value another member's reference skill reads from this skill.
///
/// The target's skill is taken at its skill level, independent of the deck
/// and of the target's own conditions: the base score-up, the character-rank
/// row with the highest threshold (not the row the owner's rank reaches), the
/// full same-unit enhancement, every counted different unit, and the largest
/// reference addition. The value is not clamped by an event skill cap.
fn static_reference_value(base_score_up: i64, parts: &StaticMaximum) -> Result<u16, BuildError> {
    let base = base_score_up.max(0);
    let character_rank = parts
        .character_rank
        .map_or(0, |(_, value)| i64::from(value).max(0));
    let unit_count = parts
        .unit_count
        .map_or(0, |value| (i64::from(value) - base).max(0));
    let different_unit =
        i64::from(parts.different_unit_increment).max(0) * i64::from(DiffSkill::MAX_COUNTED_UNITS);
    let reference = i64::from(parts.reference_max).max(0);
    let value = (base + character_rank + unit_count + different_unit + reference) as u64;
    capacity::ensure("skill reference value", value, u64::from(u16::MAX))?;
    Ok(value as u16)
}
