//! 内嵌 masterdata（仅 wasm feature）。
//!
//! 构建期由 `build_gamedata` 把扁平 `OwnedGameData` 编成 postcard 写入
//! `src/embedded/gamedata_cn.postcard`，此处 `include_bytes!` 编进 wasm。
//! 运行时 `postcard::from_bytes` 还原，无文件系统、无 JSON 解析。

use std::sync::OnceLock;

use allium_deck::engine::{EngineError, OwnedGameData};

/// cn 内嵌 masterdata（postcard）。
const GAMEDATA_CN: &[u8] = include_bytes!("embedded/gamedata_cn.postcard");
static GAMEDATA: OnceLock<Result<OwnedGameData, String>> = OnceLock::new();

/// 还原内嵌的 cn `OwnedGameData`。
pub fn embedded_gamedata() -> Result<&'static OwnedGameData, EngineError> {
    match GAMEDATA.get_or_init(|| {
        postcard::from_bytes(GAMEDATA_CN).map_err(|err| format!("内嵌 masterdata 解码失败: {err}"))
    }) {
        Ok(data) => Ok(data),
        Err(message) => Err(EngineError::Build(message.clone())),
    }
}
