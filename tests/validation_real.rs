//! Opt-in real-account measurements with explicitly derived WL requests.
//!
//! Run `cargo test --release --test validation_real -- --ignored --nocapture`.
//! Required: ALLIUM_REAL_MASTERDATA, ALLIUM_REAL_MUSIC_METAS, ALLIUM_REAL_OUTPUT.
//! Optional: ALLIUM_REAL_CORPUS (testdata/real), REGION (cn), REPEATS (3),
//! TOPKS (1,8,30), ACCOUNT_START (0; distinct-account offset), ACCOUNT_LIMIT (4),
//! TIMEOUT_MS (2000), MODE (full|oracle),
//! SUBSET_CARDS (10; oracle only, 5..=16), ORACLE_DENSE_LIMIT (12; 5..=16),
//! LIVE_TYPES (multi,solo,auto), STATE_MODES (current; current,both), CATALOG (0),
//! REQUIRE_COMPLETE (oracle:1/full:0), LATENCY_LIMIT_MS (absent; e.g. 20),
//! WL_EVENT_ID, FINALE_EVENT_ID, MUSIC_ID and MUSIC_DIFF (prefix ALLIUM_REAL_).
//! Output never includes corpus paths, suite names, account IDs or user data.
//! These are real-account-derived requests, not captured WL requests. Oracle
//! mode additionally reduces the owned collection, including its support pool.
//! CATALOG=1 appends a full-catalog-derived profile in full mode; it never replaces
//! real accounts or reduces catalog ownership. STATE_MODES are request variants.
//! With REQUIRE_COMPLETE=1, an optional latency limit must be met by every
//! measured and warmup run. Collection mode records violations but never accepts.

#![allow(dead_code)]

mod testdata_adapter;

use allium_deck::engine::OwnedGameData;
use allium_deck::handler::{BuildError, BuildParams, UserCard, UserProfile, build_card_pool};
use allium_deck::pool::CardPool;
use allium_deck::search::{DeckResult, ExactOracle, SearchCompletion, SearchParams, search};
use allium_deck::{LiveSkillOrder, LiveType, ScoreTarget, is_world_bloom_finale_event};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;
use testdata_adapter::input_transform::transform_input;
use testdata_adapter::legacy_types::LegacyInput;

#[derive(Deserialize)]
struct Manifest {
    cases: Vec<ManifestCase>,
}

#[derive(Deserialize)]
struct ManifestCase {
    input_path: String,
    // Used only in memory to distinguish accounts, never emitted.
    suite_file: String,
}

struct Account {
    user: UserProfile,
    manifest_ordinal: Option<usize>,
    source_account: usize,
    is_catalog: bool,
    original_owned_cards: usize,
    original_unique_characters: usize,
    applicable: bool,
}

impl Account {
    fn account_id(&self) -> Option<usize> {
        (!self.is_catalog).then_some(self.source_account)
    }

    fn source(&self, oracle: bool) -> &'static str {
        if self.is_catalog {
            "full_catalog_derived"
        } else if oracle {
            "real_account_derived_card_subset_and_request"
        } else {
            "real_account_derived_request"
        }
    }
}

struct Case {
    account: usize,
    mode: &'static str,
    state_mode: &'static str,
    params: BuildParams,
}

fn env(name: &str, default: &str) -> String {
    std::env::var(format!("ALLIUM_REAL_{name}")).unwrap_or_else(|_| default.to_owned())
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(format!("ALLIUM_REAL_{name}"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("ALLIUM_REAL_{name} must be set"))
}

fn number<T: std::str::FromStr>(name: &str, default: &str) -> Result<T, String> {
    env(name, default)
        .parse()
        .map_err(|_| format!("invalid ALLIUM_REAL_{name}"))
}

fn flag(name: &str, default: &str) -> Result<bool, String> {
    match env(name, default).as_str() {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("ALLIUM_REAL_{name} must be 0 or 1")),
    }
}

fn optional_positive_number(name: &str) -> Result<Option<f64>, String> {
    std::env::var(format!("ALLIUM_REAL_{name}"))
        .ok()
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite() && *number > 0.0)
                .ok_or_else(|| format!("ALLIUM_REAL_{name} must be finite and positive"))
        })
        .transpose()
}

fn optional_id(name: &str) -> Result<Option<i32>, String> {
    std::env::var(format!("ALLIUM_REAL_{name}"))
        .ok()
        .map(|value| {
            value
                .parse()
                .map_err(|_| format!("invalid ALLIUM_REAL_{name}"))
        })
        .transpose()
}

fn emit(out: &mut BufWriter<File>, record: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *out, &record).map_err(|error| error.to_string())?;
    out.write_all(b"\n").map_err(|error| error.to_string())?;
    out.flush().map_err(|error| error.to_string())
}

fn result_rows(pool: &CardPool, results: &[DeckResult]) -> Vec<Value> {
    results
        .iter()
        .map(|result| {
            json!({
                "ordered_game_ids": result.cards.map(|card| pool.game_id(card)),
                "dense_variants": result.cards.map(|card| card.raw()),
                // A decimal string preserves every bit through JavaScript readers.
                "score": result.score.to_string(),
            })
        })
        .collect()
}

/// Full card ownership with card cultivation maxima taken from this snapshot.
/// Area items, character ranks, honors, gates and canvases are not invented.
fn catalog_user(game: &OwnedGameData) -> Result<UserProfile, String> {
    let mut cards = Vec::with_capacity(game.cards.len());
    let mut identities = BTreeSet::new();
    for master in &game.cards {
        if master.id <= 0 || !identities.insert(master.id) {
            return Err("catalog has invalid or duplicate card identity".to_owned());
        }
        let rarity = game
            .card_rarities
            .iter()
            .find(|row| row.card_rarity_type == master.card_rarity_type)
            .ok_or_else(|| format!("catalog card {} has no rarity metadata", master.id))?;
        let level = master.max_level.unwrap_or(rarity.max_level);
        let skill_level = master.max_skill_level.unwrap_or(rarity.max_skill_level);
        let master_rank = master.max_master_rank.unwrap_or_else(|| {
            game.master_lessons
                .iter()
                .filter(|row| row.card_rarity_type == master.card_rarity_type)
                .map(|row| row.master_rank)
                .max()
                .unwrap_or(0)
        });
        if level < 1 || skill_level < 1 || master_rank < 0 {
            return Err(format!(
                "catalog card {} has invalid cultivation maxima",
                master.id
            ));
        }
        let trained = master.special_training_skill_id.is_some()
            || master.special_training_power1_bonus_fixed > 0
            || master.special_training_power2_bonus_fixed > 0
            || master.special_training_power3_bonus_fixed > 0;
        let episodes_read = game
            .card_episodes
            .iter()
            .filter(|row| row.card_id == master.id)
            .map(|row| row.episode_no)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        cards.push(UserCard {
            card_id: master.id,
            level,
            skill_level,
            master_rank,
            special_training_status: if trained { "done" } else { "not_doing" }.to_owned(),
            default_image: if trained {
                "special_training"
            } else {
                "original"
            }
            .to_owned(),
            episodes_read,
            is_virtual: false,
            has_canvas_bonus_override: None,
        });
    }
    if cards.is_empty() {
        return Err("catalog has no cards".to_owned());
    }
    cards.sort_unstable_by_key(|card| card.card_id);
    Ok(UserProfile {
        user_cards: cards,
        ..UserProfile::default()
    })
}

fn reduced_user(
    user: &UserProfile,
    game: &OwnedGameData,
    limit: usize,
) -> Result<UserProfile, String> {
    let characters: BTreeMap<_, _> = game
        .cards
        .iter()
        .map(|card| (card.id, card.character_id))
        .collect();
    let mut groups: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    for card in &user.user_cards {
        let character = characters
            .get(&card.card_id)
            .ok_or("owned card absent from masterdata")?;
        groups.entry(*character).or_default().insert(card.card_id);
    }
    if groups.len() < 5 {
        return Err("oracle account has fewer than five distinct owned characters".to_owned());
    }
    let groups: Vec<_> = groups
        .values()
        .take(6)
        .map(|cards| cards.iter().copied().collect::<Vec<_>>())
        .collect();
    let mut selected = BTreeSet::new();
    for rank in 0..limit {
        for cards in &groups {
            if let Some(&card) = cards.get(rank) {
                selected.insert(card);
                if selected.len() == limit {
                    break;
                }
            }
        }
        if selected.len() == limit {
            break;
        }
    }
    let mut result = user.clone();
    result
        .user_cards
        .retain(|card| selected.contains(&card.card_id));
    Ok(result)
}

#[test]
#[ignore = "external private corpus; explicit data/output paths and controlled CPU required"]
fn real_world_bloom_matrix() -> Result<(), String> {
    let corpus = PathBuf::from(env("CORPUS", "testdata/real"));
    let masterdata = required_path("MASTERDATA")?;
    let music_metas = required_path("MUSIC_METAS")?;
    let output = required_path("OUTPUT")?;
    let region = env("REGION", "cn");
    let mode = env("MODE", "full");
    if mode != "full" && mode != "oracle" {
        return Err("MODE must be full or oracle".to_owned());
    }
    let repeats: usize = number("REPEATS", "3")?;
    let account_limit: usize = number("ACCOUNT_LIMIT", "4")?;
    let account_start: usize = number("ACCOUNT_START", "0")?;
    let timeout_ms: u64 = number("TIMEOUT_MS", if mode == "oracle" { "0" } else { "2000" })?;
    let subset_cards: usize = number("SUBSET_CARDS", "10")?;
    let oracle_dense_limit: usize = number("ORACLE_DENSE_LIMIT", "12")?;
    let include_catalog = flag("CATALOG", "0")?;
    let latency_limit_ms = optional_positive_number("LATENCY_LIMIT_MS")?;
    if include_catalog && mode != "full" {
        return Err("CATALOG=1 requires MODE=full; catalog inputs are never reduced".to_owned());
    }
    if repeats == 0
        || account_limit == 0
        || !(5..=16).contains(&subset_cards)
        || !(5..=16).contains(&oracle_dense_limit)
    {
        return Err(
            "REPEATS/ACCOUNT_LIMIT must be positive; SUBSET_CARDS/ORACLE_DENSE_LIMIT must be 5..=16".to_owned(),
        );
    }
    let mut topks = env("TOPKS", "1,8,30")
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<usize>()
                .map_err(|_| "invalid TOPKS".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    topks.sort_unstable();
    topks.dedup();
    if topks.is_empty() || topks.iter().any(|&k| !(1..=100).contains(&k)) {
        return Err("TOPKS must be 1..=100".to_owned());
    }
    let live_types = env("LIVE_TYPES", "multi,solo,auto")
        .split(',')
        .map(|s| match s.trim() {
            "multi" => Ok(LiveType::Multi),
            "solo" => Ok(LiveType::Solo),
            "auto" => Ok(LiveType::Auto),
            _ => Err("LIVE_TYPES supports multi,solo,auto".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if live_types.is_empty()
        || live_types
            .iter()
            .enumerate()
            .any(|(index, live)| live_types[..index].contains(live))
    {
        return Err("LIVE_TYPES must contain distinct values".to_owned());
    }
    let state_modes = env("STATE_MODES", "current")
        .split(',')
        .map(|value| match value.trim() {
            "current" => Ok(("current", true)),
            "both" => Ok(("both", false)),
            _ => Err("STATE_MODES supports current,both".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if state_modes.is_empty()
        || state_modes
            .iter()
            .enumerate()
            .any(|(index, state)| state_modes[..index].contains(state))
    {
        return Err("STATE_MODES must contain distinct values".to_owned());
    }
    let require_complete = flag("REQUIRE_COMPLETE", if mode == "oracle" { "1" } else { "0" })?;

    // Optional loader tables must not silently erase the WL route under test.
    for name in [
        "worldBlooms.json",
        "worldBloomDifferentAttributeBonuses.json",
        "eventCards.json",
        "eventDeckBonuses.json",
        "eventRarityBonusRates.json",
        "eventCardBonusLimits.json",
        "eventHonorBonuses.json",
        "eventSkillScoreUpLimits.json",
    ] {
        if !masterdata.join(name).is_file() {
            return Err(format!("missing required validation table {name}"));
        }
    }
    let game = OwnedGameData::load(&masterdata, &music_metas)?;
    if game.world_blooms.is_empty()
        || game.world_bloom_different_attribute_bonuses.is_empty()
        || game.music_metas.is_empty()
    {
        return Err("empty WL chapter, attribute bonus or music metadata".to_owned());
    }
    let real_wl_events: BTreeSet<_> = game
        .events
        .iter()
        .filter(|event| event.event_type == "world_bloom")
        .map(|event| event.id)
        .collect();
    let finale_events: BTreeSet<_> = game
        .world_blooms
        .iter()
        .filter(|row| {
            row.world_bloom_chapter_type.as_deref() == Some("finale")
                && real_wl_events.contains(&row.event_id)
        })
        .map(|row| row.event_id)
        .collect();
    let unsupported_finales: Vec<_> = finale_events
        .iter()
        .copied()
        .filter(|id| !is_world_bloom_finale_event(*id))
        .collect();
    let supported_finales: BTreeSet<_> = finale_events
        .iter()
        .copied()
        .filter(|id| is_world_bloom_finale_event(*id))
        .collect();
    let finale = optional_id("FINALE_EVENT_ID")?
        .or_else(|| supported_finales.last().copied())
        .ok_or("no supported real finale in masterdata")?;
    if !supported_finales.contains(&finale) {
        return Err("FINALE_EVENT_ID is not a supported real finale row".to_owned());
    }
    let chapters: BTreeSet<_> = game
        .world_blooms
        .iter()
        .filter(|row| {
            real_wl_events.contains(&row.event_id)
                && !finale_events.contains(&row.event_id)
                && row.game_character_id.is_some()
        })
        .map(|row| {
            (
                row.event_id,
                row.chapter_no,
                row.game_character_id.unwrap_or_default(),
            )
        })
        .collect();
    let selected_event = optional_id("WL_EVENT_ID")?
        .or_else(|| chapters.iter().next_back().map(|row| row.0))
        .ok_or("no ordinary real WL chapter")?;
    let chapter = chapters
        .iter()
        .find(|row| row.0 == selected_event)
        .copied()
        .ok_or("WL_EVENT_ID has no ordinary chapter")?;
    let music_id = optional_id("MUSIC_ID")?;
    let difficulty = std::env::var("ALLIUM_REAL_MUSIC_DIFF").ok();
    let music = game
        .music_metas
        .iter()
        .filter(|row| {
            music_id.is_none_or(|id| row.music_id == id)
                && difficulty
                    .as_ref()
                    .is_none_or(|diff| &row.difficulty == diff)
        })
        .min_by_key(|row| (row.music_id, row.difficulty.as_str()))
        .ok_or("requested music metadata absent")?;

    let manifest: Manifest = serde_json::from_str(
        &fs::read_to_string(corpus.join("manifest.json"))
            .map_err(|_| "cannot read corpus manifest")?,
    )
    .map_err(|error| error.to_string())?;
    let mut suites = BTreeSet::new();
    let mut profiles = BTreeSet::new();
    let mut accounts = Vec::new();
    let mut excluded_region = 0;
    for (ordinal, entry) in manifest.cases.iter().enumerate() {
        if suites.contains(&entry.suite_file) {
            continue;
        }
        let path = PathBuf::from(&entry.input_path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err("invalid corpus input path".to_owned());
        }
        let mut input: LegacyInput = serde_json::from_str(
            &fs::read_to_string(corpus.join(path))
                .map_err(|_| format!("cannot read manifest entry {}", ordinal + 1))?,
        )
        .map_err(|error| format!("invalid manifest entry {}: {error}", ordinal + 1))?;
        suites.insert(entry.suite_file.clone());
        if input.region != region {
            excluded_region += 1;
            continue;
        }
        // Reuse only account conversion. Request fields below are intentionally derived.
        input.target = "score".to_owned();
        input.live_type = "multi".to_owned();
        let (_, user, _) = transform_input(&input)
            .map_err(|error| format!("account conversion at entry {}: {error}", ordinal + 1))?;
        let canonical = serde_json::to_string(&user).map_err(|error| error.to_string())?;
        if profiles.insert(canonical) {
            accounts.push(Account {
                source_account: accounts.len() + 1,
                is_catalog: false,
                original_owned_cards: user.user_cards.len(),
                user,
                manifest_ordinal: Some(ordinal + 1),
                original_unique_characters: 0,
                applicable: true,
            });
        }
    }
    let distinct_accounts = accounts.len();
    if distinct_accounts == 0 && !include_catalog {
        return Err("no distinct accounts for REGION".to_owned());
    }
    if (distinct_accounts > 0 && account_start >= distinct_accounts)
        || (distinct_accounts == 0 && account_start != 0)
    {
        return Err("ACCOUNT_START lies outside the distinct account inventory".to_owned());
    }
    let mut accounts: Vec<_> = accounts
        .into_iter()
        .skip(account_start)
        .take(account_limit)
        .collect();
    let selected_real_accounts = accounts.len();
    if include_catalog {
        let user = catalog_user(&game)?;
        accounts.push(Account {
            original_owned_cards: user.user_cards.len(),
            user,
            manifest_ordinal: None,
            source_account: 0,
            is_catalog: true,
            original_unique_characters: 0,
            applicable: true,
        });
    }
    let master_characters: BTreeMap<_, _> = game
        .cards
        .iter()
        .map(|card| (card.id, card.character_id))
        .collect();
    let mut not_applicable_records = Vec::new();
    for account in &mut accounts {
        let characters = account
            .user
            .user_cards
            .iter()
            .map(|card| {
                master_characters
                    .get(&card.card_id)
                    .copied()
                    .ok_or_else(|| {
                        format!(
                            "account {} owns cards absent from masterdata",
                            account.source_account
                        )
                    })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        account.original_unique_characters = characters.len();
        // All derived requests here require five distinct characters. This is
        // corpus eligibility, not a solver result or a successful empty search.
        // Apply before reducing oracle input, and never reduce a full input.
        if characters.len() < 5 {
            account.applicable = false;
            not_applicable_records.push(json!({
                "record":"not_applicable", "scope":if account.is_catalog {"profile"} else {"account"}, "account":account.account_id(),
                "manifest_ordinal":account.manifest_ordinal,
                "source":account.source(mode == "oracle"), "validation_mode":mode,
                "reason":"fewer_than_five_distinct_owned_characters",
                "owned_cards":account.original_owned_cards, "unique_characters":characters.len(),
                "feasible_set_empty":true, "search_executed":false,
                "complete":false, "completion":"not_applicable", "deadline":false,
                "stats":null, "results":[],
            }));
            continue;
        }
        if mode == "oracle" {
            account.user = reduced_user(&account.user, &game, subset_cards)?;
        }
    }
    let mut cases = Vec::new();
    for (account, source) in accounts.iter().enumerate() {
        if !source.applicable {
            continue;
        }
        let leader = source
            .user
            .user_cards
            .iter()
            .filter_map(|owned| {
                game.cards
                    .iter()
                    .find(|card| card.id == owned.card_id)
                    .map(|card| card.character_id)
            })
            .min()
            .ok_or("account has no owned card")?;
        for &live_type in &live_types {
            for (label, event, character, forced) in [
                ("wl_chapter", chapter.0, Some(chapter.2), None),
                ("final_fixed", finale, None, Some(leader)),
                ("final_auto", finale, None, None),
            ] {
                for &(state_mode, keep_after_training_state) in &state_modes {
                    cases.push(Case {
                        account,
                        mode: label,
                        state_mode,
                        params: BuildParams {
                            region: region.clone(),
                            event_id: Some(event),
                            world_bloom_character_id: character,
                            forced_leader_character_id: forced,
                            live_type,
                            target: ScoreTarget::Score,
                            music_id: Some(music.music_id),
                            music_diff: Some(music.difficulty.clone()),
                            live_skill_order: LiveSkillOrder::Average,
                            keep_after_training_state,
                            ..BuildParams::default()
                        },
                    });
                }
            }
        }
    }
    // Explicit output path and create_new keep runs from overwriting one another.
    let mut out = BufWriter::new(
        File::options()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|error| error.to_string())?,
    );
    emit(
        &mut out,
        json!({"record":"inventory", "source":if include_catalog {"real_accounts_and_full_catalog_derived_requests"} else {"legacy_real_accounts_derived_requests"}, "mode":mode, "region":region, "manifest_entries":manifest.cases.len(), "distinct_suites":suites.len(), "distinct_account_profiles":distinct_accounts, "excluded_region_suites":excluded_region, "selected_accounts":selected_real_accounts, "catalog_profiles":usize::from(include_catalog), "selected_profiles":accounts.len(), "catalog_card_ids":if include_catalog {Some(game.cards.len())} else {None}, "catalog_cultivation":"card level/skill/master/episode maxima from loaded masterdata; no area/rank/honor/gate/canvas additions", "state_modes":state_modes.iter().map(|state| state.0).collect::<Vec<_>>(), "state_modes_do_not_multiply_account_count":true, "cases":cases.len(), "repeats":repeats, "topks":topks, "timeout_ms":timeout_ms, "latency_limit_ms":latency_limit_ms, "require_complete":require_complete, "wl_event":chapter.0, "wl_chapter":chapter.1, "wl_character":chapter.2, "finale_event":finale, "unsupported_finale_events_excluded":unsupported_finales, "music_id":music.music_id, "music_diff":music.difficulty, "subset_cards":if mode=="oracle" {Some(subset_cards)} else {None}, "oracle_dense_limit":if mode=="oracle" {Some(oracle_dense_limit)} else {None}, "adapter":"testdata_adapter::transform_input; legacy support/deck selections not captured", "support_bonus_source":"production loader: external tables or embedded versioned WL tables", "timing_scope":"build_card_pool + complete search pipeline; input/masterdata loading, oracle and output excluded"}),
    )?;
    emit(
        &mut out,
        json!({"record":"account_coverage", "account_start":account_start, "account_limit":account_limit, "selected_accounts":selected_real_accounts, "applicable_accounts":accounts.iter().filter(|account| account.applicable && !account.is_catalog).count(), "not_applicable_accounts":accounts.iter().filter(|account| !account.applicable && !account.is_catalog).count(), "catalog_profiles":usize::from(include_catalog), "selection_excluded_before":account_start, "selection_excluded_after":distinct_accounts.saturating_sub(account_start + selected_real_accounts), "not_applicable_is_not_solver_success":true}),
    )?;
    for record in not_applicable_records {
        emit(&mut out, record)?;
    }
    let mut supported_accounts = BTreeSet::new();
    let mut unsupported_accounts = BTreeSet::new();
    let mut supported_cases = BTreeSet::new();
    let mut unsupported_cases = BTreeSet::new();
    let mut completed = 0usize;
    let mut timed_out = 0usize;
    let mut warmup_timed_out = 0usize;
    let mut measured_over_latency = 0usize;
    let mut warmup_over_latency = 0usize;
    let mut latency_failures = BTreeMap::<(usize, usize), (usize, usize, f64)>::new();
    let mut complete_reference = BTreeMap::<(usize, usize), Vec<Value>>::new();
    let catalog_ids: BTreeSet<_> = game.cards.iter().map(|card| card.id).collect();
    // Round zero is a recorded warmup; later rounds interleave every case/Top-K.
    for round in 0..=repeats {
        for (case_index, case) in cases.iter().enumerate() {
            for &top_k in &topks {
                let account = &accounts[case.account];
                let mut build_params = case.params.clone();
                build_params.limit = top_k;
                let started = Instant::now();
                let (pool, ctx) = match build_card_pool(
                    &account.user,
                    &game.as_ref(),
                    &build_params,
                ) {
                    Ok(built) => built,
                    Err(BuildError::TooManyCards(count)) if mode == "full" => {
                        unsupported_accounts.insert(case.account);
                        unsupported_cases.insert(case_index);
                        emit(
                            &mut out,
                            json!({"record":"unsupported", "account":account.account_id(), "manifest_ordinal":account.manifest_ordinal, "case":case_index+1, "source":account.source(false), "state_mode":case.state_mode, "keep_after_training_state":case.params.keep_after_training_state, "phase":if round==0 {"warmup"} else {"measured"}, "round":round, "event":case.params.event_id, "mode":case.mode, "live_type":case.params.live_type, "top_k":top_k, "owned_cards":account.user.user_cards.len(), "unique_characters":account.original_unique_characters, "pool":count, "complete":false, "completion":"unsupported_capacity", "deadline":false, "stats":null, "results":[], "error":"TooManyCards", "wall_ms":started.elapsed().as_secs_f64()*1000.0}),
                        )?;
                        continue;
                    }
                    Err(error) => {
                        return Err(format!("case {} build failed: {error}", case_index + 1));
                    }
                };
                let build_ms = started.elapsed().as_secs_f64() * 1000.0;
                supported_accounts.insert(case.account);
                supported_cases.insert(case_index);
                if !ctx.is_world_bloom || ctx.is_final_chapter != (case.mode != "wl_chapter") {
                    return Err(format!("case {} took the wrong route", case_index + 1));
                }
                if account.is_catalog {
                    let built_ids: BTreeSet<_> = pool
                        .indices()
                        .map(|card| i32::from(pool.game_id(card)))
                        .collect();
                    if built_ids != catalog_ids {
                        return Err(format!(
                            "catalog case {} did not retain every masterdata card identity",
                            case_index + 1
                        ));
                    }
                }
                if mode == "oracle" && pool.count() > oracle_dense_limit {
                    return Err(format!(
                        "case {} state {} has dense pool {}; ORACLE_DENSE_LIMIT={oracle_dense_limit}; explicitly lower SUBSET_CARDS or raise the oracle limit up to 16",
                        case_index + 1,
                        case.state_mode,
                        pool.count()
                    ));
                }
                let params = SearchParams { top_k, timeout_ms };
                let search_started = Instant::now();
                let outcome = search(&pool, &ctx, &params);
                let search_ms = search_started.elapsed().as_secs_f64() * 1000.0;
                // Only the two engine operations are timed. Route/identity checks,
                // oracle enumeration, result serialization and output are excluded.
                let wall_ms = build_ms + search_ms;
                if round > 0 {
                    if outcome.completion() == SearchCompletion::Complete {
                        completed += 1;
                    } else {
                        timed_out += 1;
                    }
                } else if outcome.completion() != SearchCompletion::Complete {
                    warmup_timed_out += 1;
                }
                let latency_met = latency_limit_ms.map(|limit| wall_ms < limit);
                if latency_met == Some(false) {
                    let entry = latency_failures.entry((case_index, top_k)).or_default();
                    if round == 0 {
                        warmup_over_latency += 1;
                        entry.0 += 1;
                    } else {
                        measured_over_latency += 1;
                        entry.1 += 1;
                    }
                    entry.2 = entry.2.max(wall_ms);
                }
                let results = result_rows(&pool, &outcome.results);
                if outcome.completion() == SearchCompletion::Complete {
                    if let Some(previous) = complete_reference.get(&(case_index, top_k)) {
                        if previous != &results {
                            return Err(format!(
                                "case {} Top-{top_k} changed across complete runs",
                                case_index + 1
                            ));
                        }
                    } else {
                        complete_reference.insert((case_index, top_k), results.clone());
                    }
                }
                let mut oracle = Value::Null;
                let mut oracle_matches = true;
                if mode == "oracle" {
                    let oracle_started = Instant::now();
                    // ExactOracle enumerates *every ordered slot assignment*, including
                    // each legal finale leader, with the original per-leader support
                    // context. It does not use production dominance/bounds/placement.
                    let (expected, stats) = ExactOracle::new(&pool, &ctx).search(&params);
                    let expected = result_rows(&pool, &expected);
                    oracle_matches =
                        outcome.completion() == SearchCompletion::Complete && results == expected;
                    oracle = json!({"strategy":"full_ordered_enumeration_with_explicit_leader_slot", "matches":oracle_matches, "wall_ms":oracle_started.elapsed().as_secs_f64()*1000.0, "candidates":stats.candidates, "evaluated":stats.evaluated, "invalid":stats.invalid, "results":expected});
                }
                emit(
                    &mut out,
                    json!({"record":"sample", "account":account.account_id(), "manifest_ordinal":account.manifest_ordinal, "case":case_index+1, "source":account.source(mode == "oracle"), "state_mode":case.state_mode, "keep_after_training_state":case.params.keep_after_training_state, "phase":if round==0 {"warmup"} else {"measured"}, "round":round, "event":case.params.event_id, "mode":case.mode, "live_type":case.params.live_type, "top_k":top_k, "owned_cards":account.user.user_cards.len(), "original_owned_cards":account.original_owned_cards, "original_unique_characters":account.original_unique_characters, "pool":pool.count(), "complete":outcome.completion()==SearchCompletion::Complete, "completion":outcome.completion(), "deadline":outcome.stats.deadline_hit, "stats":outcome.stats, "route":{"is_world_bloom":ctx.is_world_bloom, "is_final_chapter":ctx.is_final_chapter, "forced_leader":ctx.forced_leader_character_id, "support_count":ctx.support_deck.count, "support_candidates":ctx.support_deck.cards.len(), "leader_support_counts":ctx.support_decks_by_character.iter().map(|deck| deck.count).collect::<Vec<_>>(), "leader_support_candidates":ctx.support_decks_by_character.iter().map(|deck| deck.cards.len()).collect::<Vec<_>>()}, "results":results, "build_ms":build_ms, "search_ms":search_ms, "wall_ms":wall_ms, "latency_limit_ms":latency_limit_ms, "latency_met":latency_met, "oracle":oracle}),
                )?;
                if !oracle_matches {
                    return Err(format!(
                        "case {} Top-{top_k} failed complete ordered oracle comparison",
                        case_index + 1
                    ));
                }
            }
        }
    }
    let latency_failure_rows: Vec<_> = latency_failures
        .iter()
        .map(
            |(&(case_index, top_k), &(warmups, measured, max_wall_ms))| {
                let case = &cases[case_index];
                let account = &accounts[case.account];
                json!({
                    "case":case_index + 1, "top_k":top_k,
                    "account":account.account_id(), "source":account.source(mode == "oracle"),
                    "mode":case.mode, "state_mode":case.state_mode,
                    "live_type":case.params.live_type,
                    "warmup_over_limit":warmups, "measured_over_limit":measured,
                    "max_wall_ms":max_wall_ms,
                })
            },
        )
        .collect();
    let all_applicable_complete =
        timed_out == 0 && warmup_timed_out == 0 && unsupported_cases.is_empty() && completed > 0;
    let latency_passed = latency_failures.is_empty();
    let gate_passed = all_applicable_complete && latency_passed;
    emit(
        &mut out,
        json!({"record":"summary", "selected_accounts":selected_real_accounts, "catalog_profiles":usize::from(include_catalog), "selected_profiles":accounts.len(), "account_start":account_start, "not_applicable_accounts":accounts.iter().filter(|account| !account.applicable && !account.is_catalog).count(), "accounts_with_supported_cases":supported_accounts.iter().filter(|&&index| !accounts[index].is_catalog).count(), "accounts_with_unsupported_cases":unsupported_accounts.iter().filter(|&&index| !accounts[index].is_catalog).count(), "catalog_profiles_with_supported_cases":supported_accounts.iter().filter(|&&index| accounts[index].is_catalog).count(), "catalog_profiles_with_unsupported_cases":unsupported_accounts.iter().filter(|&&index| accounts[index].is_catalog).count(), "supported_cases":supported_cases.len(), "unsupported_capacity_cases":unsupported_cases.len(), "measured_complete":completed, "measured_timed_out":timed_out, "warmup_timed_out":warmup_timed_out, "latency_limit_ms":latency_limit_ms, "measured_over_latency_limit":measured_over_latency, "warmup_over_latency_limit":warmup_over_latency, "over_latency_cases":latency_failure_rows, "note":"state modes are request axes, not extra accounts; catalog is additional derived input; no full-mode card reduction is performed"}),
    )?;
    emit(
        &mut out,
        json!({
            "record": "gate", "require_complete": require_complete,
            "classification": if !require_complete { "diagnostic_collection" } else if mode == "oracle" { "ordered_oracle_equivalence" } else { "complete_stability" },
            "complete_result_stability_checked": true,
            "oracle_equivalence_checked": mode == "oracle",
            "all_applicable_complete": all_applicable_complete,
            "latency_limit_ms": latency_limit_ms,
            "latency_comparison": "wall_ms < latency_limit_ms",
            "latency_gate_includes_warmup": true,
            "latency_passed": latency_limit_ms.map(|_| latency_passed),
            "measured_over_latency_limit": measured_over_latency,
            "warmup_over_latency_limit": warmup_over_latency,
            "gate_passed": require_complete.then_some(gate_passed),
            "accepted": require_complete && gate_passed,
        }),
    )?;
    if require_complete && !gate_passed {
        return Err(format!(
            "strict validation failed: measured_timed_out={timed_out}, warmup_timed_out={warmup_timed_out}, unsupported_cases={}, completed={completed}, measured_over_latency={measured_over_latency}, warmup_over_latency={warmup_over_latency}",
            unsupported_cases.len(),
        ));
    }
    Ok(())
}
