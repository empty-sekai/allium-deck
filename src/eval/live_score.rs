use crate::eval::skill::LiveSkill;
use crate::types::{EventType, LiveSkillOrder, LiveType, MusicParams, DECK_SIZE};

pub(crate) fn calc_live_score(
    skills: &[LiveSkill; DECK_SIZE],
    order: &[usize; DECK_SIZE],
    total_power: i32,
    music: &MusicParams,
    live_type: LiveType,
    live_skill_order: LiveSkillOrder,
    specific_skill_order: Option<&[usize; DECK_SIZE]>,
    multi_teammate_score_up: Option<i32>,
    multi_teammate_power: Option<i32>,
) -> i32 {
    let mut slots = sorted_live_skills(
        skills,
        order,
        live_type,
        live_skill_order,
        specific_skill_order,
        multi_teammate_score_up,
    );
    let mut skill_rates = music.skill_scores[skill_score_index(live_type)];
    apply_live_skill_order(
        &mut slots,
        &mut skill_rates,
        live_skill_order,
        specific_skill_order,
    );

    let base_rate = match live_type {
        LiveType::Auto | LiveType::ChallengeAuto => music.base_score_auto,
        LiveType::Multi | LiveType::Cheerful => music.base_score + music.fever_score * 0.5,
        _ => music.base_score,
    };

    let mut rate = base_rate;
    let mut index = 0;
    while index < DECK_SIZE + 1 {
        rate += slots[index].score_up * skill_rates[index] / 100.0;
        index += 1;
    }

    let mut power_sum = DECK_SIZE as i32 * total_power;
    if let Some(teammate_power) = multi_teammate_power {
        power_sum = total_power + teammate_power * (DECK_SIZE as i32 - 1);
    }
    // C++ live-score/live-calculator.cpp:171 keeps active bonus on Multi only; Cheerful does not receive it.
    let active_bonus = if matches!(live_type, LiveType::Multi) {
        DECK_SIZE as f64 * 0.015 * power_sum as f64
    } else {
        0.0
    };

    (rate * total_power as f64 * 4.0 + active_bonus) as i32
}

pub(crate) fn calc_event_point(
    live_type: LiveType,
    _event_type: EventType,
    self_score: i32,
    music_rate: f64,
    deck_bonus: f64,
    boost_rate: f64,
    other_score: Option<i32>,
    life: i32,
) -> i32 {
    let music_rate = music_rate / 100.0;
    let deck_rate = deck_bonus / 100.0 + 1.0;
    let other_score = other_score.unwrap_or(self_score * (DECK_SIZE as i32 - 1));

    if matches!(live_type, LiveType::Challenge | LiveType::ChallengeAuto) {
        // B 实例 L979-982: ChallengeAuto is handled explicitly with Challenge.
        return (100 + self_score / 20_000) * 120;
    }
    if !matches!(live_type, LiveType::Multi | LiveType::Cheerful) {
        let base_score = 100 + self_score / 20_000;
        return ((base_score as f64 * music_rate * deck_rate) as i32 as f64 * boost_rate) as i32;
    }

    let base_score = 110 + (self_score as f64 / 17_000.0) as i32 + (other_score / 340_000).min(13);
    if matches!(live_type, LiveType::Cheerful) {
        // C++ event-point/event-calculator.cpp:24-31 applies life_rate only in Cheerful.
        let life_rate = 1.15 + (life as f64 / 5000.0).clamp(0.1, 0.2);
        let boosted =
            ((base_score as f64 * music_rate * deck_rate) as i32 as f64 * life_rate) as i32;
        (boosted as f64 * boost_rate) as i32
    } else {
        ((base_score as f64 * music_rate * deck_rate) as i32 as f64 * boost_rate) as i32
    }
}

fn sorted_live_skills(
    skills: &[LiveSkill; DECK_SIZE],
    order: &[usize; DECK_SIZE],
    live_type: LiveType,
    live_skill_order: LiveSkillOrder,
    specific_skill_order: Option<&[usize; DECK_SIZE]>,
    multi_teammate_score_up: Option<i32>,
) -> [LiveSkill; DECK_SIZE + 1] {
    let mut buffer = [LiveSkill::default(); DECK_SIZE + 1];
    if matches!(live_type, LiveType::Multi | LiveType::Cheerful) {
        // C++ live-score/live-calculator.cpp:183-190 computes self score-up as leader + others / deck size.
        let mut self_score_up = skills[order[0]].score_up;
        let mut index = 1;
        while index < DECK_SIZE {
            self_score_up += skills[order[index]].score_up / DECK_SIZE as f64;
            index += 1;
        }
        let self_skill = LiveSkill {
            score_up: self_score_up,
            life_recovery: skills[order[0]].life_recovery,
            ..LiveSkill::default()
        };
        let other_skill = if let Some(score_up) = multi_teammate_score_up {
            LiveSkill {
                score_up: score_up as f64,
                ..LiveSkill::default()
            }
        } else {
            self_skill
        };
        buffer[0] = self_skill;
        let mut slot = 1;
        while slot < DECK_SIZE {
            buffer[slot] = other_skill;
            slot += 1;
        }
        buffer[DECK_SIZE] = self_skill;
        return buffer;
    }

    let mut index = 0;
    while index < DECK_SIZE {
        buffer[index] = skills[order[index]];
        index += 1;
    }
    buffer[DECK_SIZE] = skills[order[0]];
    if matches!(live_skill_order, LiveSkillOrder::Specific) && specific_skill_order.is_none() {
        return buffer;
    }
    buffer
}

fn apply_live_skill_order(
    slots: &mut [LiveSkill; DECK_SIZE + 1],
    skill_rates: &mut [f64; DECK_SIZE + 1],
    live_skill_order: LiveSkillOrder,
    specific_skill_order: Option<&[usize; DECK_SIZE]>,
) {
    match live_skill_order {
        LiveSkillOrder::Best => {
            // moe live-score/live-calculator.cpp:124-130 sorts skill rates ascending whenever skills are sorted.
            sort_slots_ascending(slots);
            sort_rates_ascending(skill_rates);
        }
        LiveSkillOrder::Worst => {
            // Worst pairs large skills with small rates: slots descend, rates still ascend.
            sort_slots_descending(slots);
            sort_rates_ascending(skill_rates);
        }
        LiveSkillOrder::Average => {
            let mut total = 0.0;
            let mut index = 0;
            while index < DECK_SIZE {
                total += slots[index].score_up;
                index += 1;
            }
            let average = total / DECK_SIZE as f64;
            let mut slot = 0;
            while slot < DECK_SIZE {
                slots[slot].score_up = average;
                slot += 1;
            }
        }
        LiveSkillOrder::Specific => {
            let order = specific_skill_order.expect("validated specific skill order");
            let original = *slots;
            let mut index = 0;
            while index < DECK_SIZE {
                slots[index] = original[order[index]];
                index += 1;
            }
            slots[DECK_SIZE] = original[DECK_SIZE];
        }
    }
}

fn sort_slots_ascending(slots: &mut [LiveSkill; DECK_SIZE + 1]) {
    let mut left = 1;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 0 {
            if slots[cursor - 1].score_up <= slots[cursor].score_up {
                break;
            }
            slots.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

fn sort_slots_descending(slots: &mut [LiveSkill; DECK_SIZE + 1]) {
    let mut left = 1;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 0 {
            if slots[cursor - 1].score_up >= slots[cursor].score_up {
                break;
            }
            slots.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

fn sort_rates_ascending(skill_rates: &mut [f64; DECK_SIZE + 1]) {
    let mut left = 1;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 0 {
            if skill_rates[cursor - 1] <= skill_rates[cursor] {
                break;
            }
            skill_rates.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

fn skill_score_index(live_type: LiveType) -> usize {
    match live_type {
        LiveType::Multi | LiveType::Cheerful => 1,
        LiveType::Auto | LiveType::ChallengeAuto => 2,
        _ => 0,
    }
}
