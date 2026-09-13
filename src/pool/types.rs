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
    /// Which side table `value` indexes, per the table above.
    pub skill_type: u8,
    /// Score-up percentage for type `0`, otherwise a 1-based side-table index
    /// where `0` means "no entry".
    pub value: u8,
}

const _: () = assert!(size_of::<SkillSlot>() == 2);

/// 热路径活动加成。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct EventBonusHot {
    /// 高 12 bit 为总加成（0.1%），低 4 bit 为限定加成查表 code。
    packed: u16,
}

const _: () = assert!(size_of::<EventBonusHot>() == 2);

impl EventBonusHot {
    /// Largest representable total bonus, in tenths of a percent.
    pub const MAX_TOTAL_X10: u16 = 0x0fff;

    /// Packs a total bonus and a limited-bonus code into one `u16`.
    ///
    /// # Panics
    ///
    /// Panics if `total_x10` exceeds [`Self::MAX_TOTAL_X10`] or `limited_code`
    /// does not fit in four bits.
    #[inline(always)]
    pub const fn from_parts(total_x10: u16, limited_code: u8) -> Self {
        assert!(total_x10 <= Self::MAX_TOTAL_X10);
        assert!(limited_code <= 0x0f);
        Self {
            packed: (total_x10 << 4) | limited_code as u16,
        }
    }

    /// Total bonus in tenths of a percent.
    #[inline(always)]
    pub const fn total_x10(self) -> u16 {
        self.packed >> 4
    }

    /// Index into [`SpecialTables::limited_bonus_x10`], 1-based; `0` means the
    /// card carries no limited bonus.
    #[inline(always)]
    pub const fn limited_code(self) -> u8 {
        (self.packed & 0x0f) as u8
    }

    /// Total bonus as whole percent, rounded up.
    #[inline(always)]
    pub const fn total_ceil(self) -> u32 {
        (self.total_x10() as u32).div_ceil(10)
    }

    /// Total bonus as a percentage.
    #[inline(always)]
    pub fn total_rate(self) -> f64 {
        self.total_x10() as f64 * 0.1
    }
}

/// 构建和结果阶段使用的精确活动加成分量。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventBonusExact {
    /// Bonus that every matching card contributes, in tenths of a percent.
    pub base_x10: u16,
    /// Bonus that only counts for the first few cards, in tenths of a percent.
    ///
    /// How many cards is capped per event; the cap is applied during evaluation,
    /// not here.
    pub limited_x10: u16,
}

impl EventBonusExact {
    /// Builds from parts already expressed in tenths of a percent.
    ///
    /// # Panics
    ///
    /// Panics if the two parts together exceed
    /// [`EventBonusHot::MAX_TOTAL_X10`].
    #[inline(always)]
    pub const fn from_x10(base_x10: u16, limited_x10: u16) -> Self {
        assert!(base_x10 as u32 + limited_x10 as u32 <= EventBonusHot::MAX_TOTAL_X10 as u32);
        Self {
            base_x10,
            limited_x10,
        }
    }

    /// Builds from parts expressed in whole percent.
    ///
    /// # Panics
    ///
    /// Panics if the two parts together exceed
    /// [`EventBonusHot::MAX_TOTAL_X10`].
    #[inline(always)]
    pub const fn from_whole(base: u16, limited: u16) -> Self {
        Self::from_x10(base * 10, limited * 10)
    }

    /// Base bonus in tenths of a percent, widened.
    #[inline(always)]
    pub const fn base_x10(self) -> u32 {
        self.base_x10 as u32
    }

    /// Limited bonus in tenths of a percent, widened.
    #[inline(always)]
    pub const fn limited_x10(self) -> u32 {
        self.limited_x10 as u32
    }

    /// Both parts summed, in tenths of a percent.
    #[inline(always)]
    pub const fn total_x10(self) -> u32 {
        self.base_x10() + self.limited_x10()
    }

    /// Both parts summed, as whole percent rounded up.
    #[inline(always)]
    pub const fn total_ceil(self) -> u32 {
        self.total_x10().div_ceil(10)
    }

    /// Base bonus as whole percent, rounded up.
    #[inline(always)]
    pub const fn base_ceil(self) -> u32 {
        self.base_x10().div_ceil(10)
    }

    /// Limited bonus as whole percent, rounded up.
    #[inline(always)]
    pub const fn limited_ceil(self) -> u32 {
        self.limited_x10().div_ceil(10)
    }

    /// Base bonus as a percentage.
    #[inline(always)]
    pub fn base_rate(self) -> f64 {
        self.base_x10() as f64 * 0.1
    }

    /// Limited bonus as a percentage.
    #[inline(always)]
    pub fn limited_rate(self) -> f64 {
        self.limited_x10() as f64 * 0.1
    }

    /// Both parts summed, as a percentage.
    #[inline(always)]
    pub fn total_rate(self) -> f64 {
        self.total_x10() as f64 * 0.1
    }
}

/// 组分技能侧表项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct UnitCountSkill {
    /// Unit code the skill counts members of, as a [`crate::types::Unit`] discriminant.
    pub unit: u8,
    /// Score-up percentage for one through five matching members.
    pub score_up: [u8; 5],
}

const _: () = assert!(size_of::<UnitCountSkill>() == 6);

/// 异团技能侧表项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct DiffSkill {
    /// Score-up percentage before any per-unit increment.
    pub base: u8,
    /// Added to `base` for each additional distinct unit in the deck.
    pub increment: u8,
}

const _: () = assert!(size_of::<DiffSkill>() == 2);

/// 吸分类技能侧表项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct RefSkill {
    /// Percentage of the referenced member's score-up that is mirrored.
    pub rate: u8,
    /// Upper clamp on the mirrored score-up, in percent.
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
    limited_bonus_x10: Vec<u16>,
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

    /// 返回限定加成 code 表；code 1 对应索引 0。
    pub fn limited_bonus_x10(&self) -> &[u16] {
        &self.limited_bonus_x10
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

    #[inline(always)]
    pub(crate) fn push_limited_bonus(&mut self, value_x10: u16) {
        self.limited_bonus_x10.push(value_x10);
    }
}
