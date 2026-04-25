use serde::{Deserialize, Serialize};

pub type CardId = u16;
pub const DECK_SIZE: usize = 5;
pub const SCORE_MAX: f64 = 10_000_000.0;
pub const FINAL_CHAPTER_EVENT_ID: i32 = 180;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Unit {
    None = 0,
    LightSound = 1,
    Idol = 2,
    Street = 3,
    Themepark = 4,
    SchoolRefusal = 5,
    Piapro = 6,
    Any = 7,
    Ref = 8,
    Diff = 9,
}
pub const UNIT_COUNT: usize = 10;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Attr {
    Null = 0,
    Cool = 1,
    Cute = 2,
    Happy = 3,
    Pure = 4,
    Mysterious = 5,
}
pub const ATTR_COUNT: usize = 6;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum LiveType {
    Solo = 0,
    Auto = 1,
    Multi = 2,
    Cheerful = 3,
    Challenge = 4,
    ChallengeAuto = 5,
    Mysekai = 6,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventType {
    Marathon = 0,
    CheerfulCarnival = 1,
    WorldBloom = 2,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SkillReferenceStrategy {
    Max = 0,
    Min = 1,
    Average = 2,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum LiveSkillOrder {
    Best = 0,
    Worst = 1,
    Average = 2,
    Specific = 3,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ScoreTarget {
    Score = 0,
    Power = 1,
    Skill = 2,
    Mysekai = 4,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum DefaultImage {
    Original = 0,
    SpecialTraining = 1,
}

#[derive(Debug, Copy, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PowerDetail {
    pub base: i32,
    pub area_item_bonus: i32,
    pub character_bonus: i32,
    pub fixture_bonus: i32,
    pub gate_bonus: i32,
    pub total: i32,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillInfo {
    pub skill_id: i32,
    pub is_after_training: bool,
    pub base_score_up: f64,
    pub life_recovery: f64,
    pub has_ref: bool,
    pub ref_rate: f64,
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

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardEventBonus {
    pub base_bonus: f64,
    pub limited_bonus: f64,
    pub leader_honor_bonus: f64,
    pub leader_limit_bonus: f64,
}

impl Default for CardEventBonus {
    fn default() -> Self {
        Self {
            base_bonus: 0.0,
            limited_bonus: 0.0,
            leader_honor_bonus: 0.0,
            leader_limit_bonus: 0.0,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
/// 综合力查找表。handler 层 build_card_pool 时预构建。
///
/// ## resolved 槽位编码（member_key）
///
/// resolved[unit as usize][member_key(unit_member, attr_member)]
/// - [0] = 混组混色 (unit_member < 5, attr_member < 5)
/// - [1] = 混组同色 (unit_member < 5, attr_member == 5)
/// - [2] = 同组混色 (unit_member == 5, attr_member < 5)
/// - [3] = 同组同色 (unit_member == 5, attr_member == 5)
///
/// handler 必须为每张卡的每个 unit 预计算这 4 种组合的 PowerDetail。
/// build_card_pool 时执行完整 fallback 链（exact -> normalize -> Any），运行时无 Option。
///
/// ## diff 槽位
///
/// diff[index] 对应 diff 技能的不同组合数情况。
/// index = (unit_kind_count - 1).clamp(0, 2)
/// handler 预计算 diff unit 在 0/1/2 三种组合数下的 PowerDetail。
///
/// ## f32 精度截断（handler 职责）
///
/// 以下分项在 handler 计算时必须使用 f32 精度（对应游戏客户端行为）：
/// - characterBonus：`(rate as f32 * 0.01_f32 * basePower as f32).floor() as i32`
/// - areaItemBonus：TS 用 Math.fround 逐项累加，建议跟随 TS（游戏客户端精度）
/// - fixtureBonus / gateBonus：TS 用 Math.fround，建议跟随 TS
///
/// evaluator 层读取的 PowerDetail 全部是 i32 最终值，不做浮点截断。
pub struct PowerLookup {
    pub resolved: [[PowerDetail; 4]; UNIT_COUNT],
    pub diff: [PowerDetail; 3],
}

impl Default for PowerLookup {
    fn default() -> Self {
        Self {
            resolved: [[PowerDetail::default(); 4]; UNIT_COUNT],
            diff: [PowerDetail::default(); 3],
        }
    }
}

/// 技能查找表。按 [unit][skill_key] 索引。
/// skill_key: 0 = 非全同组(unit_member < 5), 1 = 全同组(unit_member == 5)
/// 注：技能不区分属性维度（attr_member 始终为 1），与 PowerLookup 的 4-slot 设计不同。
///
/// ## 组分技能精度说明
///
/// 组分技能（score_up_unit_count）效果与同组人数线性相关（1-5人效果不同），
/// 但 skill_key 只区分"全5人同组"和"非5人"。2/3/4人效果被合并到 index 0。
///
/// handler 预解析策略：
/// - index 0（非全同组）：存储 unit_member=1 时的值（最保守估计）
/// - index 1（全同组）：存储 unit_member=5 时的值
///
/// 这意味着当实际 deck 有 2/3/4 张同组卡时，组分技能效果被低估。
/// 这是有意的保守估计：搜索层找到的"最优解"不会因为精度偏高而无效。
/// 如果未来需要精确组分技能，可扩展 skill_key 为 [SkillInfo; 6]（0-5人），
/// 代价是每张候选卡增加约 4 * UNIT_COUNT * sizeof(SkillInfo)。
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillLookup {
    pub resolved: [[SkillInfo; 2]; UNIT_COUNT],
    pub diff: [SkillInfo; 3],
}

impl Default for SkillLookup {
    fn default() -> Self {
        Self {
            resolved: [[SkillInfo::default(); 2]; UNIT_COUNT],
            diff: [SkillInfo::default(); 3],
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupportDeckCard {
    pub card_id: CardId,
    pub bonus: f64,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct DifferentAttributeBonus {
    pub attribute_count: i32,
    pub bonus_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinalChapterSupportDeck {
    pub leader_character_id: i32,
    pub cards: Vec<SupportDeckCard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldBloomContext {
    pub support_deck_count: usize,
    pub diff_attr_bonus_table: [f64; ATTR_COUNT],
    pub support_cards: Vec<SupportDeckCard>,
    pub final_chapter_support: Vec<FinalChapterSupportDeck>,
    pub power_total_cap: Option<i32>,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomSupportUnit {
    pub character_id: i32,
    pub unit: Unit,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomBonusParams {
    pub character_mask: u32,
    pub attr: Option<Attr>,
    pub support_unit_by_char: [Unit; 27],
}

/// 活动上下文。
///
/// handler 职责（evaluator 不感知）：
/// - fixture_bonus 上限：handler 在构建 PowerLookup 时对 PowerDetail.fixture_bonus
///   执行 .min(limit)，evaluator 读取的值已是 clamp 后的。
/// - WL3 fake event 注入：handler 在构建 CardEventBonus 和 support_cards 时完成。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventContext {
    pub event_id: i32,
    pub event_type: EventType,
    pub boost_rate: f64,
    pub other_score: Option<i32>,
    pub life: i32,
    pub custom_bonus: Option<CustomBonusParams>,
    pub world_bloom: Option<WorldBloomContext>,
    pub skill_score_up_limit: Option<f64>,
    /// 活动卡加成计入上限。handler 从 masterdata eventCardBonusLimits.memberCountLimit 读取。
    /// 终章最多 4 张卡享受 limited_bonus，第 5 张扣除；非终章通常为 DECK_SIZE。
    /// 必须 > 0 且 <= DECK_SIZE。
    pub card_bonus_count_limit: usize,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicParams {
    pub event_rate: f64,
    pub base_score: f64,
    pub base_score_auto: f64,
    pub fever_score: f64,
    pub skill_scores: [[f64; 6]; 3],
    pub music_time: f64,
    pub tap_count: i32,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeckScore {
    pub card_ids: [CardId; DECK_SIZE],
    pub card_event_bonus_rates: [f64; DECK_SIZE],
    pub card_skill_score_up: [f64; DECK_SIZE],
    pub card_skill_life_recovery: [f64; DECK_SIZE],
    pub card_power_total: [i32; DECK_SIZE],
    pub total_power: i32,
    pub base_power: i32,
    pub area_item_bonus_power: i32,
    pub character_bonus_power: i32,
    pub honor_bonus_power: i32,
    pub fixture_bonus_power: i32,
    pub gate_bonus_power: i32,
    pub event_bonus_rate: f64,
    pub support_deck_bonus_rate: f64,
    pub diff_attr_bonus_rate: f64,
    pub multi_live_score_up: f64,
    pub live_score: i32,
    pub event_point: i32,
    pub mysekai_event_point: i32,
    pub mysekai_internal_point: i32,
    pub target_value: f64,
    pub chosen_mask: u32,
}

#[derive(Debug, Clone)]
pub struct DeckContext<'a> {
    pub pool: &'a crate::pool::CardPool,
    pub honor_bonus: i32,
    pub music: MusicParams,
    pub live_type: LiveType,
    pub target: ScoreTarget,
    pub event: Option<EventContext>,
    pub skill_reference_strategy: SkillReferenceStrategy,
    pub keep_after_training_state: bool,
    pub best_skill_as_leader: bool,
    pub live_skill_order: LiveSkillOrder,
    pub specific_skill_order: Option<[usize; DECK_SIZE]>,
    pub multi_teammate_score_up: Option<i32>,
    pub multi_teammate_power: Option<i32>,
    pub effective_live_type: LiveType,
    pub is_final_chapter: bool,
    pub effective_best_skill_as_leader: bool,
    pub is_mysekai: bool,
}

#[derive(Debug, Clone)]
pub struct DeckContextParams {
    pub honor_bonus: i32,
    pub music: MusicParams,
    pub live_type: LiveType,
    pub target: ScoreTarget,
    pub event: Option<EventContext>,
    pub skill_reference_strategy: SkillReferenceStrategy,
    pub keep_after_training_state: bool,
    pub best_skill_as_leader: bool,
    pub live_skill_order: LiveSkillOrder,
    pub specific_skill_order: Option<[usize; DECK_SIZE]>,
    pub multi_teammate_score_up: Option<i32>,
    pub multi_teammate_power: Option<i32>,
}

impl<'a> DeckContext<'a> {
    pub fn new(pool: &'a crate::pool::CardPool, params: DeckContextParams) -> Result<Self, String> {
        validate_pool(pool)?;
        validate_params(&params)?;

        let is_final_chapter = params
            .event
            .as_ref()
            .is_some_and(|event| event.event_id == FINAL_CHAPTER_EVENT_ID);
        let effective_live_type = if matches!(params.live_type, LiveType::Multi)
            && params
                .event
                .as_ref()
                .is_some_and(|event| matches!(event.event_type, EventType::CheerfulCarnival))
        {
            LiveType::Cheerful
        } else {
            params.live_type
        };
        let is_mysekai = matches!(params.target, ScoreTarget::Mysekai)
            || matches!(effective_live_type, LiveType::Mysekai);
        let effective_best_skill_as_leader = params.best_skill_as_leader && !is_final_chapter;

        Ok(Self {
            pool,
            honor_bonus: params.honor_bonus,
            music: params.music,
            live_type: params.live_type,
            target: params.target,
            event: params.event,
            skill_reference_strategy: params.skill_reference_strategy,
            keep_after_training_state: params.keep_after_training_state,
            best_skill_as_leader: params.best_skill_as_leader,
            live_skill_order: params.live_skill_order,
            specific_skill_order: params.specific_skill_order,
            multi_teammate_score_up: params.multi_teammate_score_up,
            multi_teammate_power: params.multi_teammate_power,
            effective_live_type,
            is_final_chapter,
            effective_best_skill_as_leader,
            is_mysekai,
        })
    }
}

fn validate_pool(pool: &crate::pool::CardPool) -> Result<(), String> {
    let count = pool.count();
    if count > u16::MAX as usize {
        return Err("card pool is too large for CardId".to_string());
    }
    if count > 512 {
        return Err("card pool exceeds 512-bit mask capacity".to_string());
    }
    Ok(())
}

fn validate_params(params: &DeckContextParams) -> Result<(), String> {
    if matches!(params.live_skill_order, LiveSkillOrder::Specific) {
        let order = params
            .specific_skill_order
            .ok_or_else(|| "specific_skill_order is required".to_string())?;
        let mut seen = [false; DECK_SIZE];
        for index in order {
            if index >= DECK_SIZE {
                return Err("specific_skill_order index out of range".to_string());
            }
            if seen[index] {
                return Err("specific_skill_order contains duplicate index".to_string());
            }
            seen[index] = true;
        }
    }
    if let Some(event) = &params.event {
        if event.card_bonus_count_limit == 0 {
            return Err("card_bonus_count_limit must be > 0".to_string());
        }
        if event.card_bonus_count_limit > DECK_SIZE {
            return Err("card_bonus_count_limit exceeds DECK_SIZE".to_string());
        }
        if matches!(event.event_type, EventType::WorldBloom) && event.world_bloom.is_none() {
            return Err("WorldBloom event requires world_bloom context".to_string());
        }
        if !matches!(event.event_type, EventType::WorldBloom) && event.world_bloom.is_some() {
            return Err("world_bloom context is only valid for WorldBloom".to_string());
        }
    }
    Ok(())
}
