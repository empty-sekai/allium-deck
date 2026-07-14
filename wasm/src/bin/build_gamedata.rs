//! 构建期工具：把 cn masterdata 扁平化后编成 postcard 二进制，供 wasm 内嵌。
//!
//! 流程：读 masterdata 目录 + music_metas → `OwnedGameData::from_sources`
//! → `postcard::to_allocvec` → 写 `src/embedded/gamedata_<region>.postcard`。
//!
//! 本地：
//!   build_gamedata --masterdata <dir> --music-metas <music_metas.json> [--region cn] [--out <path>]
//! CI：拉最新 cn masterdata 后调用，产物随 wasm 一起 `include_bytes!`。
//!
//! WL 支援表无需在目录里——引擎已 `include_str!` 内嵌兜底。

use std::path::PathBuf;

use allium_deck::engine::{MasterdataSources, OwnedGameData};

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

fn main() {
    if let Err(e) = run() {
        eprintln!("错误: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let masterdata = arg("--masterdata").ok_or("缺少 --masterdata")?;
    let music_metas = arg("--music-metas").ok_or("缺少 --music-metas")?;
    let region = arg("--region").unwrap_or_else(|| "cn".to_string());
    let out = arg("--out").unwrap_or_else(|| {
        format!(
            "{}/src/embedded/gamedata_{region}.postcard",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    // 复用与 load 同一套扁平化逻辑（from_dir → from_sources），保证产物与线上一致。
    let sources =
        MasterdataSources::from_dir(&PathBuf::from(&masterdata), &PathBuf::from(&music_metas))?;
    let owned = OwnedGameData::from_sources(&sources)?;
    if owned.music_metas.is_empty() {
        return Err(format!(
            "music_metas 为空，请检查 --music-metas 指向的文件: {music_metas}"
        ));
    }

    let bytes = postcard::to_allocvec(&owned).map_err(|e| format!("postcard 序列化失败: {e}"))?;

    let out_path = PathBuf::from(&out);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    std::fs::write(&out_path, &bytes)
        .map_err(|e| format!("写入 {} 失败: {e}", out_path.display()))?;

    eprintln!(
        "[build_gamedata] region={region} → {} ({:.2} MB, {} cards, {} music rows)",
        out_path.display(),
        bytes.len() as f64 / 1e6,
        owned.cards.len(),
        owned.music_metas.len(),
    );
    Ok(())
}
