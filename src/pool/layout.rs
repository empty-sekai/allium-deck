use std::mem::size_of;

use super::types::{
    ATTR_MASK_COUNT, CHAR_MASK_COUNT, EventBonusHot, Mask, SkillSlot, UNIT_MASK_COUNT,
};

/// 向上按 `align` 对齐。
pub(crate) const fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PoolLayout {
    pub(crate) total_size: usize,
    pub(crate) off_power_values: usize,
    pub(crate) off_power_lut: usize,
    pub(crate) off_skill_table: usize,
    pub(crate) off_event_bonus: usize,
    pub(crate) off_char_ids: usize,
    pub(crate) off_attrs: usize,
    pub(crate) off_unit_masks: usize,
    pub(crate) off_game_ids: usize,
    pub(crate) off_power_max: usize,
    pub(crate) off_skill_min: usize,
    pub(crate) off_skill_max: usize,
    pub(crate) off_masks: usize,
}

impl PoolLayout {
    pub(crate) fn compute(n: usize) -> Self {
        let mut offset = 0usize;

        let off_power_values = offset;
        offset += align_up(n * size_of::<[u16; 8]>(), 64);

        let off_power_lut = offset;
        offset += align_up(n * size_of::<u32>(), 64);

        let off_skill_table = offset;
        offset += align_up(n * size_of::<SkillSlot>(), 64);

        let off_event_bonus = offset;
        offset += align_up(n * size_of::<EventBonusHot>(), 64);

        let off_char_ids = offset;
        offset += align_up(n * size_of::<u8>(), 64);

        let off_attrs = offset;
        offset += align_up(n * size_of::<u8>(), 64);

        let off_unit_masks = offset;
        offset += align_up(n * size_of::<u8>(), 64);

        let off_game_ids = offset;
        offset += align_up(n * size_of::<u16>(), 64);

        let off_power_max = offset;
        offset += align_up(n * size_of::<u32>(), 64);

        let off_skill_min = offset;
        offset += align_up(n * size_of::<u8>(), 64);

        let off_skill_max = offset;
        offset += align_up(n * size_of::<u8>(), 64);

        let off_masks = offset;
        offset += (CHAR_MASK_COUNT + UNIT_MASK_COUNT + ATTR_MASK_COUNT) * size_of::<Mask>();

        Self {
            total_size: align_up(offset, 64),
            off_power_values,
            off_power_lut,
            off_skill_table,
            off_event_bonus,
            off_char_ids,
            off_attrs,
            off_unit_masks,
            off_game_ids,
            off_power_max,
            off_skill_min,
            off_skill_max,
            off_masks,
        }
    }
}
