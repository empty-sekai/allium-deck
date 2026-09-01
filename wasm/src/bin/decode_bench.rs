//! 一次性基准：原始 masterdata JSON 解析+扁平化 vs postcard 解码的耗时对比。
//! 用于「数据外置供给」方案选型。可保留作回归参考。

use std::path::PathBuf;
use std::time::Instant;

use allium_deck::engine::{MasterdataSources, OwnedGameData};

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: decode_bench <masterdata_dir> <packed.postcard>");
    let packed = std::env::args()
        .nth(2)
        .expect("missing packed postcard path");
    let dir = PathBuf::from(&root);

    // 1) 原始路径：读 24 张 JSON + serde 解析 + from_sources 扁平化
    let t_read = Instant::now();
    let sources = MasterdataSources::from_dir(&dir, &PathBuf::from("tests/music_metas.json"))
        .map_err(|e| eprintln!("music metas load failed: {e}"))
        .ok();
    let read_ms = t_read.elapsed().as_millis() as f64;
    if let Some(sources) = sources {
        let t_parse = Instant::now();
        let owned = OwnedGameData::from_sources(&sources).expect("from_sources failed");
        let parse_ms = t_parse.elapsed().as_millis() as f64;

        // 2) postcard 解码路径
        let bytes = std::fs::read(&packed).expect("read postcard");
        let t_decode = Instant::now();
        let restored: OwnedGameData = postcard::from_bytes(&bytes).expect("postcard decode failed");
        let decode_ms = t_decode.elapsed().as_millis() as f64;

        println!(
            "cards={} events={} honors={}",
            owned.cards.len(),
            owned.events.len(),
            owned.honors.len()
        );
        println!(
            "restored cards={} (一致性校验: {})",
            restored.cards.len(),
            restored.cards.len() == owned.cards.len()
        );
        println!("raw json 24 tables: read+serde_json+flatten = {read_ms:.0} + {parse_ms:.0} ms");
        println!("postcard {} bytes: decode = {decode_ms:.0} ms", bytes.len());
    }
}
