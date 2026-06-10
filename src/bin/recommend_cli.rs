//! allium-deck standalone 验证 CLI。
//!
//! stdout 固定输出结构化 JSON，stderr 只输出人类可读进度和耗时，方便脚本做回归和性能对比。
//! 大文件仍从路径读取：`--masterdata`、`--music-metas`、`--user`。
//! `--params` 保留为兼容入口；直接 flags 会在解析后覆盖 params JSON。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use allium_deck::engine::{parse_build_params_json, parse_user_profile_json, OwnedGameData};
use allium_deck::handler::{
    build_card_pool, cultivated_user_cards, BuildParams, CardRarityConfig, GameData, MasterCard,
    SingleCardConfig, UserCard, UserProfile,
};
use allium_deck::pool::{CardIdx, CardPool};
use allium_deck::search::{
    search_instrumented, summarize_deck, DeckResult, DeckResultSummary, SearchContext, SearchParams,
};
use allium_deck::{LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy};
use serde::Serialize;

#[derive(Debug, Default)]
struct CliArgs {
    masterdata: Option<String>,
    music_metas: Option<String>,
    user: Option<String>,
    params: Option<String>,
    top_k: Option<usize>,
    timeout_ms: Option<u64>,
    overrides: ParamOverrides,
}

#[derive(Debug, Default)]
struct ParamOverrides {
    region: Option<String>,
    event_id: Option<Option<i32>>,
    event_type: Option<Option<String>>,
    live_type: Option<LiveType>,
    target: Option<ScoreTarget>,
    music_id: Option<Option<i32>>,
    music_diff: Option<Option<String>>,
    fixed_cards: Option<Vec<i32>>,
    fixed_characters: Option<Vec<i32>>,
    excluded_cards: Option<Vec<i32>>,
    world_bloom_character_id: Option<Option<i32>>,
    world_bloom_event_turn: Option<Option<i32>>,
    challenge_live_character_id: Option<Option<i32>>,
    event_unit: Option<Option<String>>,
    event_attr: Option<Option<String>>,
    unit_filter: Option<Option<String>>,
    attr_filter: Option<Option<String>>,
    filter_other_unit: Option<bool>,
    keep_after_training_state: Option<bool>,
    best_skill_as_leader: Option<bool>,
    skill_reference_strategy: Option<SkillReferenceStrategy>,
    live_skill_order: Option<LiveSkillOrder>,
    specific_skill_order: Option<Option<[usize; 5]>>,
    multi_teammate_score_up: Option<Option<i32>>,
    multi_teammate_power: Option<Option<i32>>,
    multi_live_score_up_lower_bound: Option<Option<f64>>,
    boost: Option<Option<i32>>,
    other_score: Option<Option<i32>>,
    life: Option<Option<i32>>,
    minimize: Option<bool>,
    rarity_1_config: Option<CardRarityConfig>,
    rarity_2_config: Option<CardRarityConfig>,
    rarity_3_config: Option<CardRarityConfig>,
    rarity_4_config: Option<CardRarityConfig>,
    rarity_birthday_config: Option<CardRarityConfig>,
    single_card_configs: Vec<SingleCardConfig>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("错误: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    if args.overrides.help_requested() {
        print_help();
        return Ok(());
    }

    let masterdata = args
        .masterdata
        .as_deref()
        .ok_or_else(|| "缺少参数 --masterdata".to_string())?;
    let music_metas = args
        .music_metas
        .as_deref()
        .ok_or_else(|| "缺少参数 --music-metas".to_string())?;
    let user_path = args
        .user
        .as_deref()
        .ok_or_else(|| "缺少参数 --user".to_string())?;
    let top_k = args.top_k.unwrap_or(5);
    let timeout_ms = args.timeout_ms.unwrap_or(30_000);

    let load_start = Instant::now();
    let owned = OwnedGameData::load(&PathBuf::from(masterdata), &PathBuf::from(music_metas))?;
    let game = owned.as_ref();
    let load_ms = ms(load_start);
    eprintln!("[load] masterdata+music_metas: {load_ms:.1}ms");

    let user = parse_user_profile_json(&read(user_path)?).map_err(|e| e.to_string())?;
    let params_json = match args.params.as_deref() {
        Some(path) => read(path)?,
        None => "{}".to_string(),
    };
    let mut params =
        parse_build_params_json(&params_json).map_err(|e| format!("解析 params 失败: {e}"))?;
    apply_overrides(&mut params, args.overrides);

    let build_start = Instant::now();
    let (pool, ctx) = build_card_pool(&user, &game, &params).map_err(|e| e.to_string())?;
    let build_pool_ms = ms(build_start);
    eprintln!(
        "[build_pool] {build_pool_ms:.1}ms  pool={} effective_live={:?}",
        pool.count(),
        ctx.effective_live_type()
    );

    let search_params = SearchParams { top_k, timeout_ms };
    let search_start = Instant::now();
    let (results, stats) = search_instrumented(&pool, &ctx, &search_params);
    let search_ms = ms(search_start);
    eprintln!(
        "[search] {search_ms:.1}ms  leaf={} ub_prunes={} ep_explored={} mono_break={}",
        stats.leaf_nodes, stats.ub_prunes, stats.ep_explored, stats.mono_break_prunes
    );
    eprintln!("[total] {:.1}ms", load_ms + build_pool_ms + search_ms);

    let mut render_user = user.clone();
    render_user.user_cards = cultivated_user_cards(&user, &game, &params);
    let user_cards = render_user
        .user_cards
        .iter()
        .map(|card| (card.card_id, card))
        .collect::<HashMap<_, _>>();

    let decks = results
        .iter()
        .enumerate()
        .map(|(rank, result)| {
            DeckOut::build(rank + 1, &pool, &ctx, &game, &user, &user_cards, result)
        })
        .collect::<Vec<_>>();

    let response = CliResponse {
        effective_params: params,
        search_params: SearchParamsOut {
            top_k: search_params.top_k,
            timeout_ms: search_params.timeout_ms,
        },
        diagnostics: Diagnostics {
            pool_size: pool.count(),
            effective_live_type: format!("{:?}", ctx.effective_live_type()),
            support_deck: SupportDeckDiagnostics::from_ctx(&ctx),
            search: SearchDiagnostics {
                leaf_nodes: stats.leaf_nodes,
                ub_prunes: stats.ub_prunes,
                leader_prunes: stats.leader_prunes,
                ep_candidates: stats.ep_candidates,
                ep_break_prunes: stats.ep_break_prunes,
                ep_continue_prunes: stats.ep_continue_prunes,
                ep_explored: stats.ep_explored,
                mono_break_prunes: stats.mono_break_prunes,
            },
        },
        timing: Timing {
            load_ms,
            build_pool_ms,
            search_ms,
            total_ms: load_ms + build_pool_ms + search_ms,
        },
        decks,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn parse_args() -> Result<CliArgs, String> {
    let mut parsed = CliArgs::default();
    let mut iter = std::env::args().skip(1).peekable();
    while let Some(raw) = iter.next() {
        if raw == "--help" || raw == "-h" {
            parsed.overrides.region = Some("__HELP__".to_string());
            continue;
        }
        let (flag, inline_value) = match raw.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (raw, None),
        };
        let mut value = || -> Result<String, String> {
            if let Some(value) = inline_value.clone() {
                return Ok(value);
            }
            iter.next().ok_or_else(|| format!("参数 {flag} 缺少值"))
        };

        match flag.as_str() {
            "--masterdata" => parsed.masterdata = Some(value()?),
            "--music-metas" => parsed.music_metas = Some(value()?),
            "--user" => parsed.user = Some(value()?),
            "--params" => parsed.params = Some(value()?),
            "--top-k" => parsed.top_k = Some(parse_usize(&value()?, &flag)?),
            "--timeout-ms" => parsed.timeout_ms = Some(parse_u64(&value()?, &flag)?),
            "--region" => parsed.overrides.region = Some(value()?),
            "--event-id" => parsed.overrides.event_id = Some(parse_optional_i32(&value()?, &flag)?),
            "--event-type" => parsed.overrides.event_type = Some(parse_optional_string(value()?)),
            "--live-type" => parsed.overrides.live_type = Some(parse_live_type(&value()?)?),
            "--target" => parsed.overrides.target = Some(parse_target(&value()?)?),
            "--music-id" => parsed.overrides.music_id = Some(parse_optional_i32(&value()?, &flag)?),
            "--music-diff" => parsed.overrides.music_diff = Some(parse_optional_string(value()?)),
            "--fixed-cards" => {
                parsed.overrides.fixed_cards = Some(parse_i32_list(&value()?, &flag)?)
            }
            "--fixed-characters" => {
                parsed.overrides.fixed_characters = Some(parse_i32_list(&value()?, &flag)?)
            }
            "--excluded-cards" => {
                parsed.overrides.excluded_cards = Some(parse_i32_list(&value()?, &flag)?)
            }
            "--world-bloom-character-id" => {
                parsed.overrides.world_bloom_character_id =
                    Some(parse_optional_i32(&value()?, &flag)?)
            }
            "--world-bloom-event-turn" => {
                parsed.overrides.world_bloom_event_turn =
                    Some(parse_optional_i32(&value()?, &flag)?)
            }
            "--challenge-live-character-id" => {
                parsed.overrides.challenge_live_character_id =
                    Some(parse_optional_i32(&value()?, &flag)?)
            }
            "--event-unit" => parsed.overrides.event_unit = Some(parse_optional_string(value()?)),
            "--event-attr" => parsed.overrides.event_attr = Some(parse_optional_string(value()?)),
            "--unit-filter" => parsed.overrides.unit_filter = Some(parse_optional_string(value()?)),
            "--attr-filter" => parsed.overrides.attr_filter = Some(parse_optional_string(value()?)),
            "--filter-other-unit" => parsed.overrides.filter_other_unit = Some(true),
            "--no-filter-other-unit" => parsed.overrides.filter_other_unit = Some(false),
            "--keep-after-training-state" => {
                parsed.overrides.keep_after_training_state = Some(true)
            }
            "--no-keep-after-training-state" => {
                parsed.overrides.keep_after_training_state = Some(false)
            }
            "--best-skill-as-leader" => parsed.overrides.best_skill_as_leader = Some(true),
            "--no-best-skill-as-leader" => parsed.overrides.best_skill_as_leader = Some(false),
            "--skill-reference-strategy" => {
                parsed.overrides.skill_reference_strategy =
                    Some(parse_skill_reference_strategy(&value()?)?)
            }
            "--live-skill-order" | "--skill-order-choose-strategy" => {
                parsed.overrides.live_skill_order = Some(parse_live_skill_order(&value()?)?)
            }
            "--specific-skill-order" => {
                parsed.overrides.specific_skill_order =
                    Some(parse_optional_skill_order(&value()?, &flag)?)
            }
            "--multi-teammate-score-up" | "--teammate-score-up" => {
                parsed.overrides.multi_teammate_score_up =
                    Some(parse_optional_i32(&value()?, &flag)?)
            }
            "--multi-teammate-power" | "--teammate-power" => {
                parsed.overrides.multi_teammate_power = Some(parse_optional_i32(&value()?, &flag)?)
            }
            "--multi-live-score-up-lower-bound" => {
                parsed.overrides.multi_live_score_up_lower_bound =
                    Some(parse_optional_f64(&value()?, &flag)?)
            }
            "--boost" => parsed.overrides.boost = Some(parse_optional_i32(&value()?, &flag)?),
            "--other-score" => {
                parsed.overrides.other_score = Some(parse_optional_i32(&value()?, &flag)?)
            }
            "--life" => parsed.overrides.life = Some(parse_optional_i32(&value()?, &flag)?),
            "--minimize" => parsed.overrides.minimize = Some(true),
            "--no-minimize" => parsed.overrides.minimize = Some(false),
            "--rarity1-config" => {
                parsed.overrides.rarity_1_config = Some(parse_card_config(&value()?))
            }
            "--rarity2-config" => {
                parsed.overrides.rarity_2_config = Some(parse_card_config(&value()?))
            }
            "--rarity3-config" => {
                parsed.overrides.rarity_3_config = Some(parse_card_config(&value()?))
            }
            "--rarity4-config" => {
                parsed.overrides.rarity_4_config = Some(parse_card_config(&value()?))
            }
            "--rarity-birthday-config" => {
                parsed.overrides.rarity_birthday_config = Some(parse_card_config(&value()?))
            }
            "--single-card-config" => parsed
                .overrides
                .single_card_configs
                .push(parse_single_card_config(&value()?, &flag)?),
            _ => return Err(format!("未知参数 {flag}，使用 --help 查看支持列表")),
        }
    }
    Ok(parsed)
}

impl ParamOverrides {
    fn help_requested(&self) -> bool {
        self.region.as_deref() == Some("__HELP__")
    }
}

fn apply_overrides(params: &mut BuildParams, overrides: ParamOverrides) {
    if let Some(value) = overrides.region.filter(|value| value != "__HELP__") {
        params.region = value;
    }
    if let Some(value) = overrides.event_id {
        params.event_id = value;
    }
    if let Some(value) = overrides.event_type {
        params.event_type = value;
    }
    if let Some(value) = overrides.live_type {
        params.live_type = value;
    }
    if let Some(value) = overrides.target {
        params.target = value;
    }
    if let Some(value) = overrides.music_id {
        params.music_id = value;
    }
    if let Some(value) = overrides.music_diff {
        params.music_diff = value;
    }
    if let Some(value) = overrides.fixed_cards {
        params.fixed_cards = value;
    }
    if let Some(value) = overrides.fixed_characters {
        params.fixed_characters = value;
    }
    if let Some(value) = overrides.excluded_cards {
        params.excluded_cards = value;
    }
    if let Some(value) = overrides.world_bloom_character_id {
        params.world_bloom_character_id = value;
    }
    if let Some(value) = overrides.world_bloom_event_turn {
        params.world_bloom_event_turn = value;
    }
    if let Some(value) = overrides.challenge_live_character_id {
        params.challenge_live_character_id = value;
    }
    if let Some(value) = overrides.event_unit {
        params.event_unit = value;
    }
    if let Some(value) = overrides.event_attr {
        params.event_attr = value;
    }
    if let Some(value) = overrides.unit_filter {
        params.unit_filter = value;
    }
    if let Some(value) = overrides.attr_filter {
        params.attr_filter = value;
    }
    if let Some(value) = overrides.filter_other_unit {
        params.filter_other_unit = value;
    }
    if let Some(value) = overrides.keep_after_training_state {
        params.keep_after_training_state = value;
    }
    if let Some(value) = overrides.best_skill_as_leader {
        params.best_skill_as_leader = value;
    }
    if let Some(value) = overrides.skill_reference_strategy {
        params.skill_reference_strategy = value;
    }
    if let Some(value) = overrides.live_skill_order {
        params.live_skill_order = value;
    }
    if let Some(value) = overrides.specific_skill_order {
        params.specific_skill_order = value;
    }
    if let Some(value) = overrides.multi_teammate_score_up {
        params.multi_teammate_score_up = value;
    }
    if let Some(value) = overrides.multi_teammate_power {
        params.multi_teammate_power = value;
    }
    if let Some(value) = overrides.multi_live_score_up_lower_bound {
        params.multi_live_score_up_lower_bound = value;
    }
    if let Some(value) = overrides.boost {
        params.boost = value;
    }
    if let Some(value) = overrides.other_score {
        params.other_score = value;
    }
    if let Some(value) = overrides.life {
        params.life = value;
    }
    if let Some(value) = overrides.minimize {
        params.minimize = value;
    }
    if let Some(value) = overrides.rarity_1_config {
        params.card_configs.rarity_1_config = value;
    }
    if let Some(value) = overrides.rarity_2_config {
        params.card_configs.rarity_2_config = value;
    }
    if let Some(value) = overrides.rarity_3_config {
        params.card_configs.rarity_3_config = value;
    }
    if let Some(value) = overrides.rarity_4_config {
        params.card_configs.rarity_4_config = value;
    }
    if let Some(value) = overrides.rarity_birthday_config {
        params.card_configs.rarity_birthday_config = value;
    }
    if !overrides.single_card_configs.is_empty() {
        params.single_card_configs = overrides.single_card_configs;
    }
}

fn print_help() {
    println!(
        "recommend_cli --masterdata DIR --music-metas FILE --user FILE [--params FILE] [flags]\n\
         \n\
         常用 flags:\n\
         --event-id N --event-type marathon|cheerful_carnival|world_bloom\n\
         --music-id N --music-diff expert --live-type solo|multi|cheerful|auto|challenge|challenge_auto|mysekai\n\
         --target score|power|skill|mysekai --boost 0..10 --top-k N --timeout-ms N\n\
         --fixed-cards 1,2 --fixed-characters 1,2 --excluded-cards 3,4\n\
         --event-unit ln --event-attr cool --filter-other-unit --unit-filter ln --attr-filter cool\n\
         --world-bloom-character-id N --world-bloom-event-turn N --challenge-live-character-id N\n\
         --skill-reference-strategy average|max|min --live-skill-order best|worst|average|specific\n\
         --specific-skill-order 0,1,2,3,4 --best-skill-as-leader --keep-after-training-state\n\
         --multi-teammate-power N --multi-teammate-score-up N --multi-live-score-up-lower-bound N\n\
         --other-score N --life N --minimize\n\
         --rarity4-config level_max,skill_max,master_max,episode_read,canvas\n\
         --single-card-config 123:level_max,skill_max,master_max"
    );
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("读取 {path} 失败: {e}"))
}

fn parse_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("null")
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_optional_i32(value: &str, flag: &str) -> Result<Option<i32>, String> {
    if is_none_token(value) {
        return Ok(None);
    }
    value
        .parse::<i32>()
        .map(Some)
        .map_err(|e| format!("{flag} 需要整数: {e}"))
}

fn parse_optional_f64(value: &str, flag: &str) -> Result<Option<f64>, String> {
    if is_none_token(value) {
        return Ok(None);
    }
    value
        .parse::<f64>()
        .map(Some)
        .map_err(|e| format!("{flag} 需要数字: {e}"))
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|e| format!("{flag} 需要非负整数: {e}"))
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|e| format!("{flag} 需要非负整数: {e}"))
}

fn parse_i32_list(value: &str, flag: &str) -> Result<Vec<i32>, String> {
    if value.trim().is_empty() || is_none_token(value) {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<i32>()
                .map_err(|e| format!("{flag} 含非法整数 {part:?}: {e}"))
        })
        .collect()
}

fn parse_optional_skill_order(value: &str, flag: &str) -> Result<Option<[usize; 5]>, String> {
    if is_none_token(value) {
        return Ok(None);
    }
    let values = value
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<usize>()
                .map_err(|e| format!("{flag} 含非法索引 {part:?}: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 5 {
        return Err(format!("{flag} 需要 5 个 0-based 索引"));
    }
    let mut seen = [false; 5];
    for &index in &values {
        if index >= 5 {
            return Err(format!("{flag} 索引越界: {index}"));
        }
        if seen[index] {
            return Err(format!("{flag} 索引重复: {index}"));
        }
        seen[index] = true;
    }
    let mut order = [0usize; 5];
    order.copy_from_slice(&values);
    Ok(Some(order))
}

fn is_none_token(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed.eq_ignore_ascii_case("unset")
}

fn parse_live_type(value: &str) -> Result<LiveType, String> {
    match normalize_token(value).as_str() {
        "solo" => Ok(LiveType::Solo),
        "auto" => Ok(LiveType::Auto),
        "multi" => Ok(LiveType::Multi),
        "cheerful" => Ok(LiveType::Cheerful),
        "challenge" => Ok(LiveType::Challenge),
        "challengeauto" => Ok(LiveType::ChallengeAuto),
        "mysekai" => Ok(LiveType::Mysekai),
        _ => Err(format!("未知 live type: {value}")),
    }
}

fn parse_target(value: &str) -> Result<ScoreTarget, String> {
    match normalize_token(value).as_str() {
        "score" => Ok(ScoreTarget::Score),
        "power" => Ok(ScoreTarget::Power),
        "skill" => Ok(ScoreTarget::Skill),
        "mysekai" => Ok(ScoreTarget::Mysekai),
        _ => Err(format!("未知 target: {value}")),
    }
}

fn parse_skill_reference_strategy(value: &str) -> Result<SkillReferenceStrategy, String> {
    match normalize_token(value).as_str() {
        "max" | "best" => Ok(SkillReferenceStrategy::Max),
        "min" | "worst" => Ok(SkillReferenceStrategy::Min),
        "average" | "avg" => Ok(SkillReferenceStrategy::Average),
        _ => Err(format!("未知 skill reference strategy: {value}")),
    }
}

fn parse_live_skill_order(value: &str) -> Result<LiveSkillOrder, String> {
    match normalize_token(value).as_str() {
        "best" | "max" => Ok(LiveSkillOrder::Best),
        "worst" | "min" => Ok(LiveSkillOrder::Worst),
        "average" | "avg" => Ok(LiveSkillOrder::Average),
        "specific" => Ok(LiveSkillOrder::Specific),
        _ => Err(format!("未知 live skill order: {value}")),
    }
}

fn normalize_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['_', '-'], "")
}

fn parse_card_config(value: &str) -> CardRarityConfig {
    let mut config = CardRarityConfig::default();
    for part in value.split(',') {
        match normalize_token(part).as_str() {
            "disable" | "disabled" => config.disable = true,
            "levelmax" | "level" | "maxlevel" => config.level_max = true,
            "skillmax" | "skill" | "maxskill" => config.skill_max = true,
            "episoderead" | "episode" | "story" => config.episode_read = true,
            "mastermax" | "master" | "masterrank" => config.master_max = true,
            "canvas" | "mysekaicanvas" => config.canvas = true,
            "" => {}
            _ => {}
        }
    }
    config
}

fn parse_single_card_config(value: &str, flag: &str) -> Result<SingleCardConfig, String> {
    let (card_id, config) = value
        .split_once(':')
        .ok_or_else(|| format!("{flag} 格式应为 cardId:level_max,skill_max,..."))?;
    Ok(SingleCardConfig {
        card_id: card_id
            .trim()
            .parse::<i32>()
            .map_err(|e| format!("{flag} cardId 非法: {e}"))?,
        config: parse_card_config(config),
    })
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[derive(Serialize)]
struct CliResponse {
    effective_params: BuildParams,
    search_params: SearchParamsOut,
    diagnostics: Diagnostics,
    timing: Timing,
    decks: Vec<DeckOut>,
}

#[derive(Serialize)]
struct SearchParamsOut {
    top_k: usize,
    timeout_ms: u64,
}

#[derive(Serialize)]
struct Timing {
    load_ms: f64,
    build_pool_ms: f64,
    search_ms: f64,
    total_ms: f64,
}

#[derive(Serialize)]
struct Diagnostics {
    pool_size: usize,
    effective_live_type: String,
    support_deck: SupportDeckDiagnostics,
    search: SearchDiagnostics,
}

#[derive(Serialize)]
struct SupportDeckDiagnostics {
    count: u8,
    candidates: usize,
    nonzero_bonus: usize,
    top: Vec<SupportCardOut>,
}

impl SupportDeckDiagnostics {
    fn from_ctx(ctx: &SearchContext) -> Self {
        let top = ctx
            .support_deck
            .cards
            .iter()
            .take(5)
            .map(|(card_id, bonus)| SupportCardOut {
                card_id: *card_id,
                bonus: *bonus,
            })
            .collect::<Vec<_>>();
        Self {
            count: ctx.support_deck.count,
            candidates: ctx.support_deck.cards.len(),
            nonzero_bonus: ctx
                .support_deck
                .cards
                .iter()
                .filter(|(_, bonus)| *bonus > 0.0)
                .count(),
            top,
        }
    }
}

#[derive(Serialize)]
struct SupportCardOut {
    card_id: u16,
    bonus: f64,
}

#[derive(Serialize)]
struct SearchDiagnostics {
    leaf_nodes: u64,
    ub_prunes: u64,
    leader_prunes: u64,
    ep_candidates: u64,
    ep_break_prunes: u64,
    ep_continue_prunes: u64,
    ep_explored: u64,
    mono_break_prunes: u64,
}

#[derive(Serialize)]
struct DeckOut {
    rank: usize,
    target_value: u64,
    cards: Vec<CardOut>,
    total_power: Option<i32>,
    live_score: Option<i32>,
    event_point: Option<i32>,
    multi_live_score_up: Option<f64>,
    event_bonus_total: Option<f64>,
}

#[derive(Serialize)]
struct CardOut {
    card_id: i32,
    dense_idx: usize,
    character_id: u8,
    attr: u8,
    unit_mask_raw: u8,
    asset_key: String,
    rarity: String,
    attr_name: String,
    level: i32,
    skill_level: i32,
    master_rank: i32,
    trained: bool,
    has_canvas_bonus: bool,
    canvas_power: i32,
    power_total: Option<i32>,
    pool_power_max: u32,
    event_bonus: Option<f64>,
    skill_score_up: f64,
    pool_skill_min: u8,
    pool_skill_max: u8,
}

impl DeckOut {
    fn build(
        rank: usize,
        pool: &CardPool,
        ctx: &SearchContext,
        game: &GameData<'_>,
        original_user: &UserProfile,
        user_cards: &HashMap<i32, &UserCard>,
        result: &DeckResult,
    ) -> Self {
        let summary = summarize_deck(pool, ctx, &result.cards);
        let order = summary
            .map(|summary| summary.ordered_cards)
            .unwrap_or(result.cards);
        let cards = (0..5)
            .map(|pos| {
                let card = order[pos];
                CardOut::build(
                    pool,
                    ctx,
                    game,
                    original_user,
                    user_cards,
                    card,
                    summary.as_ref(),
                    pos,
                )
            })
            .collect::<Vec<_>>();

        Self {
            rank,
            target_value: result.score,
            cards,
            total_power: summary.map(|value| value.total_power),
            live_score: summary.map(|value| value.live_score),
            event_point: summary.and_then(|value| value.event_point),
            multi_live_score_up: summary.map(|value| value.multi_live_score_up),
            event_bonus_total: summary.and_then(|value| value.event_bonus_total),
        }
    }
}

impl CardOut {
    fn build(
        pool: &CardPool,
        ctx: &SearchContext,
        game: &GameData<'_>,
        original_user: &UserProfile,
        user_cards: &HashMap<i32, &UserCard>,
        card_idx: CardIdx,
        summary: Option<&DeckResultSummary>,
        pos: usize,
    ) -> Self {
        let card_id = pool.game_id(card_idx) as i32;
        let user_card = user_cards.get(&card_id).copied();
        let trained = user_card
            .map(default_image_is_trained)
            .unwrap_or_else(|| ctx.trained_to_special_image_at(card_idx.raw()));
        let meta = card_meta(game, card_id, trained);
        let event_bonus = summary
            .map(|value| value.card_event_bonus_rates[pos])
            .or_else(|| Some(pool.event_bonus(card_idx).total_rate()))
            .filter(|value| *value > 0.0);
        let has_canvas_bonus = user_card
            .and_then(|card| card.has_canvas_bonus_override)
            .unwrap_or_else(|| {
                original_user
                    .user_mysekai_canvas_bonus_cards
                    .contains(&card_id)
            });
        Self {
            card_id,
            dense_idx: card_idx.raw(),
            character_id: pool.char_id(card_idx),
            attr: pool.attr(card_idx),
            unit_mask_raw: pool.unit_mask_raw(card_idx),
            asset_key: meta.asset_key,
            rarity: meta.rarity.clone(),
            attr_name: meta.attr,
            level: user_card.map(|card| card.level).unwrap_or(0),
            skill_level: user_card.map(|card| card.skill_level).unwrap_or(0),
            master_rank: user_card.map(|card| card.master_rank).unwrap_or(0),
            trained,
            has_canvas_bonus,
            canvas_power: canvas_power(game, &meta.rarity, has_canvas_bonus),
            power_total: summary.map(|value| value.card_power_total[pos]),
            pool_power_max: pool.power_max(card_idx),
            event_bonus,
            skill_score_up: summary
                .map(|value| value.card_skill_score_up[pos])
                .unwrap_or_else(|| f64::from(pool.skill_max(card_idx))),
            pool_skill_min: pool.skill_min(card_idx),
            pool_skill_max: pool.skill_max(card_idx),
        }
    }
}

fn canvas_power(game: &GameData<'_>, rarity: &str, enabled: bool) -> i32 {
    if !enabled {
        return 0;
    }
    let rarity_type = rarity_type_to_index(rarity);
    game.card_mysekai_canvas_bonuses
        .iter()
        .find(|bonus| bonus.card_rarity_type == rarity_type)
        .map(|bonus| bonus.power1_bonus_fixed + bonus.power2_bonus_fixed + bonus.power3_bonus_fixed)
        .unwrap_or(0)
}

fn rarity_type_to_index(value: &str) -> i32 {
    match value.trim().to_ascii_lowercase().as_str() {
        "rarity_1" => 1,
        "rarity_2" => 2,
        "rarity_3" => 3,
        "rarity_4" => 4,
        "rarity_birthday" | "birthday" => 5,
        _ => 4,
    }
}

struct CardMeta {
    asset_key: String,
    rarity: String,
    attr: String,
}

fn card_meta(game: &GameData<'_>, card_id: i32, trained: bool) -> CardMeta {
    let training = if trained { "after_training" } else { "normal" };
    match game.cards.iter().find(|card| card.id == card_id) {
        Some(MasterCard {
            asset_bundle_name,
            rarity,
            attr,
            ..
        }) if !asset_bundle_name.is_empty() => CardMeta {
            asset_key: format!("thumbnail/chara/{asset_bundle_name}_{training}"),
            rarity: rarity.clone(),
            attr: attr.clone(),
        },
        Some(card) => CardMeta {
            asset_key: format!("thumbnail/chara/{card_id}_{training}"),
            rarity: card.rarity.clone(),
            attr: card.attr.clone(),
        },
        None => CardMeta {
            asset_key: format!("thumbnail/chara/{card_id}_{training}"),
            rarity: "rarity_4".to_string(),
            attr: "cool".to_string(),
        },
    }
}

fn default_image_is_trained(card: &UserCard) -> bool {
    matches!(
        card.default_image.trim().to_ascii_lowercase().as_str(),
        "special_training" | "trained" | "after_training"
    )
}
