//! HTTP service for the allium-deck recommendation engine.
//!
//! The engine is a pure computation library; this crate adds the parts a network
//! service needs around it: masterdata resident in memory, a bounded pool of search
//! threads, per-request ceilings, and observability.
//!
//! The binary in `main.rs` wires these together; they are exposed here so the
//! integration tests can drive the router in process.

#![deny(clippy::unwrap_used)]

pub mod api;
pub mod config;
pub mod metrics;
pub mod pool;
pub mod state;
