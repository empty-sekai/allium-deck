mod arena;
mod builder;
mod card_pool;
mod layout;
mod types;

pub use builder::PoolBuilder;
pub use card_pool::CardPool;
pub use types::{
    CardIdx, DiffSkill, EventBonusHot, Mask, RefSkill, SkillSlot, SpecialTables, UnitCountSkill,
    MASK_WORDS,
};

#[cfg(test)]
mod tests {
    use super::{
        CardIdx, CardPool, DiffSkill, EventBonusHot, Mask, PoolBuilder, RefSkill, SkillSlot,
        UnitCountSkill,
    };

    fn must_card_idx(pool: &CardPool, dense_idx: u16) -> CardIdx {
        match pool.card_idx(dense_idx) {
            Some(idx) => idx,
            None => panic!("missing card index"),
        }
    }

    fn must_mask<'a>(mask: Option<&'a Mask>) -> &'a Mask {
        match mask {
            Some(mask) => mask,
            None => panic!("missing mask"),
        }
    }

    fn build_sample_pool() -> CardPool {
        let mut builder = PoolBuilder::new(3);

        builder.add_unit_count_skill(UnitCountSkill {
            unit: 2,
            score_up: [10, 20, 30, 40, 50],
        });
        builder.add_diff_skill(DiffSkill {
            base: 11,
            increment: 7,
        });
        builder.add_ref_skill(RefSkill { rate: 8, max: 90 });

        builder.set_power_values(0, [1, 2, 3, 4, 5, 6, 7, 8]);
        builder.set_power_lut(0, 0xAA55);
        builder.set_skill(
            0,
            SkillSlot {
                skill_type: 1,
                value: 1,
            },
        );
        builder.set_event_bonus(0, EventBonusHot::from_whole(5, 9));
        builder.set_char_id(0, 1);
        builder.set_attr(0, 2);
        builder.set_unit_mask(0, 0b000011);
        builder.set_game_id(0, 101);
        builder.set_power_max(0, 2000);
        builder.set_skill_min(0, 12);
        builder.set_skill_max(0, 34);
        builder.mark_char(1, 0);
        builder.mark_unit(0, 0);
        builder.mark_unit(1, 0);
        builder.mark_attr(2, 0);

        builder.set_power_values(1, [11, 12, 13, 14, 15, 16, 17, 18]);
        builder.set_power_lut(1, 0xBB66);
        builder.set_skill(
            1,
            SkillSlot {
                skill_type: 2,
                value: 1,
            },
        );
        builder.set_event_bonus(1, EventBonusHot::from_whole(6, 10));
        builder.set_char_id(1, 3);
        builder.set_attr(1, 4);
        builder.set_unit_mask(1, 0b000100);
        builder.set_game_id(1, 202);
        builder.set_power_max(1, 2100);
        builder.set_skill_min(1, 22);
        builder.set_skill_max(1, 44);
        builder.mark_char(3, 1);
        builder.mark_unit(2, 1);
        builder.mark_attr(4, 1);

        builder.set_power_values(2, [21, 22, 23, 24, 25, 26, 27, 28]);
        builder.set_power_lut(2, 0xCC77);
        builder.set_skill(
            2,
            SkillSlot {
                skill_type: 3,
                value: 1,
            },
        );
        builder.set_event_bonus(2, EventBonusHot::from_whole(7, 11));
        builder.set_char_id(2, 5);
        builder.set_attr(2, 1);
        builder.set_unit_mask(2, 0b001001);
        builder.set_game_id(2, 303);
        builder.set_power_max(2, 2200);
        builder.set_skill_min(2, 32);
        builder.set_skill_max(2, 54);
        builder.mark_char(5, 2);
        builder.mark_unit(0, 2);
        builder.mark_unit(3, 2);
        builder.mark_attr(1, 2);

        builder.freeze()
    }

    #[test]
    fn roundtrip_pool_values_match() {
        let pool = build_sample_pool();

        assert_eq!(pool.count(), 3);
        assert_eq!(pool.indices().count(), 3);

        let idx0 = must_card_idx(&pool, 0);
        assert_eq!(pool.power_values(idx0), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(pool.power_lut(idx0), 0xAA55);
        assert_eq!(
            pool.skill(idx0),
            SkillSlot {
                skill_type: 1,
                value: 1
            }
        );
        assert_eq!(pool.event_bonus(idx0), &EventBonusHot::from_whole(5, 9));
        assert_eq!(pool.char_id(idx0), 1);
        assert_eq!(pool.attr(idx0), 2);
        assert_eq!(pool.unit_mask_raw(idx0), 0b000011);
        assert_eq!(pool.game_id(idx0), 101);
        assert_eq!(pool.power_max(idx0), 2000);
        assert_eq!(pool.skill_min(idx0), 12);
        assert_eq!(pool.skill_max(idx0), 34);

        assert_eq!(
            pool.special().unit_count(),
            &[UnitCountSkill {
                unit: 2,
                score_up: [10, 20, 30, 40, 50]
            }]
        );
        assert_eq!(
            pool.special().diff(),
            &[DiffSkill {
                base: 11,
                increment: 7
            }]
        );
        assert_eq!(
            pool.special().ref_skills(),
            &[RefSkill { rate: 8, max: 90 }]
        );

        assert!(must_mask(pool.char_mask(1)).test(0));
        assert!(must_mask(pool.unit_mask_at(0)).test(0));
        assert!(must_mask(pool.unit_mask_at(1)).test(0));
        assert!(must_mask(pool.attr_mask(2)).test(0));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn oob_panics_in_debug() {
        let pool = PoolBuilder::new(1).freeze();
        let _ = pool.power_lut(CardIdx::new(1));
    }

    #[test]
    fn scatter_freeze_smoke_for_300_cards() {
        let mut builder = PoolBuilder::new(300);
        for idx in 0..300u16 {
            let idx_u8 = idx as u8;
            builder.set_power_values(idx, [idx; 8]);
            builder.set_power_lut(idx, idx as u32 * 3);
            builder.set_skill(
                idx,
                SkillSlot {
                    skill_type: 0,
                    value: idx_u8,
                },
            );
            builder.set_event_bonus(
                idx,
                EventBonusHot::from_whole(idx_u8, idx_u8.wrapping_add(1)),
            );
            builder.set_char_id(idx, (idx % 27) as u8);
            builder.set_attr(idx, (idx % 5) as u8);
            builder.set_unit_mask(idx, 1u8 << (idx % 6));
            builder.set_game_id(idx, 1000 + idx);
            builder.set_power_max(idx, idx as u32 + 1);
            builder.set_skill_min(idx, (idx % 200) as u8);
            builder.set_skill_max(idx, (idx % 200) as u8 + 1);
            builder.mark_char((idx % 27) as u8, idx);
            builder.mark_unit((idx % 6) as u8, idx);
            builder.mark_attr((idx % 5) as u8, idx);
        }

        let pool = builder.freeze();
        assert_eq!(pool.count(), 300);

        let first = must_card_idx(&pool, 0);
        let last = must_card_idx(&pool, 299);
        assert_eq!(pool.power_values(first), &[0; 8]);
        assert_eq!(pool.power_lut(last), 897);
        assert_eq!(pool.char_id(last), 2);
        assert_eq!(pool.attr(last), 4);
        assert!(must_mask(pool.char_mask(2)).count_ones() > 0);
    }

    #[test]
    fn compact_rebuilds_columns_and_masks() {
        let pool = build_sample_pool();
        let compacted = pool.compact(&[true, false, true]);

        assert_eq!(compacted.count(), 2);

        let kept0 = must_card_idx(&compacted, 0);
        let kept1 = must_card_idx(&compacted, 1);

        assert_eq!(compacted.power_values(kept0), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            compacted.power_values(kept1),
            &[21, 22, 23, 24, 25, 26, 27, 28]
        );
        assert_eq!(compacted.char_id(kept0), 1);
        assert_eq!(compacted.char_id(kept1), 5);
        assert_eq!(compacted.unit_mask_raw(kept0), 0b000011);
        assert_eq!(compacted.unit_mask_raw(kept1), 0b001001);
        assert_eq!(compacted.special(), pool.special());

        let char1 = must_mask(compacted.char_mask(1));
        let char5 = must_mask(compacted.char_mask(5));
        let unit0 = must_mask(compacted.unit_mask_at(0));
        let unit3 = must_mask(compacted.unit_mask_at(3));
        let attr1 = must_mask(compacted.attr_mask(1));
        let attr2 = must_mask(compacted.attr_mask(2));

        assert!(char1.test(0));
        assert!(char5.test(1));
        assert!(unit0.test(0));
        assert!(unit0.test(1));
        assert!(unit3.test(1));
        assert!(attr2.test(0));
        assert!(attr1.test(1));
        assert!(!unit3.test(0));
        assert!(must_mask(compacted.char_mask(3)).is_empty());
    }

    #[test]
    fn mask_bit_operations_work() {
        let mut mask = Mask::EMPTY;
        mask.set(0);
        mask.set(63);
        mask.set(64);
        mask.set(511);

        assert!(mask.test(0));
        assert!(mask.test(63));
        assert!(mask.test(64));
        assert!(mask.test(511));
        assert_eq!(mask.count_ones(), 4);
        assert_eq!(mask.lowest_set_bit(), Some(0));

        let mut other = Mask::EMPTY;
        other.set(64);
        other.set(100);
        let anded = mask.and(&other);
        assert!(anded.test(64));
        assert_eq!(anded.count_ones(), 1);

        mask.clear_lowest();
        assert_eq!(mask.lowest_set_bit(), Some(63));
        mask.clear_lowest();
        assert_eq!(mask.lowest_set_bit(), Some(64));
        mask.clear_lowest();
        assert_eq!(mask.lowest_set_bit(), Some(511));
        mask.clear_lowest();
        assert!(mask.is_empty());
    }
}
