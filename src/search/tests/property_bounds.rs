//! property bounds contracts.
use super::*;

#[test]
fn search_suffix_bound_is_sound_and_zero_pool_is_zero() {
    let mut cards = five_unique_cards().to_vec();
    cards.push(TestCard {
        char_id: 5,
        attr: 1,
        unit_mask: 1,
        game_id: 105,
        power: 50,
        skill: SkillSlot {
            skill_type: 0,
            value: 5,
        },
        base_bonus: 0,
        limited_bonus: 0,
        power_max: 50,
        skill_max: 5,
    });
    let pool = build_pool(&cards);
    let search_ctx = ctx(ScoreTarget::Score);
    let suffix = SuffixBound::build(&pool, &search_ctx);

    let selected = pool.card_idx(0).unwrap_or(crate::pool::CardIdx::new(0));
    let mut used = UsedSet::new();
    used.insert(pool.char_id(selected));
    let partial = PartialDeck {
        power: pool.power_max(selected),
        skill: pool.skill_max(selected) as u32,
        bonus: pool.event_bonus_exact(selected).base_ceil(),
        max_skill: pool.skill_max(selected),
        limited_count: 0,
    };

    let upper = suffix.upper_bound_with_depth(1, &used, &partial);
    let mut best_real = 0u64;
    let mut i = 1usize;
    while i < pool.count() {
        let c1 = crate::pool::CardIdx::new(i as u16);
        let mut j = i + 1;
        while j < pool.count() {
            let c2 = crate::pool::CardIdx::new(j as u16);
            let mut k = j + 1;
            while k < pool.count() {
                let c3 = crate::pool::CardIdx::new(k as u16);
                let mut l = k + 1;
                while l < pool.count() {
                    let c4 = crate::pool::CardIdx::new(l as u16);
                    let deck = [selected, c1, c2, c3, c4];
                    let score = leaf_evaluate(&pool, &search_ctx, &deck);
                    if score > best_real {
                        best_real = score;
                    }
                    l += 1;
                }
                k += 1;
            }
            j += 1;
        }
        i += 1;
    }
    assert!(upper >= best_real);

    let zero_cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 200,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 201,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 202,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 203,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 204,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
    ];
    let zero_pool = build_pool(&zero_cards);
    let zero_suffix = SuffixBound::build(&zero_pool, &ctx(ScoreTarget::Power));
    assert_eq!(
        zero_suffix.upper_bound_with_depth(0, &UsedSet::new(), &PartialDeck::default()),
        0
    );
}
