//! Shared vocabulary: identifiers, enums and per-card resolved values.
//!
//! Everything here is re-exported at the crate root, so these names are the ones
//! callers normally reach for: [`Unit`], [`Attr`], [`LiveType`], [`ScoreTarget`]
//! and the deck-shape constants such as [`DECK_SIZE`].

use serde::{Deserialize, Serialize};

/// Game card id, as it appears in masterdata and player data.
///
/// Distinct from [`crate::pool::CardIdx`], which is a dense index into one
/// [`crate::pool::CardPool`] and is only meaningful together with that pool.
pub type CardId = u16;

/// Cards in a deck.
pub const DECK_SIZE: usize = 5;

/// Upper clamp applied to a computed live score.
pub const SCORE_MAX: f64 = 10_000_000.0;

/// Event id of the World Bloom chapter 2 finale, the first live with finale rules.
pub const FINAL_CHAPTER_EVENT_ID: i32 = 180;

/// 模拟 WL3 终章的假活动 ID。
///
/// WL3 模拟终章的假活动 ID
/// （= `getWorldBloomFakeEventId(3, 0)` = `3000000 + 2 * 100000`）。
pub const WL3_FAKE_FINALE_EVENT_ID: i32 = 3_200_000;

/// 终章事件判定：legacy WL2 终章（180）与模拟 WL3 终章（3_200_000）。
///
/// 真实 masterdata 出现
/// 新终章活动前，模拟终章共享 180 的终章规则（队长限定 bonus、技能上限 140、
/// 加成卡上限 4、mysekai fixture 上限 20、禁用 best_skill_as_leader）。
#[inline]
pub const fn is_world_bloom_finale_event(event_id: i32) -> bool {
    event_id == FINAL_CHAPTER_EVENT_ID || event_id == WL3_FAKE_FINALE_EVENT_ID
}

/// A unit, plus the pseudo-units that only appear as skill effect targets.
///
/// Variants 1..=6 are the real in-game units. [`Unit::Any`], [`Unit::Ref`] and
/// [`Unit::Diff`] are not units a character belongs to: they are the `unit`
/// codes masterdata uses on skill effects to mean "any unit", "mirror another
/// member's score-up" and "scales with the number of distinct units in the
/// deck". They share this enum so one lookup table can be indexed by unit code.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Unit {
    /// No unit, used where a unit code is absent.
    None = 0,
    /// Leo/need.
    LightSound = 1,
    /// MORE MORE JUMP!.
    Idol = 2,
    /// Vivid BAD SQUAD.
    Street = 3,
    /// Wonderlands x Showtime.
    Themepark = 4,
    /// 25-ji, Nightcord de.
    SchoolRefusal = 5,
    /// VIRTUAL SINGER.
    Piapro = 6,
    /// Skill effect target: applies regardless of unit.
    Any = 7,
    /// Skill effect target: reference skill, mirrors another member's score-up.
    Ref = 8,
    /// Skill effect target: scales with the number of distinct units in the deck.
    Diff = 9,
}

/// Number of [`Unit`] variants, i.e. the width of unit-indexed lookup tables.
pub const UNIT_COUNT: usize = 10;

/// Card attribute.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Attr {
    /// No attribute, used where an attribute code is absent.
    Null = 0,
    /// Cool.
    Cool = 1,
    /// Cute.
    Cute = 2,
    /// Happy.
    Happy = 3,
    /// Pure.
    Pure = 4,
    /// Mysterious.
    Mysterious = 5,
}

/// Number of [`Attr`] variants, i.e. the width of attribute-indexed tables.
pub const ATTR_COUNT: usize = 6;

/// Live mode, which decides the scoring formula and the deck constraints.
///
/// [`LiveType::Challenge`] and [`LiveType::ChallengeAuto`] require all five
/// cards to be the same character; every other mode requires five distinct
/// characters.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum LiveType {
    /// Solo live.
    Solo = 0,
    /// Auto live.
    Auto = 1,
    /// Multiplayer live.
    Multi = 2,
    /// Cheerful Carnival, a multiplayer variant with its own score-up term.
    Cheerful = 3,
    /// Challenge live: five cards of one character.
    Challenge = 4,
    /// Auto challenge live: five cards of one character.
    ChallengeAuto = 5,
    /// MySekai live.
    Mysekai = 6,
}

/// Event type, which decides how event bonuses are computed.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventType {
    /// Marathon event.
    Marathon = 0,
    /// Cheerful Carnival event.
    CheerfulCarnival = 1,
    /// World Bloom event, which adds a support deck and a different-attribute bonus.
    WorldBloom = 2,
}

/// How to resolve a reference skill ([`Unit::Ref`]), whose value depends on the
/// other members' score-up.
///
/// The referenced value is not known until the rest of the deck is fixed, so the
/// caller chooses which end of the range to score against.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SkillReferenceStrategy {
    /// Score against the highest referenced value.
    Max = 0,
    /// Score against the lowest referenced value.
    Min = 1,
    /// Score against the mean referenced value.
    Average = 2,
}

/// Which order the five deck skills are assumed to fire in.
///
/// Skill slots are worth different amounts, so the assumed order changes the
/// score. [`LiveSkillOrder::Specific`] requires an explicit permutation.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum LiveSkillOrder {
    /// Assume the order that maximises the score.
    Best = 0,
    /// Assume the order that minimises the score.
    Worst = 1,
    /// Score against the mean over orders.
    Average = 2,
    /// Use a caller-supplied slot permutation.
    Specific = 3,
}

/// What the search maximises.
///
/// Accepted as the `target` parameter by [`crate::engine::recommend_json`], and
/// spelled there in lowercase.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ScoreTarget {
    /// Event points, or live score when there is no event.
    Score = 0,
    /// Total deck power.
    Power = 1,
    /// Total skill score-up.
    Skill = 2,
    /// Event bonus rate, used to hit an exact bonus tier.
    Bonus = 3,
    /// MySekai event points.
    Mysekai = 4,
}

/// Which artwork a card shows, which decides whether its after-training skill applies.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum DefaultImage {
    /// Untrained artwork.
    Original = 0,
    /// Special training artwork.
    SpecialTraining = 1,
}

/// One card's power, broken into its additive parts.
///
/// Every field is a final `i32`. The float truncation the game applies to each
/// bonus has already happened in the pool-building layer, so the search never
/// re-rounds these.
#[derive(Debug, Copy, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PowerDetail {
    /// Power from the card itself: level, master rank, episodes and training.
    pub base: i32,
    /// Power from area items.
    pub area_item_bonus: i32,
    /// Power from character rank.
    pub character_bonus: i32,
    /// Power from MySekai fixtures, already clamped to the event's limit.
    pub fixture_bonus: i32,
    /// Power from MySekai gates.
    pub gate_bonus: i32,
    /// Sum of the other fields.
    pub total: i32,
}

/// One card's skill, resolved for a particular deck composition.
///
/// A card contributes one `SkillInfo` per unit and member-count combination; the
/// pool-building layer precomputes them so leaf evaluation is a lookup.
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillInfo {
    /// Masterdata skill id.
    pub skill_id: i32,
    /// Whether this is the after-training skill rather than the base one.
    pub is_after_training: bool,
    /// Score-up percentage this skill contributes on its own.
    pub base_score_up: f64,
    /// Life recovered when the skill fires.
    pub life_recovery: f64,
    /// Whether this is a reference skill, whose value depends on other members.
    pub has_ref: bool,
    /// Fraction of the referenced member's score-up that is mirrored.
    pub ref_rate: f64,
    /// Upper clamp on the mirrored score-up.
    pub ref_max: f64,
}

impl Default for SkillInfo {
    fn default() -> Self {
        Self {
            skill_id: 0,
            is_after_training: false,
            base_score_up: 0.0,
            life_recovery: 0.0,
            has_ref: false,
            ref_rate: 0.0,
            ref_max: 0.0,
        }
    }
}

/// Overrides which unit a character counts as for unit-sensitive skills.
///
/// Characters who belong to two units pick one per live; this lets the caller
/// state that choice instead of leaving it to the default mapping.
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomSupportUnit {
    /// Game character id.
    pub character_id: i32,
    /// Unit the character counts as.
    pub unit: Unit,
}
