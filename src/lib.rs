#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod engine;
pub mod handler;
pub mod pool;
pub mod search;
pub mod types;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod embedded;
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub mod wasm;

pub use types::*;
