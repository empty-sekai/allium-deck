use std::slice;

use super::arena::Arena;
use super::card_pool::CardPool;
use super::layout::PoolLayout;
use super::types::{
    ATTR_MASK_COUNT, CHAR_MASK_COUNT, DiffSkill, EventBonusExact, EventBonusHot, MASK_BITS, Mask,
    RefSkill, SkillSlot, SpecialTables, UNIT_MASK_COUNT, UnitCountSkill,
};

/// `CardPool` 的可写构建阶段。
pub struct PoolBuilder {
    arena: Arena,
    layout: PoolLayout,
    count: u16,
    special: SpecialTables,
}

impl PoolBuilder {
    /// 预分配指定数量候选卡的 Arena。
    pub fn new(n: u16) -> Self {
        assert!(n as usize <= MASK_BITS, "pool count exceeds mask capacity");
        let layout = PoolLayout::compute(n as usize);
        Self {
            arena: Arena::new(layout.total_size),
            layout,
            count: n,
            special: SpecialTables::default(),
        }
    }

    #[inline(always)]
    fn column_mut<T>(&mut self, offset: usize) -> &mut [T] {
        unsafe {
            slice::from_raw_parts_mut(
                self.arena.as_mut_ptr().add(offset) as *mut T,
                self.count as usize,
            )
        }
    }

    #[inline(always)]
    fn mask_slice_mut(&mut self, start: usize, len: usize) -> &mut [Mask] {
        unsafe {
            slice::from_raw_parts_mut(
                self.arena.as_mut_ptr().add(self.layout.off_masks) as *mut Mask,
                start + len,
            )
        }
        .split_at_mut(start)
        .1
    }

    #[inline(always)]
    fn char_masks_mut(&mut self) -> &mut [Mask] {
        self.mask_slice_mut(0, CHAR_MASK_COUNT)
    }

    #[inline(always)]
    fn unit_masks_mut(&mut self) -> &mut [Mask] {
        self.mask_slice_mut(CHAR_MASK_COUNT, UNIT_MASK_COUNT)
    }

    #[inline(always)]
    fn attr_masks_mut(&mut self) -> &mut [Mask] {
        self.mask_slice_mut(CHAR_MASK_COUNT + UNIT_MASK_COUNT, ATTR_MASK_COUNT)
    }

    #[inline(always)]
    pub(crate) fn set_power_values(&mut self, idx: u16, vals: [u16; 8]) {
        self.column_mut::<[u16; 8]>(self.layout.off_power_values)[idx as usize] = vals;
    }

    #[inline(always)]
    pub(crate) fn set_power_lut(&mut self, idx: u16, lut: u32) {
        self.column_mut::<u32>(self.layout.off_power_lut)[idx as usize] = lut;
    }

    #[inline(always)]
    pub(crate) fn set_skill(&mut self, idx: u16, info: SkillSlot) {
        self.column_mut::<SkillSlot>(self.layout.off_skill_table)[idx as usize] = info;
    }

    #[inline(always)]
    pub(crate) fn set_event_bonus_packed(&mut self, idx: u16, bonus: EventBonusHot) {
        self.column_mut::<EventBonusHot>(self.layout.off_event_bonus)[idx as usize] = bonus;
    }

    #[inline(always)]
    pub(crate) fn set_event_bonus(&mut self, idx: u16, bonus: EventBonusExact) {
        let limited_code = if bonus.limited_x10 == 0 {
            0
        } else if let Some(position) = self
            .special
            .limited_bonus_x10()
            .iter()
            .position(|value| *value == bonus.limited_x10)
        {
            (position + 1) as u8
        } else {
            assert!(
                self.special.limited_bonus_x10().len() < 15,
                "limited bonus side table exhausted"
            );
            self.special.push_limited_bonus(bonus.limited_x10);
            self.special.limited_bonus_x10().len() as u8
        };
        let total_x10 = bonus.base_x10 as u32 + bonus.limited_x10 as u32;
        assert!(
            total_x10 <= EventBonusHot::MAX_TOTAL_X10 as u32,
            "event bonus exceeds packed range"
        );
        self.set_event_bonus_packed(
            idx,
            EventBonusHot::from_parts(total_x10 as u16, limited_code),
        );
    }

    #[inline(always)]
    pub(crate) fn set_char_id(&mut self, idx: u16, char_id: u8) {
        self.column_mut::<u8>(self.layout.off_char_ids)[idx as usize] = char_id;
    }

    #[inline(always)]
    pub(crate) fn set_attr(&mut self, idx: u16, attr: u8) {
        self.column_mut::<u8>(self.layout.off_attrs)[idx as usize] = attr;
    }

    #[inline(always)]
    pub(crate) fn set_unit_mask(&mut self, idx: u16, mask: u8) {
        self.column_mut::<u8>(self.layout.off_unit_masks)[idx as usize] = mask;
    }

    #[inline(always)]
    pub(crate) fn set_game_id(&mut self, idx: u16, game_id: u16) {
        self.column_mut::<u16>(self.layout.off_game_ids)[idx as usize] = game_id;
    }

    #[inline(always)]
    pub(crate) fn set_power_max(&mut self, idx: u16, val: u32) {
        self.column_mut::<u32>(self.layout.off_power_max)[idx as usize] = val;
    }

    #[inline(always)]
    pub(crate) fn set_skill_min(&mut self, idx: u16, val: u8) {
        self.column_mut::<u8>(self.layout.off_skill_min)[idx as usize] = val;
    }

    #[inline(always)]
    pub(crate) fn set_skill_max(&mut self, idx: u16, val: u8) {
        self.column_mut::<u8>(self.layout.off_skill_max)[idx as usize] = val;
    }

    #[inline(always)]
    pub(crate) fn mark_char(&mut self, char_id: u8, card_idx: u16) {
        self.char_masks_mut()[char_id as usize].set(card_idx as usize);
    }

    #[inline(always)]
    pub(crate) fn mark_unit(&mut self, unit_id: u8, card_idx: u16) {
        self.unit_masks_mut()[unit_id as usize].set(card_idx as usize);
    }

    #[inline(always)]
    pub(crate) fn mark_attr(&mut self, attr_id: u8, card_idx: u16) {
        self.attr_masks_mut()[attr_id as usize].set(card_idx as usize);
    }

    #[inline(always)]
    pub(crate) fn add_unit_count_skill(&mut self, skill: UnitCountSkill) {
        assert!(
            self.special.unit_count().len() < u8::MAX as usize,
            "unit_count side table exhausted"
        );
        self.special.push_unit_count(skill);
    }

    #[inline(always)]
    pub(crate) fn add_diff_skill(&mut self, skill: DiffSkill) {
        assert!(
            self.special.diff().len() < u8::MAX as usize,
            "diff side table exhausted"
        );
        self.special.push_diff(skill);
    }

    #[inline(always)]
    pub(crate) fn add_ref_skill(&mut self, skill: RefSkill) {
        assert!(
            self.special.ref_skills().len() < u8::MAX as usize,
            "ref side table exhausted"
        );
        self.special.push_ref(skill);
    }

    #[inline(always)]
    pub(crate) fn add_limited_bonus(&mut self, value_x10: u16) {
        assert!(
            self.special.limited_bonus_x10().len() < 15,
            "limited bonus side table exhausted"
        );
        self.special.push_limited_bonus(value_x10);
    }

    /// 冻结为只读 `CardPool`。
    pub fn freeze(self) -> CardPool {
        CardPool::from_parts(self.arena, self.layout, self.count, self.special)
    }
}
