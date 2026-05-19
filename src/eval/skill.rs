use crate::types::{
    CardId, CardSpec, DefaultImage, SkillInfo, SkillReferenceStrategy, Unit, DECK_SIZE, UNIT_COUNT,
};

#[derive(Debug, Copy, Clone)]
pub(crate) struct LiveSkill {
    pub score_up: f64,
    pub score_up_to_reference: f64,
    pub life_recovery: f64,
    pub ref_rate: f64,
    pub ref_max: f64,
    pub has_ref: bool,
}

impl Default for LiveSkill {
    fn default() -> Self {
        Self {
            score_up: 0.0,
            score_up_to_reference: 0.0,
            life_recovery: 0.0,
            ref_rate: 0.0,
            ref_max: 0.0,
            has_ref: false,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct PreparedSkills {
    pub skills: [[LiveSkill; 2]; DECK_SIZE],
    pub enumerate_mask: u32,
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct EvaluatedPermutation {
    pub order: [usize; DECK_SIZE],
    pub skills: [LiveSkill; DECK_SIZE],
    pub multi_live_score_up: f64,
    pub chosen_mask: u32,
}

pub(crate) fn prepare_skills(
    cards: &[&CardSpec; DECK_SIZE],
    unit_counts: &[i32; UNIT_COUNT],
    unit_kind_count: i32,
    keep_after_training_state: bool,
    skill_score_up_limit: Option<f64>,
    is_mysekai: bool,
) -> PreparedSkills {
    let mut prepared = [[LiveSkill::default(); 2]; DECK_SIZE];
    let mut enumerate_mask = 0_u32;
    let diff_index = (unit_kind_count - 1).clamp(0, 2) as usize;

    let mut card_index = 0;
    while card_index < DECK_SIZE {
        let card = cards[card_index];
        let mut s2 = LiveSkill::default();
        let mut s2_after_training = false;
        let mut s2_skill_id = 0;
        let mut unit_index = 0;
        while unit_index < card.unit_count as usize {
            let unit = card.units[unit_index];
            let info = card.skill.resolved[unit as usize][skill_key(unit_counts[unit as usize])];
            let current = live_skill_from_info(info, false, skill_score_up_limit);
            if current.score_up > s2.score_up {
                s2 = current;
                s2_after_training = info.is_after_training;
                s2_skill_id = info.skill_id;
            }
            unit_index += 1;
        }

        let ref_info = card.skill.resolved[Unit::Ref as usize][skill_key(1)];
        let mut s1 = LiveSkill::default();
        let mut s1_skill_id = 0;
        let mut need_enumerate = false;
        if ref_info.skill_id != 0 && ref_info.skill_id != s2_skill_id {
            let current = live_skill_from_info(ref_info, true, skill_score_up_limit);
            // C++ deck-information/deck-calculator.cpp:198-201 selects Ref as an enumerated candidate.
            if current.score_up > s1.score_up {
                s1 = current;
                s1_skill_id = ref_info.skill_id;
                need_enumerate = true;
            }
        }

        let diff_info = card.skill.diff[diff_index];
        if diff_info.skill_id != 0 && diff_info.skill_id != s2_skill_id {
            let current = live_skill_from_info(diff_info, false, skill_score_up_limit);
            // B 实例 L701-704 resolves Diff by (unit_kind_count - 1).clamp(0, 2).
            if current.score_up > s1.score_up {
                s1 = current;
                s1_skill_id = diff_info.skill_id;
                need_enumerate = false;
            }
        }

        if keep_after_training_state {
            if !matches!(card.default_image, DefaultImage::SpecialTraining) && s2_after_training {
                if s1.score_up > 0.0 {
                    s2 = s1;
                }
            }
        } else if need_enumerate && s1_skill_id != 0 {
            enumerate_mask |= 1 << card_index;
        } else if s1.score_up > s2.score_up {
            s2 = s1;
        }

        prepared[card_index] = [s1, s2];
        card_index += 1;
    }

    if is_mysekai {
        // B5: Mysekai points ignore permutation/skills, so the enumeration mask is forced to zero.
        enumerate_mask = 0;
    }

    PreparedSkills {
        skills: prepared,
        enumerate_mask,
    }
}

pub(crate) fn materialize_permutation(
    cards: &[CardId; DECK_SIZE],
    selected: &[&CardSpec; DECK_SIZE],
    prepared: &PreparedSkills,
    mask: u32,
    best_skill_as_leader: bool,
    strategy: SkillReferenceStrategy,
) -> EvaluatedPermutation {
    let mut skills = [LiveSkill::default(); DECK_SIZE];
    let mut index = 0;
    while index < DECK_SIZE {
        let mut skill = if mask & (1 << index) != 0 {
            prepared.skills[index][0]
        } else {
            prepared.skills[index][1]
        };
        skill.score_up_to_reference = skill.score_up;
        skills[index] = skill;
        index += 1;
    }

    let mut ref_index = 0;
    while ref_index < DECK_SIZE {
        if skills[ref_index].has_ref {
            // moe deck-information/deck-calculator.cpp:254-270 floors each card reference before choosing strategy.
            skills[ref_index].score_up -= skills[ref_index].ref_max;
            let mut reference_scores = [0.0_f64; DECK_SIZE - 1];
            let mut reference_len = 0;
            let mut other = 0;
            while other < DECK_SIZE {
                if other != ref_index {
                    reference_scores[reference_len] =
                        (skills[other].score_up_to_reference * skills[ref_index].ref_rate / 100.0)
                            .floor()
                            .min(skills[ref_index].ref_max);
                    reference_len += 1;
                }
                other += 1;
            }
            let chosen = choose_reference_score(&reference_scores, reference_len, strategy);
            skills[ref_index].score_up += chosen;
        }
        ref_index += 1;
    }

    let mut order = [0_usize, 1, 2, 3, 4];
    if best_skill_as_leader {
        let mut best_pos = 0;
        let mut pos = 1;
        while pos < DECK_SIZE {
            let left = order[pos];
            let right = order[best_pos];
            if skills[left].score_up > skills[right].score_up
                || (skills[left].score_up == skills[right].score_up && cards[left] < cards[right])
            {
                best_pos = pos;
            }
            pos += 1;
        }
        order.swap(0, best_pos);
    } else {
        sort_tail_by_card_id(&mut order, selected);
    }

    let mut multi_live_score_up = skills[order[0]].score_up;
    let mut score_index = 1;
    while score_index < DECK_SIZE {
        multi_live_score_up += skills[order[score_index]].score_up * 0.2;
        score_index += 1;
    }

    EvaluatedPermutation {
        order,
        skills,
        multi_live_score_up,
        chosen_mask: mask,
    }
}

fn live_skill_from_info(
    info: SkillInfo,
    use_reference_ceiling: bool,
    limit: Option<f64>,
) -> LiveSkill {
    let mut base = info.base_score_up;
    if let Some(max_score_up) = limit {
        base = base.min(max_score_up);
    }
    let mut ref_max = if use_reference_ceiling && info.has_ref {
        info.ref_max
    } else {
        0.0
    };
    if let Some(max_score_up) = limit {
        ref_max = ref_max.min((max_score_up - base).max(0.0));
    }
    // TS event-service.ts:125-131 and C++ card-skill-calculator.cpp:10-21 source the limit from masterdata.
    LiveSkill {
        score_up: base + ref_max,
        score_up_to_reference: 0.0,
        life_recovery: info.life_recovery,
        ref_rate: info.ref_rate,
        ref_max,
        has_ref: use_reference_ceiling && info.has_ref,
    }
}

fn skill_key(unit_member: i32) -> usize {
    if unit_member == DECK_SIZE as i32 {
        1
    } else {
        0
    }
}

fn sort_tail_by_card_id(order: &mut [usize; DECK_SIZE], selected: &[&CardSpec; DECK_SIZE]) {
    let mut left = 2;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 1 {
            let prev = order[cursor - 1];
            let current = order[cursor];
            if selected[prev].card_id <= selected[current].card_id {
                break;
            }
            order.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

fn choose_reference_score(
    reference_scores: &[f64; DECK_SIZE - 1],
    reference_len: usize,
    strategy: SkillReferenceStrategy,
) -> f64 {
    match strategy {
        SkillReferenceStrategy::Max => {
            let mut best = 0.0;
            let mut index = 0;
            while index < reference_len {
                if reference_scores[index] > best {
                    best = reference_scores[index];
                }
                index += 1;
            }
            best
        }
        SkillReferenceStrategy::Min => {
            let mut best = reference_scores[0];
            let mut index = 1;
            while index < reference_len {
                if reference_scores[index] < best {
                    best = reference_scores[index];
                }
                index += 1;
            }
            best
        }
        SkillReferenceStrategy::Average => {
            let mut total = 0.0;
            let mut index = 0;
            while index < reference_len {
                total += reference_scores[index];
                index += 1;
            }
            total / reference_len as f64
        }
    }
}
