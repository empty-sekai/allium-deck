//! Opt-in real-account measurements with explicitly derived WL requests.
//!
//! Run `cargo test --release --test validation_real -- --ignored --nocapture`.
//! Required: ALLIUM_REAL_MASTERDATA, ALLIUM_REAL_MUSIC_METAS, ALLIUM_REAL_OUTPUT.
//! Optional: ALLIUM_REAL_CORPUS (testdata/real), REGION (cn), REPEATS (3),
//! TOPKS (1,8,30), ACCOUNT_START (0; distinct-account offset), ACCOUNT_LIMIT (4),
//! TIMEOUT_MS (2000), MODE (full|oracle),
//! SUBSET_CARDS (10; oracle only, 5..=12), LIVE_TYPES (multi,solo,auto),
//! WL_EVENT_ID, FINALE_EVENT_ID, MUSIC_ID and MUSIC_DIFF (prefix ALLIUM_REAL_).
//! Output never includes corpus paths, suite names, account IDs or user data.
//! These are real-account-derived requests, not captured WL requests. Oracle
//! mode additionally reduces the owned collection, including its support pool.

#![allow(dead_code)]

mod testdata_adapter;

use allium_deck::engine::OwnedGameData;
use allium_deck::handler::{BuildError, BuildParams, UserProfile, build_card_pool};
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
    manifest_ordinal: usize,
    source_account: usize,
    original_owned_cards: usize,
    original_unique_characters: usize,
    applicable: bool,
}

struct Case {
    account: usize,
    mode: &'static str,
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
    if repeats == 0 || account_limit == 0 || !(5..=12).contains(&subset_cards) {
        return Err(
            "REPEATS/ACCOUNT_LIMIT must be positive; SUBSET_CARDS must be 5..=12".to_owned(),
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
    let require_complete = env("REQUIRE_COMPLETE", if mode == "oracle" { "1" } else { "0" }) == "1";

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
                original_owned_cards: user.user_cards.len(),
                user,
                manifest_ordinal: ordinal + 1,
                original_unique_characters: 0,
                applicable: true,
            });
        }
    }
    let distinct_accounts = accounts.len();
    if distinct_accounts == 0 {
        return Err("no distinct accounts for REGION".to_owned());
    }
    if account_start >= distinct_accounts {
        return Err("ACCOUNT_START lies outside the distinct account inventory".to_owned());
    }
    let mut accounts: Vec<_> = accounts
        .into_iter()
        .skip(account_start)
        .take(account_limit)
        .collect();
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
                "record":"not_applicable", "scope":"account", "account":account.source_account,
                "manifest_ordinal":account.manifest_ordinal,
                "source":"real_account_derived_request", "validation_mode":mode,
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
                cases.push(Case {
                    account,
                    mode: label,
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
                        keep_after_training_state: true,
                        ..BuildParams::default()
                    },
                });
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
        json!({"record":"inventory", "source":"legacy_real_accounts_derived_requests", "mode":mode, "region":region, "manifest_entries":manifest.cases.len(), "distinct_suites":suites.len(), "distinct_account_profiles":distinct_accounts, "excluded_region_suites":excluded_region, "selected_accounts":accounts.len(), "cases":cases.len(), "repeats":repeats, "topks":topks, "timeout_ms":timeout_ms, "wl_event":chapter.0, "wl_chapter":chapter.1, "wl_character":chapter.2, "finale_event":finale, "unsupported_finale_events_excluded":unsupported_finales, "music_id":music.music_id, "music_diff":music.difficulty, "subset_cards":if mode=="oracle" {Some(subset_cards)} else {None}, "adapter":"testdata_adapter::transform_input; legacy support/deck selections not captured", "support_bonus_source":"production loader: external tables or embedded versioned WL tables", "timing_scope":"build_card_pool + complete search pipeline; input/masterdata loading excluded"}),
    )?;
    emit(
        &mut out,
        json!({"record":"account_coverage", "account_start":account_start, "account_limit":account_limit, "selected_accounts":accounts.len(), "applicable_accounts":accounts.iter().filter(|account| account.applicable).count(), "not_applicable_accounts":not_applicable_records.len(), "selection_excluded_before":account_start, "selection_excluded_after":distinct_accounts.saturating_sub(account_start + accounts.len()), "not_applicable_is_not_solver_success":true}),
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
    let mut complete_reference = BTreeMap::<(usize, usize), Vec<Value>>::new();
    // Round zero is a recorded warmup; later rounds interleave every case/Top-K.
    for round in 0..=repeats {
        for (case_index, case) in cases.iter().enumerate() {
            for &top_k in &topks {
                let account = &accounts[case.account];
                let started = Instant::now();
                let (pool, ctx) = match build_card_pool(&account.user, &game.as_ref(), &case.params)
                {
                    Ok(built) => built,
                    Err(BuildError::TooManyCards(count)) if mode == "full" => {
                        unsupported_accounts.insert(case.account);
                        unsupported_cases.insert(case_index);
                        emit(
                            &mut out,
                            json!({"record":"unsupported", "account":account.source_account, "manifest_ordinal":account.manifest_ordinal, "case":case_index+1, "source":"real_account_derived_request", "phase":if round==0 {"warmup"} else {"measured"}, "round":round, "event":case.params.event_id, "mode":case.mode, "live_type":case.params.live_type, "top_k":top_k, "owned_cards":account.user.user_cards.len(), "unique_characters":account.original_unique_characters, "pool":count, "complete":false, "completion":"unsupported_capacity", "deadline":false, "stats":null, "results":[], "error":"TooManyCards", "wall_ms":started.elapsed().as_secs_f64()*1000.0}),
                        )?;
                        continue;
                    }
                    Err(error) => {
                        return Err(format!("case {} build failed: {error}", case_index + 1));
                    }
                };
                supported_accounts.insert(case.account);
                supported_cases.insert(case_index);
                let build_ms = started.elapsed().as_secs_f64() * 1000.0;
                if !ctx.is_world_bloom || ctx.is_final_chapter != (case.mode != "wl_chapter") {
                    return Err(format!("case {} took the wrong route", case_index + 1));
                }
                if mode == "oracle" && pool.count() > 12 {
                    return Err(format!(
                        "case {} has unsupported validation pool size {}",
                        case_index + 1,
                        pool.count()
                    ));
                }
                let params = SearchParams { top_k, timeout_ms };
                let search_started = Instant::now();
                let outcome = search(&pool, &ctx, &params);
                if round > 0 {
                    if outcome.completion() == SearchCompletion::Complete {
                        completed += 1;
                    } else {
                        timed_out += 1;
                    }
                }
                let search_ms = search_started.elapsed().as_secs_f64() * 1000.0;
                let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
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
                    json!({"record":"sample", "account":account.source_account, "manifest_ordinal":account.manifest_ordinal, "case":case_index+1, "source":if mode=="oracle" {"real_account_derived_card_subset_and_request"} else {"real_account_derived_request"}, "phase":if round==0 {"warmup"} else {"measured"}, "round":round, "event":case.params.event_id, "mode":case.mode, "live_type":case.params.live_type, "top_k":top_k, "owned_cards":account.user.user_cards.len(), "original_owned_cards":account.original_owned_cards, "original_unique_characters":account.original_unique_characters, "pool":pool.count(), "complete":outcome.completion()==SearchCompletion::Complete, "completion":outcome.completion(), "deadline":outcome.stats.deadline_hit, "stats":outcome.stats, "route":{"is_world_bloom":ctx.is_world_bloom, "is_final_chapter":ctx.is_final_chapter, "forced_leader":ctx.forced_leader_character_id, "support_count":ctx.support_deck.count, "support_candidates":ctx.support_deck.cards.len(), "leader_support_counts":ctx.support_decks_by_character.iter().map(|deck| deck.count).collect::<Vec<_>>(), "leader_support_candidates":ctx.support_decks_by_character.iter().map(|deck| deck.cards.len()).collect::<Vec<_>>()}, "results":results, "build_ms":build_ms, "search_ms":search_ms, "wall_ms":wall_ms, "oracle":oracle}),
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
    emit(
        &mut out,
        json!({"record":"summary", "selected_accounts":accounts.len(), "account_start":account_start, "not_applicable_accounts":accounts.iter().filter(|account| !account.applicable).count(), "accounts_with_supported_cases":supported_accounts.len(), "accounts_with_unsupported_cases":unsupported_accounts.len(), "supported_cases":supported_cases.len(), "unsupported_capacity_cases":unsupported_cases.len(), "measured_complete":completed, "measured_timed_out":timed_out, "note":"not-applicable accounts and capacity errors are not successful searches; no full-mode card reduction is performed"}),
    )?;
    emit(
        &mut out,
        json!({
            "record": "gate", "require_complete": require_complete,
        "classification": if mode == "oracle" { "ordered_oracle_equivalence" } else if require_complete { "complete_stability" } else { "deadline_and_capacity_diagnostics" },
            "complete_result_stability_checked": true,
            "all_applicable_complete": timed_out == 0 && unsupported_cases.is_empty() && completed > 0,
        }),
    )?;
    if require_complete && (timed_out > 0 || !unsupported_cases.is_empty() || completed == 0) {
        return Err(
            "complete validation required but some applicable cases did not complete".to_owned(),
        );
    }
    Ok(())
}
