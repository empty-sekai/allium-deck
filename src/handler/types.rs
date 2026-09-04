use serde::{Deserialize, Serialize};

use crate::types::{
    Attr, DefaultImage, EventType, LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy,
    Unit,
};

pub const MAX_BUILD_LIMIT: usize = 100;
pub const MAX_BUILD_TIMEOUT_MS: u64 = 300_000;
pub const MAX_TARGET_BONUS_BUCKETS: usize = 32;
pub const MAX_TARGET_BONUS: i32 = 10_000;

/// Handler 读取的 masterdata 视图。
#[derive(Debug, Clone, Copy)]
pub struct GameData<'a> {
    /// 卡牌主表。
    pub cards: &'a [MasterCard],
    /// 卡牌等级参数表。
    pub card_parameters: &'a [CardParameter],
    /// 稀有度上限表。
    pub card_rarities: &'a [CardRarity],
    /// 剧情加成表。
    pub card_episodes: &'a [CardEpisode],
    /// 突破加成表。
    pub master_lessons: &'a [MasterLesson],
    /// 技能主表。
    pub skills: &'a [Skill],
    /// 技能效果表。
    pub skill_effects: &'a [SkillEffect],
    /// 区域道具倍率表。
    pub area_item_levels: &'a [AreaItemLevel],
    /// 角色所属团表。
    pub game_character_units: &'a [GameCharacterUnit],
    /// 角色 rank 倍率表。
    pub character_ranks: &'a [CharacterRank],
    /// MySekai 画布加成表。
    pub card_mysekai_canvas_bonuses: &'a [CardMysekaiCanvasBonus],
    /// MySekai gate 主表。
    pub mysekai_gates: &'a [MysekaiGate],
    /// MySekai gate 等级表。
    pub mysekai_gate_levels: &'a [MysekaiGateLevel],
    /// 活动主表。
    pub events: &'a [Event],
    /// 活动卡加成表。
    pub event_cards: &'a [EventCard],
    /// 活动 deck bonus 表。
    pub event_deck_bonuses: &'a [EventDeckBonus],
    /// 活动卡 bonus 张数上限表。
    pub event_card_bonus_limits: &'a [EventCardBonusLimit],
    /// 终章称号加成表。
    pub event_honor_bonuses: &'a [EventHonorBonus],
    /// World Bloom 异色加成表。
    pub world_bloom_different_attribute_bonuses: &'a [WorldBloomDiffAttrBonus],
    /// World Bloom 章节表。
    pub world_blooms: &'a [WorldBloom],
    /// WL1 支援 deck bonus 表。
    pub wb_support_deck_bonuses_wl1: &'a [WBSupportDeckBonus],
    /// WL2 支援 deck bonus 表。
    pub wb_support_deck_bonuses_wl2: &'a [WBSupportDeckBonus],
    /// WL3 支援 deck bonus 表。
    pub wb_support_deck_bonuses_wl3: &'a [WBSupportDeckBonus],
    /// World Bloom 限定支援加成表。
    pub world_bloom_support_deck_unit_event_limited_bonuses:
        &'a [WBSupportDeckUnitEventLimitedBonus],
    /// 活动 MySekai fixture 上限表。
    pub event_mysekai_fixture_performance_bonus_limits: &'a [EventFixtureBonusLimit],
    /// 活动技能上限表。
    pub event_skill_score_up_limits: &'a [EventSkillScoreUpLimit],
    /// 歌曲元数据表。
    pub music_metas: &'a [MusicMeta],
    /// 歌曲难度表。
    pub music_difficulties: &'a [MusicDifficulty],
    /// 活动稀有度 bonus 表。
    pub event_rarity_bonus_rates: &'a [EventRarityBonusRate],
    /// 称号主表。
    pub honors: &'a [Honor],
    /// 羁绊称号主表。
    pub bonds_honors: &'a [BondsHonor],
}

/// Handler 使用的最小化用户数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UserProfile {
    /// 用户拥有的卡。
    pub user_cards: Vec<UserCard>,
    /// 用户角色 rank。
    pub user_characters: Vec<UserCharacter>,
    /// 用户区域道具等级。
    pub user_area_items: Vec<UserAreaItem>,
    /// 用户编组。
    pub user_decks: Vec<UserDeck>,
    /// 用户 World Bloom 支援编组。
    pub user_world_bloom_support_decks: Vec<UserWBSupportDeck>,
    /// 用户 Challenge 编组。
    pub user_challenge_live_solo_decks: Vec<UserChallengeDeck>,
    /// 用户 MySekai fixture bonus。
    pub user_mysekai_fixture_bonuses: Vec<UserFixtureBonus>,
    /// 用户 MySekai gate bonus。
    pub user_mysekai_gate_bonuses: Vec<UserGateBonus>,
    /// 用户拥有画布 bonus 的卡。
    pub user_mysekai_canvas_bonus_cards: Vec<i32>,
    /// 用户称号。
    pub user_honors: Vec<UserHonor>,
}

/// 构建卡池所需的 handler 参数。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildParams {
    /// 区服。
    pub region: String,
    /// 活动 ID。
    pub event_id: Option<i32>,
    /// 模拟活动类型。
    pub event_type: Option<String>,
    /// Live 类型。
    pub live_type: LiveType,
    /// 搜索目标。
    pub target: ScoreTarget,
    /// 每个目标返回的卡组数量。
    pub limit: usize,
    /// 兼容调用方的成员数；核心仅支持 None/5。
    pub member: Option<usize>,
    /// 搜索超时毫秒数，最大 300 秒。
    pub timeout_ms: u64,
    /// Bonus 目标下需要精确命中的活动加成档位。
    pub target_bonus_list: Vec<i32>,
    /// 歌曲 ID。
    pub music_id: Option<i32>,
    /// 歌曲难度。
    pub music_diff: Option<String>,
    /// 稀有度默认配置。
    pub card_configs: CardConfigSet,
    /// 单卡覆盖配置。
    pub single_card_configs: Vec<SingleCardConfig>,
    /// 固定卡。
    pub fixed_cards: Vec<i32>,
    /// 固定角色。
    pub fixed_characters: Vec<i32>,
    /// 终章指定队长角色；非终章忽略。
    pub forced_leader_character_id: Option<i32>,
    /// WL 角色 ID。
    pub world_bloom_character_id: Option<i32>,
    /// WL 回合。
    pub world_bloom_event_turn: Option<i32>,
    /// 模拟 WL 终章回合；2 走 legacy 终章（180），3 合成模拟终章（3_200_000）。
    ///
    /// 模拟 WL 终章回合；2 走 legacy 终章（180），3 合成模拟终章（3_200_000）。
    pub world_bloom_finale_turn: Option<i32>,
    /// Challenge 角色 ID。
    pub challenge_live_character_id: Option<i32>,
    /// 模拟活动团。
    pub event_unit: Option<String>,
    /// 模拟活动属性。
    pub event_attr: Option<String>,
    /// 自定义混活角色集合；优先于 event_unit 展开的整团集合。
    pub custom_bonus_character_ids: Vec<i32>,
    /// 自定义混活属性。
    pub custom_bonus_attr: Option<String>,
    /// 自定义混活中 VS 角色对应的支援团约束。
    pub custom_bonus_character_support_units: Vec<crate::types::CustomSupportUnit>,
    /// 是否过滤其他团员。
    pub filter_other_unit: bool,
    /// 是否按满破计算 World Bloom 支援卡。
    pub support_master_max: bool,
    /// 是否按满技能计算 World Bloom 支援卡。
    pub support_skill_max: bool,
    /// 吸分参考策略。
    pub skill_reference_strategy: SkillReferenceStrategy,
    /// 是否保持特训前后状态。
    pub keep_after_training_state: bool,
    /// 是否让最高技能作 leader。
    pub best_skill_as_leader: bool,
    /// Live 技能顺序。
    pub live_skill_order: LiveSkillOrder,
    /// 指定技能顺序。
    pub specific_skill_order: Option<[usize; 5]>,
    /// 协力队友 score up。
    pub multi_teammate_score_up: Option<i32>,
    /// 协力队友综合力。
    pub multi_teammate_power: Option<i32>,
    /// 协力总 score up 下限。
    pub multi_live_score_up_lower_bound: Option<f64>,
    /// 排除卡。
    pub excluded_cards: Vec<i32>,
    /// 展示 boost。
    pub boost: Option<i32>,
    /// 协力对手分数。
    pub other_score: Option<i32>,
    /// Cheerful 体力。
    pub life: Option<i32>,
    /// 团过滤。
    pub unit_filter: Option<String>,
    /// 属性过滤。
    pub attr_filter: Option<String>,
    /// 反向搜索：求最弱（最小化 power）而非最强。仅 Power 目标生效，其它目标忽略。
    pub minimize: bool,
}

impl Default for BuildParams {
    fn default() -> Self {
        Self {
            region: "cn".to_string(),
            event_id: None,
            event_type: None,
            live_type: LiveType::Solo,
            target: ScoreTarget::Score,
            limit: 10,
            member: None,
            timeout_ms: MAX_BUILD_TIMEOUT_MS,
            target_bonus_list: Vec::new(),
            music_id: None,
            music_diff: None,
            card_configs: CardConfigSet::default(),
            single_card_configs: Vec::new(),
            fixed_cards: Vec::new(),
            fixed_characters: Vec::new(),
            forced_leader_character_id: None,
            world_bloom_character_id: None,
            world_bloom_event_turn: None,
            world_bloom_finale_turn: None,
            challenge_live_character_id: None,
            event_unit: None,
            event_attr: None,
            custom_bonus_character_ids: Vec::new(),
            custom_bonus_attr: None,
            custom_bonus_character_support_units: Vec::new(),
            filter_other_unit: false,
            support_master_max: false,
            support_skill_max: false,
            skill_reference_strategy: SkillReferenceStrategy::Average,
            keep_after_training_state: false,
            best_skill_as_leader: true,
            live_skill_order: LiveSkillOrder::Best,
            specific_skill_order: None,
            multi_teammate_score_up: None,
            multi_teammate_power: None,
            multi_live_score_up_lower_bound: None,
            excluded_cards: Vec::new(),
            boost: None,
            other_score: None,
            life: None,
            unit_filter: None,
            attr_filter: None,
            minimize: false,
        }
    }
}

/// 稀有度默认配置集合。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CardConfigSet {
    /// 1 星配置。
    pub rarity_1_config: CardRarityConfig,
    /// 2 星配置。
    pub rarity_2_config: CardRarityConfig,
    /// 3 星配置。
    pub rarity_3_config: CardRarityConfig,
    /// 4 星配置。
    pub rarity_4_config: CardRarityConfig,
    /// 生日卡配置。
    pub rarity_birthday_config: CardRarityConfig,
    /// 内联的单卡覆盖。
    pub single_card_configs: Vec<SingleCardConfig>,
}

/// 单个稀有度 preset。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CardRarityConfig {
    /// 是否禁用该类卡。
    pub disable: bool,
    /// 是否视为满级。
    pub level_max: bool,
    /// 指定等级；优先于 `level_max`。
    pub level: Option<i32>,
    /// 是否视为满技能。
    pub skill_max: bool,
    /// 指定技能等级；优先于 `skill_max`。
    pub skill_level: Option<i32>,
    /// 是否视为剧情已读。
    pub episode_read: bool,
    /// 指定已读剧情数量；优先于 `episode_read`。
    pub episode_read_count: Option<i32>,
    /// 是否视为满破。
    pub master_max: bool,
    /// 指定 master rank；优先于 `master_max`。
    pub master_rank: Option<i32>,
    /// 是否启用画布。
    pub canvas: bool,
}

/// 单卡覆盖配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SingleCardConfig {
    /// 卡 ID。
    pub card_id: i32,
    /// 覆盖的配置。
    pub config: CardRarityConfig,
}

/// 卡牌主表最小字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MasterCard {
    /// 卡 ID。
    pub id: i32,
    /// 角色 ID。
    pub character_id: i32,
    /// 属性代码。
    pub attr: String,
    /// 稀有度类型。
    pub card_rarity_type: i32,
    /// 稀有度类型代码（原始字符串，如 "rarity_4"）。
    pub rarity: String,
    /// 资源包名。
    pub asset_bundle_name: String,
    /// 技能 ID。
    pub skill_id: i32,
    /// 特训后技能 ID。
    pub special_training_skill_id: Option<i32>,
    /// 特训后维度 1 固定加成。
    pub special_training_power1_bonus_fixed: i32,
    /// 特训后维度 2 固定加成。
    pub special_training_power2_bonus_fixed: i32,
    /// 特训后维度 3 固定加成。
    pub special_training_power3_bonus_fixed: i32,
    /// VS 卡支援团。
    pub support_unit: Option<String>,
    /// 可直接使用的最大等级。
    pub max_level: Option<i32>,
    /// 可直接使用的最大技能等级。
    pub max_skill_level: Option<i32>,
    /// 可直接使用的最大 master rank。
    pub max_master_rank: Option<i32>,
}

/// 卡牌等级参数。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardParameter {
    /// 卡 ID。
    pub card_id: i32,
    /// 等级。
    pub level: i32,
    /// 三维 1。
    pub param1: i32,
    /// 三维 2。
    pub param2: i32,
    /// 三维 3。
    pub param3: i32,
}

/// 稀有度上限。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardRarity {
    /// 稀有度类型。
    pub card_rarity_type: i32,
    /// 最大等级。
    pub max_level: i32,
    /// 特训前最大等级。
    pub normal_max_level: i32,
    /// 最大技能等级。
    pub max_skill_level: i32,
}

/// 剧情加成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardEpisode {
    /// 卡 ID。
    pub card_id: i32,
    /// 剧情序号。
    pub episode_no: i32,
    /// 三维 1 加成。
    pub power1_bonus_fixed: i32,
    /// 三维 2 加成。
    pub power2_bonus_fixed: i32,
    /// 三维 3 加成。
    pub power3_bonus_fixed: i32,
}

/// 突破加成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MasterLesson {
    /// 稀有度类型。
    pub card_rarity_type: i32,
    /// master rank。
    pub master_rank: i32,
    /// 三维 1 加成。
    pub power1_bonus_fixed: i32,
    /// 三维 2 加成。
    pub power2_bonus_fixed: i32,
    /// 三维 3 加成。
    pub power3_bonus_fixed: i32,
}

/// 技能主表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skill {
    /// 技能 ID。
    pub id: i32,
    /// 技能等级。
    pub level: i32,
    /// 是否为花后技能。
    pub is_after_training: bool,
}

/// 技能效果表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillEffect {
    /// 技能 ID。
    pub skill_id: i32,
    /// 技能等级。
    pub skill_level: i32,
    /// 效果类型。
    pub effect_type: String,
    /// 主值。
    pub value: i32,
    /// 附加值。
    pub additional_value: Option<i32>,
    /// 组分 / 异团人数。
    pub unit_member_count: Option<i32>,
    /// 目标团。
    pub unit: Option<String>,
    /// 角色 rank 条件。
    pub activate_character_rank: Option<i32>,
}

/// 区域道具倍率。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AreaItemLevel {
    /// 道具 ID。
    pub area_item_id: i32,
    /// 等级。
    pub level: i32,
    /// 适用团。
    pub unit: Option<String>,
    /// 适用属性。
    pub attr: Option<String>,
    /// 适用角色。
    pub character_id: Option<i32>,
    /// 综合力倍率。
    pub power_rate: f64,
    /// 全匹配综合力倍率。
    pub power_all_match_rate: f64,
}

/// 角色所属团。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameCharacterUnit {
    /// 角色 ID。
    pub game_character_id: i32,
    /// 团代码。
    pub unit: String,
}

/// 角色 rank 倍率。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CharacterRank {
    /// rank。
    pub character_rank: i32,
    /// 综合力倍率。
    pub power_bonus_rate: f64,
}

/// MySekai 画布加成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardMysekaiCanvasBonus {
    /// 稀有度类型。
    pub card_rarity_type: i32,
    /// 三维 1 固定值。
    pub power1_bonus_fixed: i32,
    /// 三维 2 固定值。
    pub power2_bonus_fixed: i32,
    /// 三维 3 固定值。
    pub power3_bonus_fixed: i32,
}

/// MySekai gate 主表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MysekaiGate {
    /// gate ID。
    pub id: i32,
    /// gate 适用团。
    pub unit: String,
}

/// MySekai gate 等级表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MysekaiGateLevel {
    /// gate ID。
    pub mysekai_gate_id: i32,
    /// gate 等级。
    pub level: i32,
    /// 综合力加成率。
    pub power_bonus_rate: f64,
}

/// 活动主表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    /// 活动 ID。
    pub id: i32,
    /// 活动类型。
    pub event_type: String,
}

/// 活动卡加成表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventCard {
    /// 活动 ID。
    pub event_id: i32,
    /// 卡 ID。
    pub card_id: i32,
    /// 当期 bonus（x10 定点，保留 0.1% 精度）。
    pub bonus_rate_x10: i32,
    /// leader 额外 bonus（x10 定点）。
    pub leader_bonus_rate_x10: i32,
}

/// 活动 deck bonus 规则。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventDeckBonus {
    /// 活动 ID。
    pub event_id: i32,
    /// 角色 ID 条件。
    pub character_id: Option<i32>,
    /// 团条件。
    pub unit: Option<String>,
    /// 属性条件。
    pub attr: Option<String>,
    /// bonus（x10 定点，保留 0.1% 精度）。
    pub bonus_rate_x10: i32,
}

/// 活动卡 bonus 张数上限。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventCardBonusLimit {
    /// 活动 ID。
    pub event_id: i32,
    /// 上限。
    pub member_count_limit: i32,
}

/// 终章称号加成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventHonorBonus {
    /// 活动 ID。
    pub event_id: i32,
    /// 称号 ID。
    pub honor_id: i32,
    /// leader 角色 ID。
    pub leader_game_character_id: i32,
    /// bonus。
    pub bonus_rate: i32,
}

/// World Bloom 异色 bonus。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldBloomDiffAttrBonus {
    /// 不同属性数量。
    pub attr_count: i32,
    /// bonus。
    pub bonus_rate: i32,
}

/// World Bloom 章节。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldBloom {
    /// 活动 ID。
    pub event_id: i32,
    /// 章节角色 ID；终章可能为空。
    pub game_character_id: Option<i32>,
    /// 章节序号。
    pub chapter_no: i32,
    /// 章节类型（`game_character` / `finale`）。
    ///
    /// 对应 masterdata 字段 `worldBloomChapterType`；模拟终章行同样写
    /// `finale`。旧数据缺省时按普通章节处理。
    #[serde(default)]
    pub world_bloom_chapter_type: Option<String>,
}

/// World Bloom 支援角色类型 bonus。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WBSupportDeckCharacterBonus {
    /// 角色类型：specific / others。
    pub world_bloom_support_deck_character_type: String,
    /// bonus。
    pub bonus_rate: f64,
}

/// World Bloom 支援 master rank bonus。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WBSupportDeckMasterRankBonus {
    /// master rank。
    pub master_rank: i32,
    /// bonus。
    pub bonus_rate: f64,
}

/// World Bloom 支援技能等级 bonus。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WBSupportDeckSkillLevelBonus {
    /// 技能等级。
    pub skill_level: i32,
    /// bonus。
    pub bonus_rate: f64,
}

/// World Bloom 支援卡稀有度 bonus 表。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WBSupportDeckBonus {
    /// 稀有度类型。
    pub card_rarity_type: String,
    /// 角色类型 bonus。
    pub world_bloom_support_deck_character_bonuses: Vec<WBSupportDeckCharacterBonus>,
    /// master rank bonus。
    pub world_bloom_support_deck_master_rank_bonuses: Vec<WBSupportDeckMasterRankBonus>,
    /// 技能等级 bonus。
    pub world_bloom_support_deck_skill_level_bonuses: Vec<WBSupportDeckSkillLevelBonus>,
}

/// World Bloom 团限定支援 bonus。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WBSupportDeckUnitEventLimitedBonus {
    /// 活动 ID。
    pub event_id: i32,
    /// 角色 ID。
    pub game_character_id: i32,
    /// 卡 ID。
    pub card_id: i32,
    /// bonus。
    pub bonus_rate: f64,
}

/// 活动 fixture 上限。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventFixtureBonusLimit {
    /// 活动 ID。
    pub event_id: i32,
    /// bonus rate 上限。
    pub bonus_rate_limit: i32,
}

/// 活动技能上限。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventSkillScoreUpLimit {
    /// 活动 ID。
    pub event_id: i32,
    /// score up 上限。
    pub score_up_limit: i32,
}

/// 歌曲元数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MusicMeta {
    /// 歌曲 ID。
    pub music_id: i32,
    /// 难度代码（easy/normal/hard/expert/master/append）。base_score/skill_scores 按难度不同。
    #[serde(default)]
    pub difficulty: String,
    /// Solo 活动倍率。
    pub event_rate_solo: i32,
    /// Multi 活动倍率。
    pub event_rate_multi: i32,
    /// Auto 活动倍率。
    pub event_rate_auto: i32,
    /// base score。
    pub base_score: f64,
    /// auto base score。
    pub base_score_auto: f64,
    /// fever score。
    pub fever_score: f64,
    /// Solo 技能系数。
    pub solo_skill_scores: [f64; 6],
    /// Multi 技能系数。
    pub multi_skill_scores: [f64; 6],
    /// Auto 技能系数。
    pub auto_skill_scores: [f64; 6],
    /// 歌曲时长。
    pub music_time: f64,
    /// note 数。
    pub tap_count: i32,
}

/// 歌曲难度表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MusicDifficulty {
    /// 歌曲 ID。
    pub music_id: i32,
    /// 难度代码。
    pub difficulty: String,
    /// 难度特定活动倍率。
    pub event_rate: Option<i32>,
}

/// 活动稀有度 bonus。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventRarityBonusRate {
    /// 活动 ID。
    pub event_id: i32,
    /// 稀有度类型。
    pub card_rarity_type: i32,
    /// master rank 上界。
    pub master_rank: i32,
    /// bonus，单位为 0.1%。
    pub bonus_rate_x10: i32,
}

/// 称号等级。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HonorLevel {
    /// 等级。
    pub level: i32,
    /// 综合力加成。
    pub bonus: i32,
}

/// 称号主表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Honor {
    /// 称号 ID。
    pub id: i32,
    /// 各等级信息。
    pub levels: Vec<HonorLevel>,
    /// 资源包名（如 `honor_top_001000_event_wl_3rd_part1_cp1`）。
    ///
    /// 模拟 WL3 终章用它识别排行称号并合成 50% 队长加成；
    /// 旧数据缺省时跳过这些称号。
    #[serde(default)]
    pub asset_bundle_name: Option<String>,
}

/// 羁绊称号主表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BondsHonor {
    /// 羁绊称号 ID。
    pub id: i32,
}

/// 用户卡。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserCard {
    /// 卡 ID。
    pub card_id: i32,
    /// 等级。
    pub level: i32,
    /// 技能等级。
    pub skill_level: i32,
    /// master rank。
    pub master_rank: i32,
    /// 特训状态。
    pub special_training_status: String,
    /// 默认立绘。
    pub default_image: String,
    /// 已读剧情编号。
    pub episodes_read: Vec<i32>,
    /// 是否虚拟卡。
    pub is_virtual: bool,
    /// 画布 bonus 覆盖。
    pub has_canvas_bonus_override: Option<bool>,
}

/// 用户角色。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserCharacter {
    /// 角色 ID。
    pub character_id: i32,
    /// character rank。
    pub character_rank: i32,
}

/// 用户区域道具。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserAreaItem {
    /// 道具 ID。
    pub area_item_id: i32,
    /// 等级。
    pub level: i32,
}

/// 用户编组。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UserDeck {
    /// 编组 ID。
    pub deck_id: i32,
    /// 成员卡。
    pub cards: Vec<i32>,
}

/// 用户 WL 支援编组。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UserWBSupportDeck {
    /// 角色 ID。
    pub character_id: i32,
    /// 卡 ID。
    pub cards: Vec<i32>,
}

/// 用户 Challenge 编组。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UserChallengeDeck {
    /// 角色 ID。
    pub character_id: i32,
    /// 卡 ID。
    pub card_id: i32,
}

/// 用户 fixture bonus。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserFixtureBonus {
    /// 角色 ID。
    pub character_id: i32,
    /// 活动 ID。
    pub event_id: Option<i32>,
    /// 总 bonus rate。
    pub total_bonus_rate: i32,
}

/// 用户 gate bonus。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserGateBonus {
    /// 原始 gate ID。真实用户数据优先通过 masterdata 查表解析。
    pub mysekai_gate_id: Option<i32>,
    /// 原始 gate 等级。
    pub mysekai_gate_level: Option<i32>,
    /// 团代码。
    pub unit: String,
    /// bonus rate。
    pub bonus_rate: f64,
}

/// 用户称号。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserHonor {
    /// 称号 ID。
    pub honor_id: i32,
    /// 等级。
    pub level: i32,
}

/// 将单位代码转换为 allium `Unit`。
pub(crate) fn parse_unit_code(code: &str) -> Option<Unit> {
    let code = code.trim();
    let matches = |value: &str| code.eq_ignore_ascii_case(value);
    if matches("light_sound")
        || matches("lightsound")
        || matches("ln")
        || matches("leo_need")
        || matches("leoneed")
    {
        Some(Unit::LightSound)
    } else if matches("idol")
        || matches("mmj")
        || matches("more_more_jump")
        || matches("moremorejump")
    {
        Some(Unit::Idol)
    } else if matches("street")
        || matches("vbs")
        || matches("vivids_bad_squad")
        || matches("vividsbadsquad")
    {
        Some(Unit::Street)
    } else if matches("themepark")
        || matches("theme_park")
        || matches("wonderlands_x_showtime")
        || matches("wxs")
    {
        Some(Unit::Themepark)
    } else if matches("school_refusal")
        || matches("schoolrefusal")
        || matches("25ji")
        || matches("nightcord")
    {
        Some(Unit::SchoolRefusal)
    } else if matches("piapro") || matches("virtual_singer") || matches("vs") {
        Some(Unit::Piapro)
    } else if matches("any") {
        Some(Unit::Any)
    } else if matches("ref") {
        Some(Unit::Ref)
    } else if matches("diff") {
        Some(Unit::Diff)
    } else {
        None
    }
}

/// 将属性代码转换为 allium `Attr`。
pub(crate) fn parse_attr_code(code: &str) -> Option<Attr> {
    let code = code.trim();
    if code.eq_ignore_ascii_case("cool") {
        Some(Attr::Cool)
    } else if code.eq_ignore_ascii_case("cute") {
        Some(Attr::Cute)
    } else if code.eq_ignore_ascii_case("happy") {
        Some(Attr::Happy)
    } else if code.eq_ignore_ascii_case("pure") {
        Some(Attr::Pure)
    } else if code.eq_ignore_ascii_case("mysterious") || code.eq_ignore_ascii_case("mystery") {
        Some(Attr::Mysterious)
    } else {
        None
    }
}

/// 将 `Attr` 映射到 pool 使用的 0-based 5 属性索引。
pub(crate) fn attr_to_pool_index(attr: Attr) -> Option<u8> {
    match attr {
        Attr::Cool => Some(0),
        Attr::Cute => Some(1),
        Attr::Happy => Some(2),
        Attr::Pure => Some(3),
        Attr::Mysterious => Some(4),
        _ => None,
    }
}

/// 将 `Unit` 映射到 pool 使用的 0-based 6 个真实团索引。
pub(crate) fn unit_to_pool_index(unit: Unit) -> Option<u8> {
    match unit {
        Unit::LightSound => Some(0),
        Unit::Idol => Some(1),
        Unit::Street => Some(2),
        Unit::Themepark => Some(3),
        Unit::SchoolRefusal => Some(4),
        Unit::Piapro => Some(5),
        _ => None,
    }
}

/// 返回稀有度对应的 preset。
pub(crate) fn config_for_rarity(
    configs: &CardConfigSet,
    card_rarity_type: i32,
) -> &CardRarityConfig {
    match card_rarity_type {
        1 => &configs.rarity_1_config,
        2 => &configs.rarity_2_config,
        3 => &configs.rarity_3_config,
        4 => &configs.rarity_4_config,
        5 => &configs.rarity_birthday_config,
        _ => &configs.rarity_4_config,
    }
}

/// 返回卡牌默认立绘枚举。
pub(crate) fn default_image_kind(value: &str) -> DefaultImage {
    let value = value.trim();
    if value.eq_ignore_ascii_case("special_training")
        || value.eq_ignore_ascii_case("trained")
        || value.eq_ignore_ascii_case("after_training")
    {
        DefaultImage::SpecialTraining
    } else {
        DefaultImage::Original
    }
}

/// 判断卡是否已特训。
pub(crate) fn is_after_training(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("done")
        || value.eq_ignore_ascii_case("special_training")
        || value.eq_ignore_ascii_case("trained")
        || value.eq_ignore_ascii_case("after_training")
}

/// 将活动类型字符串转换为枚举。
pub(crate) fn parse_event_type(value: &str) -> Option<EventType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "marathon" => Some(EventType::Marathon),
        "cheerful" | "cheerful_carnival" | "cheerfulcarnival" => Some(EventType::CheerfulCarnival),
        "world_bloom" | "worldbloom" | "wl" => Some(EventType::WorldBloom),
        _ => None,
    }
}

/// 根据 event_id / event_type 解析有效活动类型。
pub(crate) fn resolve_event_type(game: &GameData<'_>, params: &BuildParams) -> Option<EventType> {
    if let Some(event_id) = params.event_id
        && let Some(event) = game.events.iter().find(|event| event.id == event_id)
        && let Some(kind) = parse_event_type(&event.event_type)
    {
        return Some(kind);
    }
    params.event_type.as_deref().and_then(parse_event_type)
}
