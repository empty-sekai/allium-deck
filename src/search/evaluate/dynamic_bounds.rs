//! Literal per-card skill upper bounds across every materialized skill state.
use super::*;
use crate::pool::PoolBuilder;

fn mixed_skill_pool() -> CardPool {
    let mut builder = PoolBuilder::new(7);
    builder.add_unit_count_skill(UnitCountSkill {
        unit: 0,
        score_up: [30, 100, 80, 190, 140],
    });
    builder.add_diff_skill(DiffSkill {
        base: 100,
        increment: 90,
    });
    builder.add_ref_skill(RefSkill { rate: 100, max: 55 });
    builder.add_ref_skill(RefSkill { rate: 33, max: 120 });
    for dense in 0..7u16 {
        let (kind, value, lower, upper) = match dense {
            0 => (0, 255, 255, 255),
            1 => (1, 1, 30, 190),
            2 => (2, 1, 100, 240), // min(raw different-unit value, event cap)
            3 => (3, 1, 200, 255),
            4 => (3, 2, 80, 200),
            5 => (0, 0, 0, 0),
            _ => (0, 70, 70, 70),
        };
        let units = [1, 1, 2, 3, 4, 8, 16][dense as usize];
        builder.set_power_values(dense, [1_000; 8]);
        builder.set_power_lut(dense, 0);
        builder.set_power_max(dense, 1_000);
        builder.set_skill(
            dense,
            SkillSlot {
                skill_type: kind,
                value,
            },
        );
        builder.set_skill_min(dense, lower);
        builder.set_skill_max(dense, upper);
        builder.set_char_id(dense, dense as u8 + 1);
        builder.set_attr(dense, dense as u8 % 5);
        builder.set_unit_mask(dense, units);
        builder.set_game_id(dense, 100 + dense);
        builder.mark_char(dense as u8 + 1, dense);
        builder.mark_attr(dense as u8 % 5, dense);
        for unit in 0..6 {
            if units & (1 << unit) != 0 {
                builder.mark_unit(unit, dense);
            }
        }
    }
    builder.freeze()
}

#[test]
fn every_resolved_dynamic_skill_is_bounded_without_a_margin() {
    let pool = mixed_skill_pool();
    let mut states = 0u64;
    for selection in 0u8..128 {
        if selection.count_ones() != DECK_SIZE as u32 {
            continue;
        }
        let picked = pool
            .indices()
            .filter(|card| selection & (1 << card.raw()) != 0)
            .collect::<Vec<_>>();
        let mut deck: [CardIdx; DECK_SIZE] = picked.try_into().unwrap();
        for _ in 0..DECK_SIZE {
            for reference in [
                SkillReferenceStrategy::Max,
                SkillReferenceStrategy::Min,
                SkillReferenceStrategy::Average,
            ] {
                for order in [
                    LiveSkillOrder::Best,
                    LiveSkillOrder::Worst,
                    LiveSkillOrder::Average,
                    LiveSkillOrder::Specific,
                ] {
                    for live_type in [LiveType::Solo, LiveType::Multi, LiveType::Auto] {
                        for training in 0..4 {
                            let mut ctx = super::tests::ctx(live_type);
                            ctx.skill_reference_strategy = reference;
                            ctx.live_skill_order = order;
                            ctx.specific_skill_order = Some([4, 2, 0, 3, 1]);
                            ctx.keep_after_training_state = training != 0;
                            ctx.skill_is_after_training = (0..pool.count())
                                .map(|i| training == 1 || i % 2 == 0)
                                .collect();
                            ctx.trained_to_special_image = (0..pool.count())
                                .map(|i| training == 2 || i % 2 != 0)
                                .collect();
                            let prepared = prepare_skills(&pool, &ctx, &deck);
                            let mut mask = prepared.enumerate_mask;
                            loop {
                                let resolved =
                                    materialize_permutation(&pool, &deck, &ctx, &prepared, mask);
                                for (&card, actual) in deck.iter().zip(&resolved.skills) {
                                    let upper = f64::from(pool.skill_max(card));
                                    assert!(actual.score_up.is_finite() && actual.score_up >= 0.0);
                                    assert!(
                                        actual.score_up <= upper,
                                        "card={card:?} skill={} upper={upper} ref={reference:?} order={order:?} training={training}",
                                        actual.score_up
                                    );
                                }
                                // A role-aware additive relaxation bounds the exact
                                // leader-plus-one-fifth-members Skill objective.
                                let upper = resolved
                                    .order
                                    .iter()
                                    .enumerate()
                                    .map(|(slot, &position)| {
                                        f64::from(pool.skill_max(deck[position]))
                                            * if slot == 0 { 1.0 } else { 0.2 }
                                    })
                                    .sum::<f64>();
                                assert!(resolved.multi_live_score_up <= upper);
                                states += 1;
                                if mask == 0 {
                                    break;
                                }
                                mask = (mask - 1) & prepared.enumerate_mask;
                            }
                        }
                    }
                }
            }
            deck.rotate_left(1);
        }
    }
    assert!(
        states > 20_000,
        "expected exhaustive mixed skill-state coverage"
    );
    eprintln!(
        "DYNAMIC_SKILL_BOUND states={states} card_checks={}",
        states * DECK_SIZE as u64
    );
}
