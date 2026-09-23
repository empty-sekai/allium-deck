//! Pool-building layer: masterdata and player data in, search inputs out.
//!
//! [`build_card_pool`] resolves each owned card's power, skill and event bonus,
//! applies only hard/exact-safe candidate filters and produces the
//! [`crate::pool::CardPool`] and [`crate::search::SearchContext`] the search
//! layer consumes. The `_prepared` and `_fully_prepared` variants reuse
//! masterdata indexes and per-user preparation across repeated builds, and the
//! `_with_details` variants additionally return full-precision display data
//! aligned with the pool's dense card indexes.

mod build;
mod capacity;
mod card_config;
mod event_bonus;
mod filter;
mod gather;
mod index;
mod music;
mod power;
mod skill;
#[cfg(test)]
mod tests;
/// Handler 层的输入类型：masterdata 视图、用户数据与建池参数。
pub mod types;
mod validate;
pub(crate) mod world_bloom;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::search::SearchContext;

pub use build::{PreparedPoolBuild, cultivated_user_cards};
pub use gather::FullPrecisionCard;
pub use types::*;
pub use world_bloom::{WorldBloomSupportCard, world_bloom_support_cards};

pub(crate) use validate::validate_build_params;

use build::build_card_pool_fully_prepared_internal;

/// handler 构建阶段的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// 过滤后无候选卡。
    EmptyPool,
    /// 候选卡超过稠密 `CardIdx` 可表示的数量。
    TooManyCards(usize),
    /// A value or an interned table cannot be represented by the compact pool.
    CapacityExceeded {
        /// Compact field or side table which cannot represent the input.
        field: &'static str,
        /// Required value or number of distinct entries.
        value: u64,
        /// Largest exactly representable value or entry count.
        max: u64,
    },
    /// 参数非法。
    InvalidConfig(String),
}

impl Display for BuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPool => f.write_str("候选卡池为空"),
            Self::TooManyCards(count) => write!(f, "候选卡数量超过稠密索引容量: {count}"),
            Self::CapacityExceeded { field, value, max } => write!(
                f,
                "{field} exceeds exact representation capacity: {value} > {max}"
            ),
            Self::InvalidConfig(reason) => write!(f, "构建参数非法: {reason}"),
        }
    }
}

impl Error for BuildError {}

/// Reusable masterdata indexes for repeated pool builds.
///
/// Construct this once for an immutable `GameData` snapshot, then reuse it
/// across accounts and parameter sets to avoid rebuilding masterdata indexes.
#[derive(Clone)]
pub struct PreparedGameIndexes {
    indexes: Arc<index::PoolIndexes>,
}

impl PreparedGameIndexes {
    /// 为一份 masterdata 建立按 id 的查表索引。
    ///
    /// 索引与 masterdata 同生命周期，可在多次建池间复用。
    pub fn new(game: &types::GameData<'_>) -> Self {
        Self {
            indexes: Arc::new(index::PoolIndexes::build(game)),
        }
    }
}

/// 一份 masterdata 视图与其查表索引的绑定。
pub struct PreparedGameData<'a> {
    game: types::GameData<'a>,
    indexes: Arc<index::PoolIndexes>,
}

impl<'a> PreparedGameData<'a> {
    /// 就地建立索引并绑定。索引只服务这一次，重复建池请改用
    /// [`PreparedGameData::with_indexes`] 复用同一份索引。
    pub fn new(game: types::GameData<'a>) -> Self {
        let indexes = PreparedGameIndexes::new(&game);
        Self::with_indexes(game, &indexes)
    }

    /// 绑定到一份已建好的索引，索引本身按引用计数共享。
    pub fn with_indexes(game: types::GameData<'a>, indexes: &PreparedGameIndexes) -> Self {
        Self {
            game,
            indexes: Arc::clone(&indexes.indexes),
        }
    }

    /// 返回绑定的 masterdata 视图。
    #[inline]
    pub fn game(&self) -> &types::GameData<'a> {
        &self.game
    }
}
/// 将 masterdata + userdata 构建为搜索使用的 `CardPool` 与 `SearchContext`。
pub fn build_card_pool(
    user: &types::UserProfile,
    game: &types::GameData<'_>,
    params: &types::BuildParams,
) -> Result<(crate::pool::CardPool, SearchContext), BuildError> {
    let prepared = PreparedGameData::new(*game);
    build_card_pool_prepared(user, &prepared, params)
}

/// Build a search pool while reusing immutable masterdata indexes.
pub fn build_card_pool_prepared(
    user: &types::UserProfile,
    prepared: &PreparedGameData<'_>,
    params: &types::BuildParams,
) -> Result<(crate::pool::CardPool, SearchContext), BuildError> {
    let build = PreparedPoolBuild::new(user, prepared, params)?;
    build_card_pool_fully_prepared(prepared, &build)
}

/// 构建搜索池并保留与 dense card index 一一对应的全精度展示信息。
pub fn build_card_pool_with_details(
    user: &types::UserProfile,
    game: &types::GameData<'_>,
    params: &types::BuildParams,
) -> Result<(crate::pool::CardPool, SearchContext, Vec<FullPrecisionCard>), BuildError> {
    let prepared = PreparedGameData::new(*game);
    build_card_pool_with_details_prepared(user, &prepared, params)
}

/// Build a search pool with display details while reusing masterdata indexes.
pub fn build_card_pool_with_details_prepared(
    user: &types::UserProfile,
    prepared: &PreparedGameData<'_>,
    params: &types::BuildParams,
) -> Result<(crate::pool::CardPool, SearchContext, Vec<FullPrecisionCard>), BuildError> {
    let build = PreparedPoolBuild::new(user, prepared, params)?;
    build_card_pool_with_details_fully_prepared(prepared, &build)
}

/// Build a search pool from reusable user, parameter, and masterdata preparation.
pub fn build_card_pool_fully_prepared(
    prepared: &PreparedGameData<'_>,
    build: &PreparedPoolBuild<'_>,
) -> Result<(crate::pool::CardPool, SearchContext), BuildError> {
    let (pool, context, _) = build_card_pool_fully_prepared_internal(prepared, build, false)?;
    Ok((pool, context))
}

/// Build a pool with display details from reusable preparation.
pub fn build_card_pool_with_details_fully_prepared(
    prepared: &PreparedGameData<'_>,
    build: &PreparedPoolBuild<'_>,
) -> Result<(crate::pool::CardPool, SearchContext, Vec<FullPrecisionCard>), BuildError> {
    build_card_pool_fully_prepared_internal(prepared, build, true)
}
