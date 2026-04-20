#![cfg_attr(not(test), deny(clippy::unwrap_used))]

// TASK-010 删除旧 CardSpec/CardPool 后，遗留 eval 层与新池模型不兼容；
// 该模块在 TASK-011 之前不参与编译。
#[cfg(any())]
pub mod eval;
pub mod handler;
pub mod pool;
// TASK-011: 搜索层已完成落地，保持为对外公开入口。
pub mod search;
pub mod types;

pub use types::*;
