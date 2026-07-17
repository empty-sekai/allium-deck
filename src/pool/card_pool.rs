use std::slice;

use super::arena::Arena;
use super::builder::PoolBuilder;
use super::layout::PoolLayout;
use super::types::{
    CardIdx, EventBonusExact, EventBonusHot, Mask, SkillSlot, SpecialTables, ATTR_MASK_COUNT,
    CHAR_MASK_COUNT, UNIT_MASK_COUNT,
};

/// HPC SoA 卡池。
#[derive(Debug)]
pub struct CardPool {
    arena: Arena,
    layout: PoolLayout,
    count: u16,
    special: SpecialTables,
}

impl CardPool {
    pub(crate) fn from_parts(
        arena: Arena,
        layout: PoolLayout,
        count: u16,
        special: SpecialTables,
    ) -> Self {
        Self {
            arena,
            layout,
            count,
            special,
        }
    }

    #[inline(always)]
    fn column<T>(&self, offset: usize) -> &[T] {
        unsafe {
            slice::from_raw_parts(
                self.arena.as_ptr().add(offset) as *const T,
                self.count as usize,
            )
        }
    }

    #[inline(always)]
    fn masks(&self) -> &[Mask] {
        unsafe {
            slice::from_raw_parts(
                self.arena.as_ptr().add(self.layout.off_masks) as *const Mask,
                CHAR_MASK_COUNT + UNIT_MASK_COUNT + ATTR_MASK_COUNT,
            )
        }
    }

    #[inline(always)]
    fn char_masks(&self) -> &[Mask] {
        &self.masks()[..CHAR_MASK_COUNT]
    }

    #[inline(always)]
    fn unit_masks(&self) -> &[Mask] {
        &self.masks()[CHAR_MASK_COUNT..CHAR_MASK_COUNT + UNIT_MASK_COUNT]
    }

    #[inline(always)]
    fn attr_masks(&self) -> &[Mask] {
        &self.masks()
            [CHAR_MASK_COUNT + UNIT_MASK_COUNT..CHAR_MASK_COUNT + UNIT_MASK_COUNT + ATTR_MASK_COUNT]
    }

    /// 返回卡池中的候选卡数量。
    #[inline(always)]
    pub fn count(&self) -> usize {
        self.count as usize
    }

    /// 返回所有合法索引的迭代器。
    #[inline(always)]
    pub fn indices(&self) -> impl Iterator<Item = CardIdx> + '_ {
        (0..self.count).map(CardIdx::new)
    }

    /// 将外部传入的稠密索引转换为合法 `CardIdx`。
    #[inline(always)]
    pub fn card_idx(&self, dense_idx: u16) -> Option<CardIdx> {
        if dense_idx < self.count {
            Some(CardIdx::new(dense_idx))
        } else {
            None
        }
    }

    /// 返回编码后的 8 槽 power 表。
    #[inline(always)]
    pub fn power_values(&self, idx: CardIdx) -> &[u16; 8] {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            self.column::<[u16; 8]>(self.layout.off_power_values)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回对应卡的 power LUT。
    #[inline(always)]
    pub fn power_lut(&self, idx: CardIdx) -> u32 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u32>(self.layout.off_power_lut)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回技能主表槽位。
    #[inline(always)]
    pub fn skill(&self, idx: CardIdx) -> SkillSlot {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<SkillSlot>(self.layout.off_skill_table)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回活动加成热字段。
    #[inline(always)]
    pub fn event_bonus(&self, idx: CardIdx) -> &EventBonusHot {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            self.column::<EventBonusHot>(self.layout.off_event_bonus)
                .get_unchecked(idx.raw())
        }
    }

    #[inline(always)]
    pub fn event_bonus_exact(&self, idx: CardIdx) -> EventBonusExact {
        let hot = *self.event_bonus(idx);
        let limited_x10 = match hot.limited_code() {
            0 => 0,
            code => unsafe {
                *self
                    .special
                    .limited_bonus_x10()
                    .get_unchecked(code as usize - 1)
            },
        };
        EventBonusExact::from_x10(hot.total_x10() - limited_x10, limited_x10)
    }

    /// 返回角色 ID。
    #[inline(always)]
    pub fn char_id(&self, idx: CardIdx) -> u8 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u8>(self.layout.off_char_ids)
                .get_unchecked(idx.raw())
        }
    }

    #[inline(always)]
    pub(crate) fn char_ids(&self) -> &[u8] {
        self.column::<u8>(self.layout.off_char_ids)
    }

    /// 返回属性 ID。
    #[inline(always)]
    pub fn attr(&self, idx: CardIdx) -> u8 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u8>(self.layout.off_attrs)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回原始 unit bitmask。
    #[inline(always)]
    pub fn unit_mask_raw(&self, idx: CardIdx) -> u8 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u8>(self.layout.off_unit_masks)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回游戏 ID。
    #[inline(always)]
    pub fn game_id(&self, idx: CardIdx) -> u16 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u16>(self.layout.off_game_ids)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回综合力上界摘要。
    #[inline(always)]
    pub fn power_max(&self, idx: CardIdx) -> u32 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u32>(self.layout.off_power_max)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回技能下界摘要。
    #[inline(always)]
    pub fn skill_min(&self, idx: CardIdx) -> u8 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u8>(self.layout.off_skill_min)
                .get_unchecked(idx.raw())
        }
    }

    /// 返回技能上界摘要。
    #[inline(always)]
    pub fn skill_max(&self, idx: CardIdx) -> u8 {
        debug_assert!(idx.raw() < self.count());
        unsafe {
            *self
                .column::<u8>(self.layout.off_skill_max)
                .get_unchecked(idx.raw())
        }
    }

    /// 安全读取角色掩码。
    #[inline(always)]
    pub fn char_mask(&self, char_id: u8) -> Option<&Mask> {
        if (char_id as usize) < CHAR_MASK_COUNT {
            Some(unsafe { self.char_mask_unchecked(char_id) })
        } else {
            None
        }
    }

    /// 安全读取团属性掩码。
    #[inline(always)]
    pub fn unit_mask_at(&self, unit_id: u8) -> Option<&Mask> {
        if (unit_id as usize) < UNIT_MASK_COUNT {
            Some(unsafe { self.unit_mask_unchecked(unit_id) })
        } else {
            None
        }
    }

    /// 安全读取属性掩码。
    #[inline(always)]
    pub fn attr_mask(&self, attr_id: u8) -> Option<&Mask> {
        if (attr_id as usize) < ATTR_MASK_COUNT {
            Some(unsafe { self.attr_mask_unchecked(attr_id) })
        } else {
            None
        }
    }

    /// 返回特殊技能侧表。
    #[inline(always)]
    pub fn special(&self) -> &SpecialTables {
        &self.special
    }

    #[inline(always)]
    pub(crate) unsafe fn char_mask_unchecked(&self, char_id: u8) -> &Mask {
        debug_assert!((char_id as usize) < CHAR_MASK_COUNT);
        self.char_masks().get_unchecked(char_id as usize)
    }

    #[inline(always)]
    pub(crate) unsafe fn unit_mask_unchecked(&self, unit_id: u8) -> &Mask {
        debug_assert!((unit_id as usize) < UNIT_MASK_COUNT);
        self.unit_masks().get_unchecked(unit_id as usize)
    }

    #[inline(always)]
    pub(crate) unsafe fn attr_mask_unchecked(&self, attr_id: u8) -> &Mask {
        debug_assert!((attr_id as usize) < ATTR_MASK_COUNT);
        self.attr_masks().get_unchecked(attr_id as usize)
    }

    /// 根据保留位图重新打包一个紧凑卡池。
    pub fn compact(&self, keep: &[bool]) -> CardPool {
        assert_eq!(
            keep.len(),
            self.count(),
            "keep length must match pool count"
        );

        let retained = keep.iter().copied().filter(|flag| *flag).count();
        assert!(retained <= u16::MAX as usize, "compacted pool is too large");

        let mut builder = PoolBuilder::new(retained as u16);
        for skill in self.special().unit_count().iter().copied() {
            builder.add_unit_count_skill(skill);
        }
        for skill in self.special().diff().iter().copied() {
            builder.add_diff_skill(skill);
        }
        for skill in self.special().ref_skills().iter().copied() {
            builder.add_ref_skill(skill);
        }
        for value in self.special().limited_bonus_x10().iter().copied() {
            builder.add_limited_bonus(value);
        }

        let mut next_idx = 0u16;
        for (dense_idx, retain) in keep.iter().copied().enumerate() {
            if !retain {
                continue;
            }

            let src = CardIdx::new(dense_idx as u16);
            builder.set_power_values(next_idx, *self.power_values(src));
            builder.set_power_lut(next_idx, self.power_lut(src));
            builder.set_skill(next_idx, self.skill(src));
            builder.set_event_bonus_packed(next_idx, *self.event_bonus(src));
            builder.set_char_id(next_idx, self.char_id(src));
            builder.set_attr(next_idx, self.attr(src));
            builder.set_unit_mask(next_idx, self.unit_mask_raw(src));
            builder.set_game_id(next_idx, self.game_id(src));
            builder.set_power_max(next_idx, self.power_max(src));
            builder.set_skill_min(next_idx, self.skill_min(src));
            builder.set_skill_max(next_idx, self.skill_max(src));

            builder.mark_char(self.char_id(src), next_idx);
            let unit_mask = self.unit_mask_raw(src);
            for unit_id in 0..UNIT_MASK_COUNT {
                if unit_mask & (1u8 << unit_id) != 0 {
                    builder.mark_unit(unit_id as u8, next_idx);
                }
            }
            builder.mark_attr(self.attr(src), next_idx);
            next_idx += 1;
        }

        builder.freeze()
    }
}
