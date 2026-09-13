//! Project Sekai deck recommendation engine.
//!
//! Given a player's card collection, event bonuses and an objective, this crate
//! searches for the best five-card deck using exact DFS with branch and bound.
//!
//! # Entry points
//!
//! [`engine::recommend_json`] takes JSON and returns JSON. [`engine::recommend`]
//! is the typed equivalent and skips serialization. Both run the same two stages:
//! [`handler::build_card_pool`] turns masterdata and player data into a
//! [`pool::CardPool`] plus a [`search::SearchContext`], then [`search::search`]
//! explores that pool.
//!
//! ```no_run
//! use allium_deck::engine::recommend_json;
//!
//! let decks = recommend_json(
//!     &std::fs::read_to_string("masterdata.json")?,
//!     &std::fs::read_to_string("music_metas.json")?,
//!     &std::fs::read_to_string("user.json")?,
//!     r#"{"target": "score", "liveType": "multi"}"#,
//! )?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! `target` accepts `score`, `power`, `skill`, `bonus` and `mysekai`. The full
//! parameter contract and the per-mode exactness matrix (which modes are exact
//! against brute force and which are heuristic) are in `docs/parameters.md`.
//!
//! The response is `{"decks": [{"cards": [id; 5], "score": u64}]}`, where
//! `cards` holds game card ids in deck order, leader first. Panel details such
//! as total power and live score are not part of it: build the pool with
//! [`handler::build_card_pool`] and summarise each result with
//! [`search::summarize_deck`] when you need them.
//!
//! # Modules
//!
//! | Module | Responsibility |
//! | --- | --- |
//! | [`engine`] | Entry points, masterdata loading, JSON parameter parsing |
//! | [`types`] | Shared identifiers and enums, re-exported at the crate root |
//! | [`handler`] | Pool building: candidate pruning, power / skill / event bonus precomputation, search context construction |
//! | [`pool`] | Structure-of-arrays card pool: columnar storage, bitmaps, read-only once frozen |
//! | [`search`] | Dominance pruning, suffix upper bounds, warm start, branch and bound dispatched by objective |
//! | [`auxiliary`] | Supporting calculations shared by the pool and search layers |
//!
//! # Feature flags
//!
//! On x86-64 the search detects AVX-512F/BW at runtime; unsupported CPUs and
//! other architectures fall back to scalar code automatically.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![warn(missing_docs)]

pub mod auxiliary;
pub mod engine;
pub mod handler;
pub mod pool;
pub mod search;
pub(crate) mod simd;
pub mod types;

pub use types::*;
