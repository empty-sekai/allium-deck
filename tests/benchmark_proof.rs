#![allow(dead_code)]

mod testdata_adapter;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use allium_deck::handler::{build_card_pool, UserProfile};
use allium_deck::pool::CardPool;
use allium_deck::search::{brute_force_search, search_instrumented, DeckResult};
use serde::{Deserialize, Serialize};
use testdata_adapter::input_transform::transform_input;
use testdata_adapter::legacy_types::LegacyInput;
use testdata_adapter::masterdata_loader::OwnedGameData;

const DEFAULT_BF_CANDIDATE_LIMIT: u64 = 10_000_000;
const DEFAULT_BF_LARGE_CANDIDATE_LIMIT: u64 = 200_000_000;
const DEFAULT_BF_PER_CHAR_KEEP: usize = 2;
const DEFAULT_BF_CASE_LIMIT: usize = 24;
const DEFAULT_BF_LARGE_CASE_LIMIT: usize = 24;
const DEFAULT_BF_LARGE_SOURCE_MIN_POOL: usize = 180;
const DEFAULT_BF_MIN_RARITY: i32 = 3;

static GAME_CN: OnceLock<Result<OwnedGameData, String>> = OnceLock::new();
static GAME_JP: OnceLock<Result<OwnedGameData, String>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct Manifest {
    cases: Vec<ManifestCase>,
}

#[derive(Debug, Deserialize)]
struct ManifestCase {
    name: String,
    input_path: String,
    #[serde(default)]
    output_path: Option<String>,
    #[serde(default)]
    combo: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    live_type: Option<String>,
    #[serde(default)]
    event_id: Option<i32>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    expected_outcome: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct CorpusSummary {
    dataset: String,
    cases: usize,
    success_cases: usize,
    inputs: usize,
    missing_inputs: usize,
    outputs: usize,
    references: usize,
    triples: usize,
    by_target: BTreeMap<String, usize>,
    by_live_type: BTreeMap<String, usize>,
    by_combo: BTreeMap<String, usize>,
    with_event: usize,
    without_event: usize,
    sample_bf_candidates: Vec<CaseSketch>,
}

#[derive(Debug, Serialize)]
struct CaseSketch {
    name: String,
    input_path: String,
    target: String,
    live_type: String,
    event_id: Option<i32>,
    user_card_count: usize,
    has_output: bool,
    has_reference: bool,
}

#[derive(Debug, Serialize)]
struct ProofCaseReport {
    case: String,
    pool_size: usize,
    exact_score: Option<u64>,
    brute_score: Option<u64>,
    exact_cards: Vec<u16>,
    brute_cards: Vec<u16>,
    brute_candidates: u64,
    brute_evaluated: u64,
    brute_invalid: u64,
}

#[test]
fn testdata_corpus_layers_are_classified() {
    let summaries = ["mock", "real"]
        .into_iter()
        .map(|dataset| summarize_dataset(dataset).unwrap())
        .collect::<Vec<_>>();

    for summary in &summaries {
        assert!(
            summary.cases > 0,
            "{} manifest should not be empty",
            summary.dataset
        );
        assert!(
            summary.inputs > 0,
            "{} should contain readable inputs",
            summary.dataset
        );
        assert!(
            summary.by_target.contains_key("score"),
            "{} should contain score cases",
            summary.dataset
        );
    }

    let out_dir = proof_output_dir();
    fs::create_dir_all(&out_dir).unwrap();
    let path = out_dir.join("corpus-summary.json");
    fs::write(&path, serde_json::to_string_pretty(&summaries).unwrap()).unwrap();
    write_markdown_report(&summaries, &[]).unwrap();
    eprintln!("wrote {}", path.display());
}

#[test]
fn rust_bruteforce_matches_exact_on_full_testdata_pools() {
    let Some(masterdata_hint) = usable_masterdata_hint() else {
        eprintln!(
            "skip BF proof: set ALLIUM_MASTERDATA_CN/JP or provide ../masterdata_* from repo root"
        );
        return;
    };
    eprintln!(
        "BF proof using masterdata hint: {}",
        masterdata_hint.display()
    );

    let mut reports = Vec::new();
    for dataset in ["mock", "real"] {
        let root = testdata_dir(dataset);
        let manifest = load_manifest(&root).unwrap();
        for (case, input) in select_proof_cases(&root, &manifest, bf_case_limit()) {
            let Ok((params, user, mut search_params)) = transform_input(&input) else {
                continue;
            };
            search_params.top_k = search_params.top_k.min(bf_top_k());
            let Ok(game) = game_for_region(&params.region) else {
                continue;
            };
            let Ok((pool, ctx)) = build_card_pool(&user, &game.as_ref(), &params) else {
                continue;
            };
            if pool.count() < allium_deck::DECK_SIZE {
                continue;
            }
            let candidates = combination_count(pool.count(), allium_deck::DECK_SIZE);
            if candidates > bf_candidate_limit() {
                continue;
            }

            let (exact, _) = search_instrumented(&pool, &ctx, &search_params);
            let (brute, brute_stats) = brute_force_search(&pool, &ctx, &search_params);
            assert_same_results(&pool, &case.name, &exact, &brute);

            reports.push(ProofCaseReport {
                case: format!("{dataset}/{}", case.name),
                pool_size: pool.count(),
                exact_score: exact.first().map(|result| result.score),
                brute_score: brute.first().map(|result| result.score),
                exact_cards: exact
                    .first()
                    .map(|result| game_cards(&pool, result))
                    .unwrap_or_default(),
                brute_cards: brute
                    .first()
                    .map(|result| game_cards(&pool, result))
                    .unwrap_or_default(),
                brute_candidates: brute_stats.candidates,
                brute_evaluated: brute_stats.evaluated,
                brute_invalid: brute_stats.invalid,
            });
        }
    }

    assert!(
        !reports.is_empty(),
        "BF proof should exercise at least one full-pool fixture under ALLIUM_BF_CANDIDATE_LIMIT"
    );
    let out_dir = proof_output_dir();
    fs::create_dir_all(&out_dir).unwrap();
    let path = out_dir.join("rust-bf-proof.json");
    fs::write(&path, serde_json::to_string_pretty(&reports).unwrap()).unwrap();
    let summaries = ["mock", "real"]
        .into_iter()
        .map(|dataset| summarize_dataset(dataset).unwrap())
        .collect::<Vec<_>>();
    write_markdown_report(&summaries, &reports).unwrap();
    eprintln!("wrote {}", path.display());
}

/// issue #2 的 Top-K 回归：dominance 裁剪曾丢失被支配卡参与的次优解。
/// 完整卡池 C(n,5) 约 7000 万，默认 1000 万预算下会被全池对照跳过，
/// 故单列一个用例锁住该 fixture（仍受 ALLIUM_BF_CANDIDATE_LIMIT 控制，
/// 按 issue 验证命令以 ALLIUM_BF_CANDIDATE_LIMIT=100000000 运行）。
#[test]
fn rust_bruteforce_matches_exact_top_k_on_issue2_fixture() {
    const ISSUE2_CASE: &str = "mass_392500_score_multi_ev";

    if usable_masterdata_hint().is_none() {
        eprintln!(
            "skip issue #2 Top-K proof: set ALLIUM_MASTERDATA_CN/JP or provide ../masterdata_* from repo root"
        );
        return;
    }

    let root = testdata_dir("real");
    let manifest = load_manifest(&root).unwrap();
    let case = manifest
        .cases
        .iter()
        .find(|case| case.name == ISSUE2_CASE)
        .expect("issue #2 fixture should exist in real manifest");
    let input = load_json::<LegacyInput>(&root.join(&case.input_path)).unwrap();
    let (params, user, mut search_params) = transform_input(&input).unwrap();
    search_params.top_k = search_params.top_k.min(bf_top_k());
    let game = game_for_region(&params.region).unwrap();
    let (pool, ctx) = build_card_pool(&user, &game.as_ref(), &params).unwrap();
    let candidates = combination_count(pool.count(), allium_deck::DECK_SIZE);
    if candidates > bf_candidate_limit() {
        eprintln!(
            "skip issue #2 Top-K proof: {candidates} candidates exceed ALLIUM_BF_CANDIDATE_LIMIT={}",
            bf_candidate_limit()
        );
        return;
    }

    let (exact, _) = search_instrumented(&pool, &ctx, &search_params);
    let (brute, _) = brute_force_search(&pool, &ctx, &search_params);
    assert_same_results(&pool, ISSUE2_CASE, &exact, &brute);
}

#[test]
fn rust_bruteforce_matches_exact_on_large_filtered_pools() {
    let Some(masterdata_hint) = usable_masterdata_hint() else {
        eprintln!(
            "skip large filtered BF proof: set ALLIUM_MASTERDATA_CN/JP or provide ../masterdata_* from repo root"
        );
        return;
    };
    eprintln!(
        "large filtered BF proof using masterdata hint: {}",
        masterdata_hint.display()
    );

    let root = testdata_dir("real");
    let manifest = load_manifest(&root).unwrap();
    let mut reports = Vec::new();
    for (case, input) in select_large_pool_cases(&root, &manifest, bf_large_case_limit()) {
        let Ok((params, user, mut search_params)) = transform_input(&input) else {
            continue;
        };
        search_params.top_k = search_params.top_k.min(bf_top_k());
        let Ok(game) = game_for_region(&params.region) else {
            continue;
        };
        let Ok((source_pool, _)) = build_card_pool(&user, &game.as_ref(), &params) else {
            continue;
        };
        if source_pool.count() < bf_large_source_min_pool() {
            continue;
        }
        let filtered_user = filter_user_by_min_rarity(&user, game, bf_min_rarity());
        let Ok((filtered_pool, _)) = build_card_pool(&filtered_user, &game.as_ref(), &params)
        else {
            continue;
        };
        // 方案 B：按角色 top-N 裁剪，把高练度大池压到可暴力规模。
        let restricted_user =
            restrict_user_to_top_per_char(&filtered_user, &filtered_pool, bf_per_char_keep());
        let Ok((pool, ctx)) = build_card_pool(&restricted_user, &game.as_ref(), &params) else {
            continue;
        };
        if pool.count() < allium_deck::DECK_SIZE {
            continue;
        }
        let candidates = combination_count(pool.count(), allium_deck::DECK_SIZE);
        if candidates > bf_large_candidate_limit() {
            continue;
        }

        let (exact, _) = search_instrumented(&pool, &ctx, &search_params);
        let (brute, brute_stats) = brute_force_search(&pool, &ctx, &search_params);
        assert_same_results(&pool, &case.name, &exact, &brute);

        reports.push(ProofCaseReport {
            case: format!("real/{}", case.name),
            pool_size: pool.count(),
            exact_score: exact.first().map(|result| result.score),
            brute_score: brute.first().map(|result| result.score),
            exact_cards: exact
                .first()
                .map(|result| game_cards(&pool, result))
                .unwrap_or_default(),
            brute_cards: brute
                .first()
                .map(|result| game_cards(&pool, result))
                .unwrap_or_default(),
            brute_candidates: brute_stats.candidates,
            brute_evaluated: brute_stats.evaluated,
            brute_invalid: brute_stats.invalid,
        });
    }

    assert!(
        !reports.is_empty(),
        "large filtered BF proof should exercise at least one fixture"
    );
    let out_dir = proof_output_dir();
    fs::create_dir_all(&out_dir).unwrap();
    let path = out_dir.join("rust-bf-large-filtered-proof.json");
    fs::write(&path, serde_json::to_string_pretty(&reports).unwrap()).unwrap();
    eprintln!("wrote {}", path.display());
}

fn write_markdown_report(
    summaries: &[CorpusSummary],
    reports: &[ProofCaseReport],
) -> Result<(), String> {
    let mut out = String::new();
    out.push_str(
        "# Allium Deck Benchmark Proof Report

",
    );
    out.push_str(
        "## Corpus Summary

",
    );
    out.push_str("| Dataset | Cases | Inputs | Missing Inputs | References | Triples | Score | Power | Skill | Bonus | Mysekai |
");
    out.push_str(
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
",
    );
    for summary in summaries {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |
",
            summary.dataset,
            summary.cases,
            summary.inputs,
            summary.missing_inputs,
            summary.references,
            summary.triples,
            summary.by_target.get("score").copied().unwrap_or(0),
            summary.by_target.get("power").copied().unwrap_or(0),
            summary.by_target.get("skill").copied().unwrap_or(0),
            summary.by_target.get("bonus").copied().unwrap_or(0),
            summary.by_target.get("mysekai").copied().unwrap_or(0),
        ));
    }

    out.push_str(
        "
## Rust Exact vs Brute Force

",
    );
    if reports.is_empty() {
        out.push_str("Brute-force proof was not executed in this run. Configure `ALLIUM_MASTERDATA_CN`/`ALLIUM_MASTERDATA_JP` to enable it.
");
    } else {
        let total_candidates: u64 = reports.iter().map(|report| report.brute_candidates).sum();
        let total_evaluated: u64 = reports.iter().map(|report| report.brute_evaluated).sum();
        let total_invalid: u64 = reports.iter().map(|report| report.brute_invalid).sum();
        out.push_str(&format!(
            "- Result: Rust exact matched brute force on {}/{} full-pool fixtures.\n",
            reports.len(),
            reports.len(),
        ));
        out.push_str(&format!("- Brute-force candidates: {}\n", total_candidates));
        out.push_str(&format!("- Evaluated candidates: {}\n", total_evaluated));
        out.push_str(&format!("- Invalid candidates: {}\n", total_invalid));
        out.push_str(&format!("- BF candidate limit: {}\n", bf_candidate_limit()));
        out.push_str(&format!("- BF case limit: {}\n", bf_case_limit()));
        out.push_str(&format!("- BF top_k: {}\n", bf_top_k()));
        out.push_str("\n### Proof Cases\n\n");
        out.push_str("| Case | Pool | Score | Cards | BF Candidates |\n");
        out.push_str("|---|---:|---:|---|---:|\n");
        for report in reports {
            out.push_str(&format!(
                "| {} | {} | {} | {:?} | {} |\n",
                report.case,
                report.pool_size,
                report.exact_score.unwrap_or(0),
                report.exact_cards,
                report.brute_candidates,
            ));
        }
    }

    let path = proof_output_dir().join("report.md");
    fs::create_dir_all(proof_output_dir()).map_err(|err| err.to_string())?;
    fs::write(&path, out).map_err(|err| format!("write {} failed: {err}", path.display()))?;
    Ok(())
}

fn select_proof_cases<'a>(
    root: &Path,
    manifest: &'a Manifest,
    limit: usize,
) -> Vec<(&'a ManifestCase, LegacyInput)> {
    let mut selected = Vec::new();
    let mut seen_cases = BTreeSet::new();
    let mut seen_buckets = BTreeSet::new();

    for case in manifest.cases.iter().filter(|case| is_success_case(case)) {
        if selected.len() >= limit {
            break;
        }
        let Ok(input) = load_json::<LegacyInput>(&root.join(&case.input_path)) else {
            continue;
        };
        if input.target == "bonus" {
            continue;
        }
        let bucket = format!(
            "{}:{}:{}",
            input.target,
            input.live_type,
            input.event_id.is_some()
        );
        if seen_buckets.insert(bucket) && seen_cases.insert(case.name.clone()) {
            selected.push((case, input));
        }
    }

    for case in manifest.cases.iter().filter(|case| is_success_case(case)) {
        if selected.len() >= limit {
            break;
        }
        if !seen_cases.insert(case.name.clone()) {
            continue;
        }
        let Ok(input) = load_json::<LegacyInput>(&root.join(&case.input_path)) else {
            continue;
        };
        if input.target == "bonus" {
            continue;
        }
        selected.push((case, input));
    }

    selected
}

fn select_large_pool_cases<'a>(
    root: &Path,
    manifest: &'a Manifest,
    limit: usize,
) -> Vec<(&'a ManifestCase, LegacyInput)> {
    // 收集所有满足条件的候选（去掉同 user+target+live+event 的重复 combo），
    // 再按 user 轮询取样，使 case limit 下覆盖尽量多的不同练度账号。
    let mut candidates: Vec<(&ManifestCase, LegacyInput, String)> = Vec::new();
    let mut seen_buckets = BTreeSet::new();

    for case in manifest.cases.iter().filter(|case| is_success_case(case)) {
        let Ok(input) = load_json::<LegacyInput>(&root.join(&case.input_path)) else {
            continue;
        };
        if input.target == "bonus" {
            continue;
        }
        let Ok((params, user, _)) = transform_input(&input) else {
            continue;
        };
        let Ok(game) = game_for_region(&params.region) else {
            continue;
        };
        let Ok((source_pool, _)) = build_card_pool(&user, &game.as_ref(), &params) else {
            continue;
        };
        if source_pool.count() < bf_large_source_min_pool() {
            continue;
        }
        let filtered_user = filter_user_by_min_rarity(&user, game, bf_min_rarity());
        let Ok((filtered_pool, _)) = build_card_pool(&filtered_user, &game.as_ref(), &params)
        else {
            continue;
        };
        let restricted_user =
            restrict_user_to_top_per_char(&filtered_user, &filtered_pool, bf_per_char_keep());
        let Ok((pool, _)) = build_card_pool(&restricted_user, &game.as_ref(), &params) else {
            continue;
        };
        if pool.count() < allium_deck::DECK_SIZE {
            continue;
        }
        if combination_count(pool.count(), allium_deck::DECK_SIZE) > bf_large_candidate_limit() {
            continue;
        }
        let user_id = case
            .name
            .split('_')
            .nth(1)
            .unwrap_or(&case.name)
            .to_string();
        let bucket = format!(
            "{}:{}:{}:{}",
            user_id,
            input.target,
            input.live_type,
            input.event_id.is_some()
        );
        if seen_buckets.insert(bucket) {
            candidates.push((case, input, user_id));
        }
    }

    // 按 user 分组后轮询，优先覆盖不同账号。
    let mut by_user: BTreeMap<String, Vec<(&ManifestCase, LegacyInput)>> = BTreeMap::new();
    for (case, input, user_id) in candidates {
        by_user.entry(user_id).or_default().push((case, input));
    }
    let mut queues: Vec<_> = by_user.into_values().collect();
    let mut selected = Vec::new();
    let mut round = 0;
    while selected.len() < limit {
        let mut progressed = false;
        for queue in queues.iter_mut() {
            if let Some(item) = queue.get(round).cloned() {
                selected.push(item);
                progressed = true;
                if selected.len() >= limit {
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
        round += 1;
    }

    selected
}

fn filter_user_by_min_rarity(
    user: &UserProfile,
    game: &OwnedGameData,
    min_rarity: i32,
) -> UserProfile {
    let mut filtered = user.clone();
    filtered.user_cards.retain(|card| {
        game.cards
            .iter()
            .find(|master| master.id == card.card_id)
            .is_some_and(|master| master.card_rarity_type >= min_rarity)
    });
    filtered
}

/// 方案 B 大池 stress 裁剪：在已 build 的过滤池上，按角色对 power / skill / event-bonus
/// 三个维度各取 top-N，合并成需要保留的 game_id 集合，再据此裁剪用户卡。
///
/// 这是 stress 子集，用于覆盖高练度账号的高价值候选区，不声称是完整池证明：
/// 每角色被裁掉的低价值卡仍可能进入某些 Top-K 次优解。
fn restrict_user_to_top_per_char(
    user: &UserProfile,
    pool: &CardPool,
    per_char_keep: usize,
) -> UserProfile {
    use std::collections::BTreeMap;

    // char_id -> 候选 (game_id, power, skill, bonus)
    let mut by_char: BTreeMap<u8, Vec<(u16, u32, u8, u32)>> = BTreeMap::new();
    for card in pool.indices() {
        let entry = by_char.entry(pool.char_id(card)).or_default();
        entry.push((
            pool.game_id(card),
            pool.power_max(card),
            pool.skill_max(card),
            pool.event_bonus(card).total_ceil(),
        ));
    }

    let mut keep_game_ids = BTreeSet::new();
    for cards in by_char.values() {
        // 三个维度各取 top-N
        let mut by_power = cards.clone();
        by_power.sort_by(|a, b| b.1.cmp(&a.1));
        let mut by_skill = cards.clone();
        by_skill.sort_by(|a, b| b.2.cmp(&a.2));
        let mut by_bonus = cards.clone();
        by_bonus.sort_by(|a, b| b.3.cmp(&a.3));
        for ranked in [&by_power, &by_skill, &by_bonus] {
            for entry in ranked.iter().take(per_char_keep) {
                keep_game_ids.insert(entry.0);
            }
        }
    }

    let mut restricted = user.clone();
    restricted
        .user_cards
        .retain(|card| keep_game_ids.contains(&(card.card_id as u16)));
    restricted
}

fn bf_case_limit() -> usize {
    env_usize("ALLIUM_BF_CASE_LIMIT", DEFAULT_BF_CASE_LIMIT)
}

fn bf_large_case_limit() -> usize {
    env_usize("ALLIUM_BF_LARGE_CASE_LIMIT", DEFAULT_BF_LARGE_CASE_LIMIT)
}

fn bf_top_k() -> usize {
    env_usize("ALLIUM_BF_TOP_K", 3)
}

fn bf_candidate_limit() -> u64 {
    env_u64("ALLIUM_BF_CANDIDATE_LIMIT", DEFAULT_BF_CANDIDATE_LIMIT)
}

fn bf_large_candidate_limit() -> u64 {
    env_u64(
        "ALLIUM_BF_LARGE_CANDIDATE_LIMIT",
        DEFAULT_BF_LARGE_CANDIDATE_LIMIT,
    )
}

fn bf_large_source_min_pool() -> usize {
    env_usize(
        "ALLIUM_BF_LARGE_SOURCE_MIN_POOL",
        DEFAULT_BF_LARGE_SOURCE_MIN_POOL,
    )
}

fn bf_min_rarity() -> i32 {
    env_i32("ALLIUM_BF_MIN_RARITY", DEFAULT_BF_MIN_RARITY)
}

fn bf_per_char_keep() -> usize {
    env_usize("ALLIUM_BF_PER_CHAR_KEEP", DEFAULT_BF_PER_CHAR_KEEP)
}

fn env_i32(name: &str, default: i32) -> i32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn summarize_dataset(dataset: &str) -> Result<CorpusSummary, String> {
    let root = testdata_dir(dataset);
    let manifest = load_manifest(&root)?;
    let mut summary = CorpusSummary {
        dataset: dataset.to_string(),
        cases: manifest.cases.len(),
        ..CorpusSummary::default()
    };

    for case in &manifest.cases {
        let has_input = root.join(&case.input_path).exists();
        summary.inputs += has_input as usize;
        summary.missing_inputs += (!has_input) as usize;
        let has_output = case
            .output_path
            .as_ref()
            .is_some_and(|path| root.join(path).exists());
        let has_reference = case
            .output_path
            .as_ref()
            .map(|path| reference_path(&root, path))
            .is_some_and(|path| path.exists());
        summary.outputs += has_output as usize;
        summary.references += has_reference as usize;
        summary.triples += (has_output && has_reference) as usize;
        summary.success_cases += is_success_case(case) as usize;
        if case.event_id.is_some() {
            summary.with_event += 1;
        } else {
            summary.without_event += 1;
        }
        if let Some(target) = case
            .target
            .as_ref()
            .or_else(|| tag_value(&case.tags, &["score", "power", "skill", "bonus", "mysekai"]))
        {
            *summary.by_target.entry(target.clone()).or_default() += 1;
        }
        if let Some(live_type) = case
            .live_type
            .as_ref()
            .or_else(|| tag_value(&case.tags, &["solo", "multi", "auto", "challenge"]))
        {
            *summary.by_live_type.entry(live_type.clone()).or_default() += 1;
        }
        if let Some(combo) = &case.combo {
            *summary.by_combo.entry(combo.clone()).or_default() += 1;
        }

        if summary.sample_bf_candidates.len() < 16 {
            if let Ok(input) = load_json::<LegacyInput>(&root.join(&case.input_path)) {
                let user_card_count =
                    serde_json::from_str::<serde_json::Value>(&input.user_data_str)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("userCards")
                                .and_then(|cards| cards.as_array())
                                .map(Vec::len)
                        })
                        .unwrap_or(0);
                summary.sample_bf_candidates.push(CaseSketch {
                    name: case.name.clone(),
                    input_path: case.input_path.clone(),
                    target: input.target,
                    live_type: input.live_type,
                    event_id: input.event_id,
                    user_card_count,
                    has_output,
                    has_reference,
                });
            }
        }
    }
    Ok(summary)
}

fn combination_count(n: usize, k: usize) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut result = 1u64;
    for i in 0..k {
        result = result * (n - i) as u64 / (i + 1) as u64;
    }
    result
}

fn assert_same_results(
    pool: &CardPool,
    case_name: &str,
    exact: &[DeckResult],
    brute: &[DeckResult],
) {
    assert_eq!(
        exact.len(),
        brute.len(),
        "{case_name}: result length differs: exact={:?}, brute={:?}",
        describe_results(pool, exact),
        describe_results(pool, brute),
    );
    for (idx, (left, right)) in exact.iter().zip(brute.iter()).enumerate() {
        assert_eq!(
            left.score,
            right.score,
            "{case_name}: score differs at rank {idx}: exact={:?}, brute={:?}",
            describe_results(pool, exact),
            describe_results(pool, brute),
        );
        assert_eq!(
            left.game_card_set_key(pool),
            right.game_card_set_key(pool),
            "{case_name}: card set differs at rank {idx}: exact={:?}, brute={:?}",
            game_cards(pool, left),
            game_cards(pool, right),
        );
    }
}

fn describe_results(pool: &CardPool, results: &[DeckResult]) -> Vec<(u64, Vec<u16>)> {
    results
        .iter()
        .map(|result| (result.score, game_cards(pool, result)))
        .collect()
}

fn game_cards(pool: &CardPool, result: &DeckResult) -> Vec<u16> {
    let mut cards = result.cards.map(|card| pool.game_id(card)).to_vec();
    cards.sort_unstable();
    cards
}

fn load_manifest(root: &Path) -> Result<Manifest, String> {
    load_json(&root.join("manifest.json"))
}

fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("read {} failed: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("parse {} failed: {err}", path.display()))
}

fn reference_path(root: &Path, output_path: &str) -> PathBuf {
    let reference_name = output_path
        .strip_suffix("_output.json")
        .map(|prefix| format!("{prefix}_reference.json"))
        .unwrap_or_else(|| output_path.to_string());
    root.join(reference_name)
}

fn tag_value<'a>(tags: &'a [String], values: &[&str]) -> Option<&'a String> {
    tags.iter()
        .find(|tag| values.iter().any(|value| tag.as_str() == *value))
}

fn is_success_case(case: &ManifestCase) -> bool {
    case.expected_outcome.as_deref().unwrap_or("success") == "success"
}

fn testdata_dir(dataset: &str) -> PathBuf {
    let env_key = format!("ALLIUM_TESTDATA_{}", dataset.to_ascii_uppercase());
    std::env::var_os(&env_key)
        .map(PathBuf::from)
        .or_else(|| {
            if dataset == "real" {
                std::env::var_os("ALLIUM_TESTDATA").map(PathBuf::from)
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("testdata")
                .join(dataset)
        })
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
        other => Err(format!("unknown region: {other}")),
    }
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

fn usable_masterdata_hint() -> Option<PathBuf> {
    let cn = masterdata_dir("cn");
    cn.join("cards.json").exists().then_some(cn)
}

fn proof_output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("benchmark-proof")
}
