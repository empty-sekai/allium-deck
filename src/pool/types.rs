use std::mem::size_of;

pub(crate) const CHAR_MASK_COUNT: usize = 27;
pub(crate) const UNIT_MASK_COUNT: usize = 6;
pub(crate) const ATTR_MASK_COUNT: usize = 5;
pub(crate) const MASK_BITS: usize = MASK_WORDS * 64;

/// 稠密卡索引。
///
/// 该类型只能在 `pool` 模块内部构造，外部调用方只能通过 `CardPool`
/// 提供的安全接口获得合法索引。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct CardIdx(u16);

impl CardIdx {
    #[inline(always)]
    pub(crate) const fn new(raw: u16) -> Self {
        Self(raw)
    }

    /// 返回原始稠密索引。
    #[inline(always)]
    pub const fn raw(self) -> usize {
        self.0 as usize
    }
}

const _: () = assert!(size_of::<CardIdx>() == 2);

/// 主技能槽位。
///
/// `skill_type` 约定：
/// - `0`：普通技能，仅使用 `value`
/// - `1`：组分技能，`value` 为 `SpecialTables::unit_count()` 的 1-based 索引
/// - `2`：异团技能，`value` 为 `SpecialTables::diff()` 的 1-based 索引
/// - `3`：吸分技能，`value` 为 `SpecialTables::ref_skills()` 的 1-based 索引
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct SkillSlot {
    pub skill_type: u8,
    pub value: u8,
}

const _: () = assert!(size_of::<SkillSlot>() == 2);

/// 热路径活动加成。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct EventBonusHot {
    pub base_bonus: u8,
    pub limited_bonus: u8,
}

const _: () = assert!(size_of::<EventBonusHot>() == 2);

/// 组分技能侧表项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct UnitCountSkill {
    pub unit: u8,
    pub score_up: [u8; 5],
}

const _: () = assert!(size_of::<UnitCountSkill>() == 6);

/// 异团技能侧表项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct DiffSkill {
    pub base: u8,
    pub increment: u8,
}

const _: () = assert!(size_of::<DiffSkill>() == 2);

/// 吸分类技能侧表项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct RefSkill {
    pub rate: u8,
    pub max: u8,
}

const _: () = assert!(size_of::<RefSkill>() == 2);

/// 单个掩码的机器字数量。
pub const MASK_WORDS: usize = 8;

/// 512-bit 候选掩码。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C, align(64))]
pub struct Mask([u64; MASK_WORDS]);

impl Mask {
    /// 全零掩码。
    pub const EMPTY: Self = Self([0; MASK_WORDS]);

    /// 置位指定 bit。
    #[inline(always)]
    pub fn set(&mut self, bit: usize) {
        debug_assert!(bit < MASK_BITS, "mask bit out of range");
        let word = unsafe { self.0.get_unchecked_mut(bit >> 6) };
        *word |= 1u64 << (bit & 63);
    }

    /// 测试指定 bit 是否已置位。
    #[inline(always)]
    pub fn test(&self, bit: usize) -> bool {
        debug_assert!(bit < MASK_BITS, "mask bit out of range");
        let word = unsafe { *self.0.get_unchecked(bit >> 6) };
        word & (1u64 << (bit & 63)) != 0
    }

    /// 返回按位与结果。
    #[inline(always)]
    pub fn and(&self, other: &Mask) -> Mask {
        let mut result = Self::EMPTY;
        for idx in 0..MASK_WORDS {
            unsafe {
                *result.0.get_unchecked_mut(idx) =
                    *self.0.get_unchecked(idx) & *other.0.get_unchecked(idx);
            }
        }
        result
    }

    /// 判断掩码是否全零。
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|word| *word == 0)
    }

    /// 返回置位 bit 的数量。
    #[inline(always)]
    pub fn count_ones(&self) -> u32 {
        self.0.iter().map(|word| word.count_ones()).sum()
    }

    /// 返回最低置位 bit 的位置。
    #[inline(always)]
    pub fn lowest_set_bit(&self) -> Option<usize> {
        for (word_idx, word) in self.0.iter().copied().enumerate() {
            if word != 0 {
                return Some((word_idx << 6) | word.trailing_zeros() as usize);
            }
        }
        None
    }

    /// 清除最低置位 bit。
    #[inline(always)]
    pub fn clear_lowest(&mut self) {
        for word in &mut self.0 {
            if *word != 0 {
                *word &= *word - 1;
                return;
            }
        }
    }
}

const _: () = assert!(size_of::<Mask>() == 64);

/// 特殊技能侧表集合。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecialTables {
    unit_count: Vec<UnitCountSkill>,
    diff: Vec<DiffSkill>,
    ref_skills: Vec<RefSkill>,
}

impl SpecialTables {
    /// 返回组分技能侧表。
    pub fn unit_count(&self) -> &[UnitCountSkill] {
        &self.unit_count
    }

    /// 返回异团技能侧表。
    pub fn diff(&self) -> &[DiffSkill] {
        &self.diff
    }

    /// 返回吸分类技能侧表。
    pub fn ref_skills(&self) -> &[RefSkill] {
        &self.ref_skills
    }

    #[inline(always)]
    pub(crate) fn push_unit_count(&mut self, skill: UnitCountSkill) {
        self.unit_count.push(skill);
    }

    #[inline(always)]
    pub(crate) fn push_diff(&mut self, skill: DiffSkill) {
        self.diff.push(skill);
    }

    #[inline(always)]
    pub(crate) fn push_ref(&mut self, skill: RefSkill) {
        self.ref_skills.push(skill);
    }
}
