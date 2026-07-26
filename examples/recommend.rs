//! `engine::recommend_json` 最小可运行示例：JSON 入、JSON 出。
//!
//! ```text
//! cargo run --release --example recommend -- \
//!     <masterdata目录> <music_metas.json> <user.json> [params.json]
//! ```
//!
//! # 数据从哪里来
//!
//! 本仓库不携带任何游戏数据，三份输入都需要自备：
//!
//! 1. **masterdata 目录** — 游戏主数据 JSON（`cards.json`、`skills.json`、
//!    `events.json`、`cardRarities.json`、`gameCharacterUnits.json` 等，平铺在
//!    同一目录）。公开镜像：GitHub 上的 sekai-master-db 系列仓库，按区服选择，
//!    例如 `Sekai-World/sekai-master-db-diff`（日服）、
//!    `sekai-master-db-en-diff` / `-tc-diff` / `-kr-diff` 等区服变体；
//!    clone 后将本参数指向仓库根目录即可。
//! 2. **music_metas.json** — 歌曲元数据（分难度的 base_score / 技能系数 /
//!    event_rate）。sekai.best 公开维护一份：
//!    `https://storage.sekai.best/sekai-best-assets/music_metas.json`。
//!    本仓库 `tests/music_metas.json` 也有一份用于测试的副本，可直接使用。
//! 3. **user.json** — camelCase 用户数据（suite 上传链路格式）。最少需要
//!    `userCards`（cardId / level / skillLevel / masterRank /
//!    specialTrainingStatus / episodes）与 `userCharacters`；`userAreas`、
//!    `userMysekaiCanvases` 等字段可选，缺省按无加成处理。
//!
//! 可选的第 4 个参数是 params JSON（camelCase 或 snake_case 均可），字段见
//! `docs/parameters.md`；缺省使用下方 `DEFAULT_PARAMS`（多人 Live、活动分数
//! 目标需另传 `eventId`）。
//!
//! `recommend_json` 的第一个参数是扁平化后的 `OwnedGameData` JSON，不是
//! 原始 masterdata。本示例先用 `OwnedGameData::load` 从目录组装再序列化；
//! 服务端场景可以把这份序列化结果缓存起来，避免每次请求重复解析。

use std::path::Path;

use allium_deck::engine::{recommend_json, OwnedGameData};

const DEFAULT_PARAMS: &str = r#"{"liveType":"multi","target":"score","limit":5}"#;

fn main() {
    if let Err(error) = run() {
        eprintln!("错误: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [masterdata_dir, music_metas_path, user_path, rest @ ..] = args.as_slice() else {
        return Err(
            "用法: recommend <masterdata目录> <music_metas.json> <user.json> [params.json]"
                .to_string(),
        );
    };

    // 1. 从磁盘组装 OwnedGameData（原始 masterdata → 扁平化表），再序列化成
    //    recommend_json 期望的 masterdata JSON。
    let owned = OwnedGameData::load(Path::new(masterdata_dir), Path::new(music_metas_path))?;
    let masterdata_json = serde_json::to_string(&owned)
        .map_err(|error| format!("序列化 masterdata 失败: {error}"))?;

    // 2. 用户数据与参数。music metas 已并入 OwnedGameData，这里传空串即可。
    let user_json = read(user_path)?;
    let params_json = match rest {
        [params_path, ..] => read(params_path)?,
        [] => DEFAULT_PARAMS.to_string(),
    };

    // 3. 组卡：返回 top-K 卡组。注意当前 `recommend_json` 返回的 `cards`
    //    是候选池内的稠密索引（`DeckResult` 的 `CardIdx::raw()`），不是
    //    masterdata 卡 ID；需要卡 ID / 面板明细时用 `recommend` 结构体入口
    //    配合 `CardPool::game_id` / `summarize_deck`（参考 `src/bin/recommend_cli.rs`）。
    let response = recommend_json(&masterdata_json, "", &user_json, &params_json)
        .map_err(|error| format!("组卡失败: {error}"))?;

    // 美化输出。
    let pretty = serde_json::from_str::<serde_json::Value>(&response)
        .and_then(|value| serde_json::to_string_pretty(&value))
        .unwrap_or(response);
    println!("{pretty}");
    Ok(())
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("读取 {path} 失败: {error}"))
}
