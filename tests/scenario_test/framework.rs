use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use allium_deck::handler::{build_card_pool, BuildError, BuildParams, UserProfile};
use allium_deck::search::{SearchContext, SearchParams};
use allium_deck::{LiveType, ScoreTarget};

use crate::testdata_adapter::input_transform::transform_input;
use crate::testdata_adapter::legacy_types::{
    LegacyInput, LegacyManifest, LegacyManifestCase, LegacyOutput, LegacyOutputFile,
};
use crate::testdata_adapter::masterdata_loader::OwnedGameData;

use super::scenarios::{ScenarioDef, ScenarioKind};

static MANIFEST: OnceLock<Result<LegacyManifest, String>> = OnceLock::new();
static SUITE_KEEP_IDS: OnceLock<Result<BTreeMap<String, BTreeSet<i32>>, String>> = OnceLock::new();
static GAME_CN: OnceLock<Result<OwnedGameData, String>> = OnceLock::new();
static GAME_JP: OnceLock<Result<OwnedGameData, String>> = OnceLock::new();

pub enum VerificationKind {
    Golden(Vec<LegacyOutput>),
    Soundness,
}

pub struct PreparedCase {
    pub scenario_name: &'static str,
    pub case_name: String,
    pub pool: allium_deck::pool::CardPool,
    pub ctx: SearchContext,
    pub search_params: SearchParams,
    pub verify: VerificationKind,
    pub timeout_ms: u64,
}

pub fn prepare_scenario_cases(def: &ScenarioDef) -> Result<Vec<PreparedCase>, String> {
    let manifest = manifest()?;
    let mut prepared = Vec::new();
    let mut skipped = 0usize;

    for (index, case) in manifest
        .cases
        .iter()
        .filter(|case| case.combo == def.source_combo)
        .filter(|case| case.algorithm_override.is_none())
        .enumerate()
    {
        match prepare_case(def, case, index)? {
            Some(prepared_case) => {
                prepared.push(prepared_case);
                if def.max_cases.is_some_and(|limit| prepared.len() >= limit) {
                    break;
                }
            }
            None => skipped += 1,
        }
    }

    if prepared.len() < def.min_cases {
        return Err(format!(
            "scenario {} 可用 case 只有 {} 个，跳过 {} 个，低于最小要求 {}",
            def.name,
            prepared.len(),
            skipped,
            def.min_cases,
        ));
    }
    Ok(prepared)
}

fn prepare_case(
    def: &ScenarioDef,
    case: &LegacyManifestCase,
    index: usize,
) -> Result<Option<PreparedCase>, String> {
    let input: LegacyInput = load_json(&testdata_dir().join(&case.input_path))?;
    let expected = load_output_file(&cpp_output_path(case))?;
    let (mut params, user, search_params) = transform_input(&input)?;
    let user = maybe_trim_user_for_mask(case, &user)?;
    let game = game_for_region(&params.region)?;

    if !apply_scenario_overrides(def.kind, index, &mut params, &user, game)? {
        return Ok(None);
    }

    match build_card_pool(&user, &game.as_ref(), &params) {
        Ok((pool, ctx)) => Ok(Some(PreparedCase {
            scenario_name: def.name,
            case_name: case.name.clone(),
            pool,
            ctx,
            search_params,
            verify: if matches!(def.kind, ScenarioKind::LegacyCombo)
                && case.verify_output.unwrap_or(true)
            {
                VerificationKind::Golden(expected)
            } else {
                VerificationKind::Soundness
            },
            timeout_ms: case.timeout_ms,
        })),
        Err(BuildError::EmptyPool) => Ok(None),
        Err(err) => Err(format!(
            "{} / {}: build_card_pool 失败: {err}",
            def.name, case.name
        )),
    }
}

fn apply_scenario_overrides(
    kind: ScenarioKind,
    index: usize,
    params: &mut BuildParams,
    user: &UserProfile,
    game: &OwnedGameData,
) -> Result<bool, String> {
    match kind {
        ScenarioKind::LegacyCombo => {}
        ScenarioKind::ScoreSoloEv => {
            params.target = ScoreTarget::Score;
            params.live_type = LiveType::Solo;
        }
        ScenarioKind::ScoreAutoEv => {
            params.target = ScoreTarget::Score;
            params.live_type = LiveType::Auto;
        }
        ScenarioKind::ScoreCheerful => {
            params.target = ScoreTarget::Score;
            params.live_type = LiveType::Multi;
            params.event_id = None;
            params.event_type = Some("cheerful_carnival".to_string());
            params.life = Some(1000);
        }
        ScenarioKind::PowerNoev => {
            params.target = ScoreTarget::Power;
            params.live_type = LiveType::Solo;
            params.event_id = None;
            params.event_type = None;
        }
        ScenarioKind::BonusWl => {
            params.target = ScoreTarget::Bonus;
            params.live_type = LiveType::Multi;
            params.event_id = None;
            params.event_type = Some("world_bloom".to_string());
            params.world_bloom_event_turn = Some(1);
            let Some(character_id) = pick_world_bloom_character(user, game) else {
                return Ok(false);
            };
            params.world_bloom_character_id = Some(character_id);
        }
        ScenarioKind::Mysekai => {
            params.target = ScoreTarget::Mysekai;
            params.live_type = LiveType::Mysekai;
            params.event_id = None;
            params.event_type = None;
        }
        ScenarioKind::ScoreFinalChapter => {
            params.target = ScoreTarget::Score;
            params.live_type = LiveType::Multi;
            params.event_id = Some(180);
            params.event_type = None;
        }
        ScenarioKind::BonusFinalChapter => {
            params.target = ScoreTarget::Bonus;
            params.live_type = LiveType::Multi;
            params.event_id = Some(180);
            params.event_type = None;
        }
        ScenarioKind::ScoreFixedCard => {
            params.target = ScoreTarget::Score;
            let Some(card_id) = pick_fixed_card(user) else {
                return Ok(false);
            };
            params.fixed_cards = vec![card_id];
            params.fixed_characters.clear();
        }
        ScenarioKind::ScoreFixedChar => {
            params.target = ScoreTarget::Score;
            let Some(character_id) = pick_fixed_character(user, game) else {
                return Ok(false);
            };
            params.fixed_cards.clear();
            params.fixed_characters = vec![character_id];
        }
        ScenarioKind::ScoreMultiEvDiffMusic => {
            params.target = ScoreTarget::Score;
            let Some(music_id) = pick_alternate_music(params.music_id, game, index) else {
                return Ok(false);
            };
            params.music_id = Some(music_id);
        }
    }
    Ok(true)
}

fn pick_fixed_card(user: &UserProfile) -> Option<i32> {
    user.user_cards.first().map(|card| card.card_id)
}

fn pick_fixed_character(user: &UserProfile, game: &OwnedGameData) -> Option<i32> {
    user.user_cards.iter().find_map(|user_card| {
        game.cards
            .iter()
            .find(|card| card.id == user_card.card_id)
            .map(|card| card.character_id)
    })
}

fn pick_world_bloom_character(user: &UserProfile, game: &OwnedGameData) -> Option<i32> {
    pick_fixed_character(user, game)
}

fn pick_alternate_music(
    current_music_id: Option<i32>,
    game: &OwnedGameData,
    index: usize,
) -> Option<i32> {
    let mut music_ids = game
        .music_metas
        .iter()
        .map(|meta| meta.music_id)
        .filter(|music_id| Some(*music_id) != current_music_id)
        .collect::<Vec<_>>();
    music_ids.sort_unstable();
    if music_ids.is_empty() {
        return None;
    }
    music_ids.get(index % music_ids.len()).copied()
}

fn manifest() -> Result<&'static LegacyManifest, String> {
    MANIFEST
        .get_or_init(|| {
            let path = testdata_dir().join("manifest.json");
            let text = fs::read_to_string(&path)
                .map_err(|err| format!("读取 manifest 失败 {}: {err}", path.display()))?;
            serde_json::from_str(&text)
                .map_err(|err| format!("解析 manifest 失败 {}: {err}", path.display()))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn game_for_region(region: &str) -> Result<&'static OwnedGameData, String> {
    match region.trim().to_ascii_lowercase().as_str() {
        "cn" => GAME_CN
            .get_or_init(|| OwnedGameData::load(&masterdata_dir("cn"), &music_metas_path()))
            .as_ref()
            .map_err(Clone::clone),
        "jp" => GAME_JP
            .get_or_init(|| OwnedGameData::load(&masterdata_dir("jp"), &music_metas_path()))
            .as_ref()
            .map_err(Clone::clone),
        other => Err(format!("未知 region: {other}")),
    }
}

fn suite_keep_ids() -> Result<&'static BTreeMap<String, BTreeSet<i32>>, String> {
    SUITE_KEEP_IDS
        .get_or_init(|| {
            let manifest = manifest()?;
            let mut result = BTreeMap::<String, BTreeSet<i32>>::new();
            for case in &manifest.cases {
                let outputs = load_output_file(&cpp_output_path(case))?;
                let entry = result.entry(case.suite_file.clone()).or_default();
                for output in outputs {
                    for card in output.cards {
                        entry.insert(card.card_id);
                    }
                }
            }
            Ok(result)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn maybe_trim_user_for_mask(
    case: &LegacyManifestCase,
    user: &UserProfile,
) -> Result<UserProfile, String> {
    const USER_CARD_TRIM_LIMIT: usize = 480;

    if user.user_cards.len() <= USER_CARD_TRIM_LIMIT {
        return Ok(user.clone());
    }

    let keep_ids = suite_keep_ids()?
        .get(&case.suite_file)
        .cloned()
        .unwrap_or_default();
    let mut kept_cards = user
        .user_cards
        .iter()
        .filter(|card| keep_ids.contains(&card.card_id))
        .cloned()
        .collect::<Vec<_>>();
    if kept_cards.len() > USER_CARD_TRIM_LIMIT {
        kept_cards.truncate(USER_CARD_TRIM_LIMIT);
    }
    if kept_cards.len() < USER_CARD_TRIM_LIMIT {
        for card in &user.user_cards {
            if kept_cards.len() >= USER_CARD_TRIM_LIMIT {
                break;
            }
            if !kept_cards
                .iter()
                .any(|existing| existing.card_id == card.card_id)
            {
                kept_cards.push(card.clone());
            }
        }
    }

    let kept_set = kept_cards
        .iter()
        .map(|card| card.card_id)
        .collect::<BTreeSet<_>>();
    let mut trimmed = user.clone();
    trimmed.user_cards = kept_cards;
    trimmed.user_mysekai_canvas_bonus_cards = trimmed
        .user_mysekai_canvas_bonus_cards
        .into_iter()
        .filter(|card_id| kept_set.contains(card_id))
        .collect();
    Ok(trimmed)
}

fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("读取 {} 失败: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("解析 {} 失败: {err}", path.display()))
}

fn load_output_file(path: &Path) -> Result<Vec<LegacyOutput>, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("读取 {} 失败: {err}", path.display()))?;
    let parsed: LegacyOutputFile = serde_json::from_str(&text)
        .map_err(|err| format!("解析 {} 失败: {err}", path.display()))?;
    Ok(parsed.into_results())
}

fn cpp_output_path(case: &LegacyManifestCase) -> PathBuf {
    let cpp_name = case
        .output_path
        .strip_suffix("_output.json")
        .map(|prefix| format!("{prefix}_cpp_output.json"))
        .unwrap_or_else(|| case.output_path.clone());
    testdata_dir().join(cpp_name)
}

fn testdata_dir() -> PathBuf {
    std::env::var_os("ALLIUM_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("scapus-deck-engine")
                .join("testdata")
                .join("real")
        })
}

fn masterdata_dir(region: &str) -> PathBuf {
    let env_key = match region {
        "cn" => "ALLIUM_MASTERDATA_CN",
        "jp" => "ALLIUM_MASTERDATA_JP",
        _ => "ALLIUM_MASTERDATA_CN",
    };
    std::env::var_os(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("..")
                .join(format!("masterdata_{region}"))
        })
}

fn music_metas_path() -> PathBuf {
    std::env::var_os("ALLIUM_MUSIC_METAS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("music_metas.json")
        })
}
