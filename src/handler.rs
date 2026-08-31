mod build;
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
mod validate;
pub(crate) mod world_bloom;
pub mod types;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::search::SearchContext;

pub use build::{cultivated_user_cards, PreparedPoolBuild};
pub use gather::FullPrecisionCard;
pub use types::*;
pub use world_bloom::{world_bloom_support_cards, WorldBloomSupportCard};

pub(crate) use validate::validate_build_params;

use build::build_card_pool_fully_prepared_internal;

/// handler 构建阶段的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// 过滤后无候选卡。
    EmptyPool,
    /// 候选卡超过 512-bit mask 容量。
    TooManyCards(usize),
    /// 参数非法。
    InvalidConfig(String),
}

impl Display for BuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPool => f.write_str("候选卡池为空"),
            Self::TooManyCards(count) => write!(f, "候选卡数量超过 mask 容量: {count}"),
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
    pub fn new(game: &types::GameData<'_>) -> Self {
        Self {
            indexes: Arc::new(index::PoolIndexes::build(game)),
        }
    }
}

pub struct PreparedGameData<'a> {
    game: types::GameData<'a>,
    indexes: Arc<index::PoolIndexes>,
}

impl<'a> PreparedGameData<'a> {
    pub fn new(game: types::GameData<'a>) -> Self {
        let indexes = PreparedGameIndexes::new(&game);
        Self::with_indexes(game, &indexes)
    }

    pub fn with_indexes(game: types::GameData<'a>, indexes: &PreparedGameIndexes) -> Self {
        Self {
            game,
            indexes: Arc::clone(&indexes.indexes),
        }
    }

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

