//! 全模式性能扫描：对每种 live 类型 / 活动形态 / 打分目标各跑一次建池 + 搜索，
//! 打印候选池规模、单角色最大保留数与耗时，用于定位最重的模式。
//!
//! `cargo run --release --example mode_sweep`
//!
//! 数据来自 benches 下的合成 masterdata，不依赖任何游戏数据。

#[path = "../benches/synth_masterdata/mod.rs"]
mod synth_masterdata;

use allium_deck::engine::{
    MasterdataSources, OwnedGameData, parse_build_params_json, parse_user_profile_json,
};
use allium_deck::handler::build_card_pool;
use allium_deck::search::{SearchParams, search_instrumented};
use std::time::Instant;

/// 每个模式一行：名字 + params JSON。
const MODES: &[(&str, &str)] = &[
    (
        "marathon/multi/score",
        r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"score","limit":8}"#,
    ),
    (
        "marathon/solo/score",
        r#"{"eventId":1,"eventType":"marathon","liveType":"solo","target":"score","limit":8}"#,
    ),
    (
        "marathon/auto/score",
        r#"{"eventId":1,"eventType":"marathon","liveType":"auto","target":"score","limit":8}"#,
    ),
    (
        "cheerful/multi/score",
        r#"{"eventId":2,"eventType":"cheerful_carnival","liveType":"multi","target":"score","limit":8}"#,
    ),
    (
        "marathon/multi/bonus",
        r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"bonus","limit":8}"#,
    ),
    (
        "marathon/multi/power",
        r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"power","limit":8}"#,
    ),
    (
        "marathon/multi/skill",
        r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"skill","limit":8}"#,
    ),
    (
        "no_event/multi/score",
        r#"{"liveType":"multi","target":"score","limit":8,"attrFilter":"cool"}"#,
    ),
    (
        "challenge_all/score",
        r#"{"liveType":"challenge","target":"score","limit":8}"#,
    ),
    (
        "challenge_one/score",
        r#"{"liveType":"challenge","target":"score","limit":8,"challengeLiveCharacterId":1}"#,
    ),
    (
        "wl_chapter/multi/score",
        r#"{"liveType":"multi","target":"score","limit":8,"worldBloomEventTurn":3,"worldBloomCharacterId":1}"#,
    ),
    (
        "wl_finale/multi/score",
        r#"{"liveType":"multi","target":"score","limit":8,"worldBloomFinaleTurn":3,"attrFilter":"cool"}"#,
    ),
];

const SEARCH_PARAMS: SearchParams = SearchParams {
    top_k: 8,
    timeout_ms: 600_000,
};

fn main() {
    let synth = synth_masterdata::generate(synth_masterdata::DEFAULT_SEED);
    let sources = MasterdataSources::from_strings(synth.tables, synth.music_metas_json);
    let owned = OwnedGameData::from_sources(&sources).expect("synth masterdata parses");
    let user = parse_user_profile_json(&synth.user_json).expect("synth user parses");
    let game = owned.as_ref();

    println!(
        "{:<24} {:>6} {:>8} {:>10} {:>12} {:>12} {:>6} {:>16}",
        "mode", "pool", "max/char", "build_ms", "search_ms", "leaf_nodes", "decks", "top1_score"
    );
    for (name, json) in MODES {
        let params = match parse_build_params_json(json) {
            Ok(params) => params,
            Err(err) => {
                println!("{name:<24} params error: {err}");
                continue;
            }
        };

        let build_start = Instant::now();
        let built = build_card_pool(&user, &game, &params);
        let build_ms = build_start.elapsed().as_secs_f64() * 1000.0;
        let (pool, ctx) = match built {
            Ok(built) => built,
            Err(err) => {
                println!("{name:<24} build error: {err}");
                continue;
            }
        };

        // 单角色保留上限：定位建池裁剪的实际强度。
        let mut per_char = [0usize; 27];
        for card in pool.indices() {
            per_char[pool.char_id(card) as usize] += 1;
        }
        let max_per_char = per_char.iter().copied().max().unwrap_or(0);

        // challenge_all 的逐角色分流已下沉到 search_instrumented，
        // 所有模式共用同一个入口。
        let search_start = Instant::now();
        let (results, stats) = search_instrumented(&pool, &ctx, &SEARCH_PARAMS);
        let search_ms = search_start.elapsed().as_secs_f64() * 1000.0;

        println!(
            "{name:<24} {:>6} {max_per_char:>8} {build_ms:>10.2} {search_ms:>12.2} {:>12} {:>6} {:>16}",
            pool.count(),
            stats.leaf_nodes,
            results.len(),
            results.first().map(|result| result.score).unwrap_or(0),
        );
    }
}
