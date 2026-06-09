//! allium-deck standalone 验证 CLI。
//!
//! 用途：脱离渲染层快速验证「建池 + 搜索」的改动——给一组 masterdata/music_metas/user/params，
//! 打印推荐卡组和分阶段耗时（建池 vs 搜索），方便核对 P3 建池性能和 P1/P2 语义改动。
//!
//! 它只用 allium-deck 的公开入口（engine + handler + search），不碰渲染、不碰服务层。
//! 放在 src/bin/ 下，`cargo install allium-deck` 即得 recommend_cli 命令。
//!
//! 运行（standalone Docker 内或本机 `cargo run -p allium-deck --bin recommend_cli --release --`）：
//!   recommend_cli \
//!     --masterdata <dir> \
//!     --music-metas <music_metas.json> \
//!     --user <user.json> \
//!     --params <params.json>
//!
//! params.json 形如：
//!   { "region":"cn", "eventId":160, "liveType":"multi", "target":"score",
//!     "musicId":74, "musicDiff":"expert" }

use std::path::PathBuf;
use std::time::Instant;

use allium_deck::engine::{parse_build_params_json, parse_user_profile_json, OwnedGameData};
use allium_deck::handler::build_card_pool;
use allium_deck::search::{search_instrumented, SearchParams};

fn arg(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == flag {
            return args.next();
        }
        if let Some(rest) = a.strip_prefix(&format!("{flag}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

fn require(flag: &str) -> Result<String, String> {
    arg(flag).ok_or_else(|| format!("缺少参数 {flag}"))
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("读取 {path} 失败: {e}"))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("错误: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let masterdata = require("--masterdata")?;
    let music_metas = require("--music-metas")?;
    let user_path = require("--user")?;
    let params_path = require("--params")?;
    let top_k: usize = arg("--top-k").and_then(|v| v.parse().ok()).unwrap_or(5);
    let timeout_ms: u64 = arg("--timeout-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000);

    // 1) 加载 masterdata（一次性，对应服务里的 OwnedGameData 缓存）。
    let load_start = Instant::now();
    let owned = OwnedGameData::load(&PathBuf::from(&masterdata), &PathBuf::from(&music_metas))?;
    let game = owned.as_ref();
    eprintln!("[load] masterdata+music_metas: {:.1}ms", ms(load_start));

    // 2) 解析 user / params。params 走真实 camelCase 解析器（与 recommend_json 同一路径，
    //    顺带复现 P2：card_configs 当前不被解析）。
    let user = parse_user_profile_json(&read(&user_path)?).map_err(|e| e.to_string())?;
    let params = parse_build_params_json(&read(&params_path)?)
        .map_err(|e| format!("解析 params 失败: {e}"))?;

    // 3) 建池（P3 关注点：这里通常是大头）。
    let build_start = Instant::now();
    let (pool, ctx) = build_card_pool(&user, &game, &params).map_err(|e| e.to_string())?;
    let build_ms = ms(build_start);
    eprintln!(
        "[build_pool] {:.1}ms  pool={} 张候选卡",
        build_ms,
        pool.count()
    );

    // WL 支援卡组诊断（P1）：count + 非零 bonus 数 + top3，用于确认支援加成不再恒为 0。
    {
        let sd = &ctx.support_deck;
        let nonzero = sd.cards.iter().filter(|(_, b)| *b > 0.0).count();
        let top: Vec<String> = sd
            .cards
            .iter()
            .take(3)
            .map(|(id, b)| format!("{id}:{b:.1}"))
            .collect();
        eprintln!(
            "[support_deck] count={} 候选={} 非零bonus={} top3=[{}]",
            sd.count,
            sd.cards.len(),
            nonzero,
            top.join(", ")
        );
    }

    // 4) 搜索 + 剪枝统计。
    let search_params = SearchParams { top_k, timeout_ms };
    let search_start = Instant::now();
    let (results, stats) = search_instrumented(&pool, &ctx, &search_params);
    let search_ms = ms(search_start);
    eprintln!(
        "[search] {:.1}ms  leaf={} ub_prunes={} ep_explored={} mono_break={}",
        search_ms, stats.leaf_nodes, stats.ub_prunes, stats.ep_explored, stats.mono_break_prunes
    );
    eprintln!("[total] build+search = {:.1}ms", build_ms + search_ms);

    // 5) 打印结果（game_id + score）。
    println!("# top {} decks", results.len());
    for (rank, deck) in results.iter().enumerate() {
        let cards: Vec<u16> = deck.cards.iter().map(|c| pool.game_id(*c)).collect();
        println!(
            "{:>2}. score={:<14} cards={:?}",
            rank + 1,
            deck.score,
            cards
        );
    }
    Ok(())
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
