mod testdata_adapter;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use allium_deck::handler::{build_card_pool, BuildError};
use allium_deck::search::search;
use serde::Serialize;
use testdata_adapter::input_transform::transform_input;
use testdata_adapter::legacy_types::{
    LegacyInput, LegacyManifest, LegacyManifestCase, LegacyOutput, LegacyOutputFile,
};
use testdata_adapter::masterdata_loader::OwnedGameData;
use testdata_adapter::output_compare::{compare, CaseSummary, CompareCategory};

const COMBOS: [&str; 8] = [
    "bonus_multi_ev",
    "power_solo_ev",
    "power_solo_fast",
    "score_multi_ev",
    "score_multi_fast",
    "score_multi_noev",
    "score_noev_fast",
    "skill_auto_ev",
];

static MANIFEST: OnceLock<Result<LegacyManifest, String>> = OnceLock::new();
static SUITE_KEEP_IDS: OnceLock<Result<BTreeMap<String, BTreeSet<i32>>, String>> = OnceLock::new();
static GAME_CN: OnceLock<Result<OwnedGameData, String>> = OnceLock::new();
static GAME_JP: OnceLock<Result<OwnedGameData, String>> = OnceLock::new();

fn manifest() -> Result<&'static LegacyManifest, String> {
    let manifest = MANIFEST
        .get_or_init(|| {
            let path = testdata_dir().join("manifest.json");
            let text = fs::read_to_string(&path)
                .map_err(|err| format!("读取 manifest 失败 {}: {err}", path.display()))?;
            serde_json::from_str(&text)
                .map_err(|err| format!("解析 manifest 失败 {}: {err}", path.display()))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let _ = manifest.case_count;
    Ok(manifest)
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

fn run_combo(combo: &str) -> Result<Vec<CaseSummary>, String> {
    let manifest = manifest()?;
    let mut summaries = Vec::new();
    for case in manifest.cases.iter().filter(|case| case.combo == combo) {
        summaries.push(run_case(case)?);
    }
    write_combo_summary(combo, &summaries)?;
    Ok(summaries)
}

fn run_case(case: &LegacyManifestCase) -> Result<CaseSummary, String> {
    let input: LegacyInput = load_json(&testdata_dir().join(&case.input_path))?;
    let expected = load_output_file(&cpp_output_path(case))?;
    if input.algorithm != "dfs" && case.verify_output.unwrap_or(true) {
        return Err(format!(
            "verify_output=true 但 algorithm 不是 dfs: {}",
            input.algorithm
        ));
    }
    if input.target != case.target {
        return Err(format!(
            "case.target 与 input.target 不一致: {} vs {}",
            case.target, input.target
        ));
    }
    if input.live_type != case.live_type {
        return Err(format!(
            "case.live_type 与 input.live_type 不一致: {} vs {}",
            case.live_type, input.live_type
        ));
    }
    if input.event_id != case.event_id {
        return Err(format!(
            "case.event_id 与 input.event_id 不一致: {:?} vs {:?}",
            case.event_id, input.event_id
        ));
    }
    if case.algorithm_override.is_some() && case.verify_output.unwrap_or(true) {
        return Err("algorithm_override case 不应要求 verify_output=true".to_string());
    }
    let (params, user, search_params) = transform_input(&input)?;
    let user = maybe_trim_user_for_mask(case, &user)?;
    let game = game_for_region(&params.region)?;
    let summary = match build_card_pool(&user, &game.as_ref(), &params) {
        Ok((pool, ctx)) => {
            let results = search(&pool, &ctx, &search_params);
            let cmp = compare(
                params.target,
                &results,
                &pool,
                &ctx,
                &expected,
                case.verify_output.unwrap_or(true),
                case.timeout_ms,
            );
            CaseSummary {
                name: case.name.clone(),
                combo: case.combo.clone(),
                category: cmp.category,
                passed: cmp.passed,
                detail: cmp.detail,
            }
        }
        Err(BuildError::EmptyPool) if expected.is_empty() => CaseSummary {
            name: case.name.clone(),
            combo: case.combo.clone(),
            category: CompareCategory::Empty,
            passed: true,
            detail: "allium 与 C++ golden 均为空结果".to_string(),
        },
        Err(err) => CaseSummary {
            name: case.name.clone(),
            combo: case.combo.clone(),
            category: if case.timeout_ms <= 5_000 {
                CompareCategory::Timeout
            } else {
                CompareCategory::Bug
            },
            passed: case.timeout_ms <= 5_000,
            detail: format!("build_card_pool 失败: {err}"),
        },
    };

    Ok(summary)
}

fn maybe_trim_user_for_mask(
    case: &LegacyManifestCase,
    user: &allium_deck::handler::UserProfile,
) -> Result<allium_deck::handler::UserProfile, String> {
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

fn run_smoke(case_name: &str) -> Result<(), String> {
    let manifest = manifest()?;
    let case = manifest
        .cases
        .iter()
        .find(|entry| entry.name == case_name)
        .ok_or_else(|| format!("manifest 中不存在 case: {case_name}"))?;
    run_case(case).map(|_| ())
}

fn write_combo_summary(combo: &str, summaries: &[CaseSummary]) -> Result<(), String> {
    let dir = summary_dir();
    fs::create_dir_all(&dir)
        .map_err(|err| format!("创建 summary 目录失败 {}: {err}", dir.display()))?;
    let path = dir.join(format!("{combo}.json"));
    let payload = serde_json::to_string_pretty(&ComboSummary {
        combo: combo.to_string(),
        cases: summaries.to_vec(),
    })
    .map_err(|err| format!("序列化 combo summary 失败: {err}"))?;
    fs::write(&path, payload)
        .map_err(|err| format!("写入 combo summary 失败 {}: {err}", path.display()))
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

fn summary_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("e2e_regression")
}

#[derive(Debug, Serialize)]
struct ComboSummary {
    combo: String,
    cases: Vec<CaseSummary>,
}

#[test]
fn power_solo_smoke() {
    run_smoke("mass_005748_power_solo_ev").unwrap();
}

#[test]
fn e2e_bonus_multi_ev() {
    run_combo(COMBOS[0]).unwrap();
}

#[test]
fn e2e_power_solo_ev() {
    run_combo(COMBOS[1]).unwrap();
}

#[test]
fn e2e_power_solo_fast() {
    run_combo(COMBOS[2]).unwrap();
}

#[test]
fn e2e_score_multi_ev() {
    run_combo(COMBOS[3]).unwrap();
}

#[test]
fn e2e_score_multi_fast() {
    run_combo(COMBOS[4]).unwrap();
}

#[test]
fn e2e_score_multi_noev() {
    run_combo(COMBOS[5]).unwrap();
}

#[test]
fn e2e_score_noev_fast() {
    run_combo(COMBOS[6]).unwrap();
}

#[test]
fn e2e_skill_auto_ev() {
    run_combo(COMBOS[7]).unwrap();
}
