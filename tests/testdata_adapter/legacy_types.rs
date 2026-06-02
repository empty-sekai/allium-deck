use serde::Deserialize;

/// 回归清单根结构。
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyManifest {
    pub case_count: usize,
    pub cases: Vec<LegacyManifestCase>,
}

/// 单个 manifest case。
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyManifestCase {
    pub name: String,
    pub combo: String,
    pub suite_file: String,
    pub input_path: String,
    pub output_path: String,
    pub timeout_ms: u64,
    pub target: String,
    pub live_type: String,
    pub event_id: Option<i32>,
    #[serde(default)]
    pub algorithm_override: Option<String>,
    #[serde(default)]
    pub verify_output: Option<bool>,
}

/// 旧引擎 input JSON。
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyInput {
    pub algorithm: String,
    pub limit: usize,
    pub music_id: Option<i32>,
    pub music_diff: Option<String>,
    pub region: String,
    pub target: String,
    pub live_type: String,
    pub timeout_ms: u64,
    pub user_data_str: String,
    #[serde(default)]
    pub event_id: Option<i32>,
    #[serde(default)]
    pub target_bonus_list: Option<Vec<i32>>,
    #[serde(default)]
    pub fixed_cards: Option<Vec<i32>>,
    #[serde(default)]
    pub fixed_characters: Option<Vec<i32>>,
    #[serde(default)]
    pub excluded_cards: Option<Vec<i32>>,
    #[serde(default)]
    pub challenge_live_character_id: Option<i32>,
    #[serde(default)]
    pub multi_live_teammate_score_up: Option<i32>,
    #[serde(default)]
    pub multi_live_teammate_power: Option<i32>,
    #[serde(default)]
    pub unit_filter: Option<String>,
    #[serde(default)]
    pub attr_filter: Option<String>,
}

/// 旧引擎 output JSON 的单条结果。
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyOutput {
    pub total_power: i32,
    pub score: i32,
    pub live_score: i32,
    pub event_bonus_rate: f64,
    pub multi_live_score_up: f64,
    pub cards: Vec<LegacyOutputCard>,
}

/// 旧引擎 output JSON 的卡片条目。
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyOutputCard {
    pub card_id: i32,
}

/// output 文件可能是旧结果数组、decks 包装对象，也可能是 skipped 对象。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum LegacyOutputFile {
    Results(Vec<LegacyOutput>),
    WrappedResults(ReferenceOutputFile),
    Skipped(LegacySkippedOutput),
}

impl LegacyOutputFile {
    /// 提取为 reference_output 结果数组；skipped 输出返回空数组。
    pub fn into_results(self) -> Vec<LegacyOutput> {
        match self {
            Self::Results(results) => results,
            Self::WrappedResults(results) => results.decks,
            Self::Skipped(skipped) => {
                let _ = (&skipped.error, &skipped.status);
                Vec::new()
            }
        }
    }

    /// 判断 reference_output 是否为 skipped。
    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped(_))
    }
}

/// 参照输出 `{ "decks": [...] }` 形式的文件包装。
#[derive(Debug, Clone, Deserialize)]
pub struct ReferenceOutputFile {
    #[serde(default)]
    pub decks: Vec<LegacyOutput>,
}

/// skipped output 的最小结构。
#[derive(Debug, Clone, Deserialize)]
pub struct LegacySkippedOutput {
    pub error: String,
    pub status: String,
}

/// `user_data_str` 解出的旧用户数据。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserData {
    #[serde(default)]
    pub user_areas: Vec<LegacyUserArea>,
    #[serde(default)]
    pub user_cards: Vec<LegacyUserCard>,
    #[serde(default)]
    pub user_characters: Vec<LegacyUserCharacter>,
    #[serde(default)]
    pub user_honors: Vec<LegacyUserHonor>,
    #[serde(default)]
    pub user_mysekai_canvases: Vec<LegacyUserMysekaiCanvas>,
    #[serde(default)]
    pub user_mysekai_fixture_game_character_performance_bonuses: Vec<LegacyUserMysekaiFixtureBonus>,
    #[serde(default)]
    pub user_mysekai_gates: Vec<LegacyUserMysekaiGate>,
}

/// 旧用户区域数据。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserArea {
    #[serde(default)]
    pub area_items: Vec<LegacyAreaItem>,
}

/// 旧用户区域道具。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyAreaItem {
    pub area_item_id: i32,
    pub level: i32,
}

/// 旧用户卡。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserCard {
    pub card_id: i32,
    pub level: i32,
    pub skill_level: i32,
    pub master_rank: i32,
    pub special_training_status: String,
    pub default_image: String,
    #[serde(default)]
    pub episodes: Vec<LegacyCardEpisodeState>,
}

/// 旧用户卡剧情状态。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyCardEpisodeState {
    pub card_episode_id: i32,
    pub scenario_status: String,
}

/// 旧用户角色 rank。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserCharacter {
    pub character_id: i32,
    pub character_rank: i32,
}

/// 旧用户称号。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserHonor {
    pub honor_id: i32,
    pub level: i32,
}

/// 旧用户 MySekai 画布。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserMysekaiCanvas {
    pub card_id: i32,
}

/// 旧用户 MySekai 家具表现加成。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserMysekaiFixtureBonus {
    #[serde(rename = "gameCharacterId")]
    pub game_character_id: i32,
    pub total_bonus_rate: i32,
}

/// 旧用户 MySekai Gate。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUserMysekaiGate {
    pub mysekai_gate_id: i32,
    pub mysekai_gate_level: i32,
}
