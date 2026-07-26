mod card_config;
mod event_bonus;
mod gather;
mod index;
mod music;
mod power;
mod skill;
mod support_bonus;
pub mod types;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::pool::EventBonusExact;
use crate::search::{SearchContext, SupportDeck};
use crate::types::{DefaultImage, FINAL_CHAPTER_EVENT_ID};

use card_config::apply_card_config;
use event_bonus::{build_card_event_bonus, build_event_context, EventContext};
pub use gather::FullPrecisionCard;
use gather::{sort_and_gather, CardIntermediate, GatheredContext};
use music::build_music_params;
use power::{
    build_power_batch_from_fn, resolve_unit_mask, PowerInput, PowerResult, PreparedPowerContext,
};
use skill::{build_skill, is_bfes_skill_pair, SkillResult, SkillState};
use support_bonus::calc_wb_support_bonus;
use types::{
    attr_to_pool_index, default_image_kind, is_after_training, parse_attr_code, parse_unit_code,
};

pub use types::*;

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

pub(crate) fn validate_build_params(params: &types::BuildParams) -> Result<(), BuildError> {
    let configs = [
        &params.card_configs.rarity_1_config,
        &params.card_configs.rarity_2_config,
        &params.card_configs.rarity_3_config,
        &params.card_configs.rarity_4_config,
        &params.card_configs.rarity_birthday_config,
    ]
    .into_iter()
    .chain(
        params
            .card_configs
            .single_card_configs
            .iter()
            .map(|entry| &entry.config),
    )
    .chain(params.single_card_configs.iter().map(|entry| &entry.config));
    for config in configs {
        if config.level.is_some_and(|value| value <= 0) {
            return Err(BuildError::InvalidConfig(
                "level must be positive".to_string(),
            ));
        }
        if config.skill_level.is_some_and(|value| value <= 0) {
            return Err(BuildError::InvalidConfig(
                "skillLevel must be positive".to_string(),
            ));
        }
        if config
            .master_rank
            .is_some_and(|value| !(0..=5).contains(&value))
        {
            return Err(BuildError::InvalidConfig(
                "masterRank must be in 0..=5".to_string(),
            ));
        }
        if config
            .episode_read_count
            .is_some_and(|value| !(0..=2).contains(&value))
        {
            return Err(BuildError::InvalidConfig(
                "episodeReadCount must be in 0..=2".to_string(),
            ));
        }
    }
    if params
        .forced_leader_character_id
        .is_some_and(|id| !(1..=26).contains(&id))
    {
        return Err(BuildError::InvalidConfig(
            "forcedLeaderCharacterId must be in 1..=26".to_string(),
        ));
    }
    if !(1..=types::MAX_BUILD_LIMIT).contains(&params.limit) {
        return Err(BuildError::InvalidConfig(format!(
            "limit 必须在 1..={} 范围内",
            types::MAX_BUILD_LIMIT
        )));
    }
    if !(1..=types::MAX_BUILD_TIMEOUT_MS).contains(&params.timeout_ms) {
        return Err(BuildError::InvalidConfig(format!(
            "timeout_ms 必须在 1..={} 范围内",
            types::MAX_BUILD_TIMEOUT_MS
        )));
    }
    if params
        .member
        .is_some_and(|member| member != crate::types::DECK_SIZE)
    {
        return Err(BuildError::InvalidConfig(format!(
            "member 仅支持 {}",
            crate::types::DECK_SIZE
        )));
    }
    if params.target_bonus_list.len() > types::MAX_TARGET_BONUS_BUCKETS {
        return Err(BuildError::InvalidConfig(format!(
            "target_bonus_list 最多支持 {} 个档位",
            types::MAX_TARGET_BONUS_BUCKETS
        )));
    }
    let mut bonus_targets = BTreeSet::new();
    for &bonus in &params.target_bonus_list {
        if !(0..=types::MAX_TARGET_BONUS).contains(&bonus) {
            return Err(BuildError::InvalidConfig(format!(
                "target bonus 必须在 0..={} 范围内",
                types::MAX_TARGET_BONUS
            )));
        }
        if !bonus_targets.insert(bonus) {
            return Err(BuildError::InvalidConfig(
                "target_bonus_list 不得包含重复档位".to_string(),
            ));
        }
    }
    if params.custom_bonus_character_ids.len() > 26 {
        return Err(BuildError::InvalidConfig(
            "custom bonus character 最多支持 26 项".to_string(),
        ));
    }
    let mut custom_characters = BTreeSet::new();
    if params
        .custom_bonus_character_ids
        .iter()
        .any(|id| !(1..=26).contains(id) || !custom_characters.insert(*id))
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus character id 非法或重复".to_string(),
        ));
    }
    if params
        .custom_bonus_attr
        .as_deref()
        .is_some_and(|attr| parse_attr_code(attr).is_none())
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus attr 非法".to_string(),
        ));
    }
    if params.custom_bonus_character_support_units.len() > 26 {
        return Err(BuildError::InvalidConfig(
            "custom bonus support unit 最多支持 26 项".to_string(),
        ));
    }
    let mut support_characters = BTreeSet::new();
    if params
        .custom_bonus_character_support_units
        .iter()
        .any(|entry| {
            !(1..=26).contains(&entry.character_id)
                || !support_characters.insert(entry.character_id)
                || !matches!(
                    entry.unit,
                    crate::types::Unit::LightSound
                        | crate::types::Unit::Idol
                        | crate::types::Unit::Street
                        | crate::types::Unit::Themepark
                        | crate::types::Unit::SchoolRefusal
                        | crate::types::Unit::Piapro
                )
        })
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus support unit 非法或重复".to_string(),
        ));
    }
    if params
        .custom_bonus_character_support_units
        .iter()
        .any(|entry| !custom_characters.contains(&entry.character_id))
    {
        return Err(BuildError::InvalidConfig(
            "custom bonus support unit character 必须包含在 custom bonus character 中".to_string(),
        ));
    }
    if params.multi_live_score_up_lower_bound.is_some()
        && !matches!(params.live_type, crate::types::LiveType::Multi)
    {
        return Err(BuildError::InvalidConfig(
            "multi_live_score_up_lower_bound 仅支持 multi live".to_string(),
        ));
    }
    if !params.target_bonus_list.is_empty()
        && !matches!(params.target, crate::types::ScoreTarget::Bonus)
    {
        return Err(BuildError::InvalidConfig(
            "target_bonus_list 仅支持 bonus target".to_string(),
        ));
    }
    if matches!(
        params.live_skill_order,
        crate::types::LiveSkillOrder::Specific
    ) && params.specific_skill_order.is_none()
    {
        return Err(BuildError::InvalidConfig(
            "specific_skill_order 是 specific 策略的必填项".to_string(),
        ));
    }
    Ok(())
}

fn enrich_master(master: &types::MasterCard, game: &types::GameData<'_>) -> types::MasterCard {
    let mut master = master.clone();
    if master.max_level.is_none() || master.max_skill_level.is_none() {
        if let Some(rarity) = game
            .card_rarities
            .iter()
            .find(|entry| entry.card_rarity_type == master.card_rarity_type)
        {
            master.max_level.get_or_insert(rarity.max_level);
            master.max_skill_level.get_or_insert(rarity.max_skill_level);
        }
    }
    if master.max_master_rank.is_none() {
        let max_master_rank = game
            .master_lessons
            .iter()
            .filter(|entry| entry.card_rarity_type == master.card_rarity_type)
            .map(|entry| entry.master_rank)
            .max()
            .unwrap_or(0);
        master.max_master_rank = Some(max_master_rank);
    }
    master
}

fn merged_configs(params: &types::BuildParams) -> types::CardConfigSet {
    let mut configs = params.card_configs.clone();
    configs
        .single_card_configs
        .extend(params.single_card_configs.iter().cloned());
    configs
}

fn normalize_user_cards(
    user: &types::UserProfile,
    params: &types::BuildParams,
    game: &types::GameData<'_>,
) -> Vec<types::UserCard> {
    let mut cards = user.user_cards.clone();
    for &card_id in &params.fixed_cards {
        if cards.iter().any(|card| card.card_id == card_id) {
            continue;
        }
        // 虚拟固定卡代表「假设我满配持有这张卡」。能否特训取决于 master 是否有花后技能；
        // 旧逻辑写死 none/original，导致可特训的固定卡渲染成花前、且 build_power 漏掉
        // special_training 固定 power 加成。这里按 master.special_training_skill_id 判定。
        let can_train = game
            .cards
            .iter()
            .find(|card| card.id == card_id)
            .is_some_and(card_can_special_train);
        let (special_training_status, default_image) = if can_train {
            ("done".to_string(), "special_training".to_string())
        } else {
            ("none".to_string(), "original".to_string())
        };
        cards.push(types::UserCard {
            card_id,
            level: 1,
            skill_level: 1,
            master_rank: 0,
            special_training_status,
            default_image,
            episodes_read: Vec::new(),
            is_virtual: true,
            has_canvas_bonus_override: None,
        });
    }
    cards
}

const EP_PREFILTER_MIN_POOL: usize = 50;
const PER_CHAR_KEEP: usize = 6;
const FINAL_CHAPTER_PER_CHAR_KEEP: usize = 16;
const GENERAL_TRIM_THRESHOLD: usize = 400;
const GENERAL_PER_CHAR_KEEP: usize = 10;
const CHALLENGE_ALL_PER_CHAR_KEEP: usize = 19;

struct PreparedCardSeed<'a> {
    master: &'a types::MasterCard,
    user_card: Cow<'a, types::UserCard>,
    unit_mask: u8,
    attr: u8,
    default_image_kind: DefaultImage,
    default_image: DefaultImage,
    after_training: bool,
    event_bonus: EventBonusExact,
    has_char_bonus: bool,
    has_attr_bonus: bool,
    leader_honor_bonus: u16,
    leader_limit_bonus: u16,
}

struct PreparedCardBuild<'a> {
    master: &'a types::MasterCard,
    user_card: Cow<'a, types::UserCard>,
    unit_mask: u8,
    attr: u8,
    default_image: DefaultImage,
    after_training: bool,
    event_bonus: EventBonusExact,
    has_char_bonus: bool,
    has_attr_bonus: bool,
    leader_honor_bonus: u16,
    leader_limit_bonus: u16,
    skill_options: [Option<(SkillState, SkillResult)>; 2],
    skill_state_controls_image: bool,
}

/// Reusable user and parameter preparation for repeated pool builds.
pub struct PreparedPoolBuild<'a> {
    params: types::BuildParams,
    event_ctx: Option<EventContext>,
    music: Option<music::MusicParams>,
    powers: Vec<PowerResult>,
    cards: Vec<PreparedCardBuild<'a>>,
    ep_prefilter_applied: bool,
    honor_bonus: u32,
}

impl<'a> PreparedPoolBuild<'a> {
    pub fn new(
        user: &'a types::UserProfile,
        prepared: &'a PreparedGameData<'_>,
        params: &types::BuildParams,
    ) -> Result<Self, BuildError> {
        let game = prepared.game();
        let indexes = prepared.indexes.as_ref();
        validate_build_params(params)?;
        if params.multi_live_score_up_lower_bound.is_some()
            && !matches!(params.live_type, crate::types::LiveType::Multi)
        {
            return Err(BuildError::InvalidConfig(
                "multi_live_score_up_lower_bound 仅支持 multi live".to_string(),
            ));
        }
        let event_ctx = build_event_context(game, params);
        if !params.target_bonus_list.is_empty()
            && !matches!(params.target, crate::types::ScoreTarget::Bonus)
        {
            return Err(BuildError::InvalidConfig(
                "target_bonus_list 仅支持 bonus target".to_string(),
            ));
        }
        if matches!(params.target, crate::types::ScoreTarget::Bonus) {
            if event_ctx.is_none() {
                return Err(BuildError::InvalidConfig(
                    "bonus target 需要活动上下文".to_string(),
                ));
            }
            if params.event_id == Some(crate::types::FINAL_CHAPTER_EVENT_ID) {
                return Err(BuildError::InvalidConfig(
                    "终章不支持 bonus target".to_string(),
                ));
            }
        }

        let fixture_bonus_limit = resolve_fixture_bonus_limit(game, event_ctx.as_ref());
        let music = build_music_params(game, params);
        let configs = merged_configs(params);
        let normalized_cards =
            (!params.fixed_cards.is_empty()).then(|| normalize_user_cards(user, params, game));
        let configs_are_noop = configs == types::CardConfigSet::default();
        let power_ctx = PreparedPowerContext::new(user, game, indexes, fixture_bonus_limit);
        let card_count = normalized_cards
            .as_ref()
            .map_or(user.user_cards.len(), Vec::len);
        let mut seeds = Vec::with_capacity(card_count);

        // (bonus_rate, leader_bonus_rate) of the first event-card row per card id.
        let limited_bonus_by_card: std::collections::HashMap<i32, (i32, i32)> = event_ctx
            .as_ref()
            .map(|ctx| {
                let mut map = std::collections::HashMap::with_capacity(ctx.event_cards.len());
                for entry in &ctx.event_cards {
                    map.entry(entry.card_id)
                        .or_insert((entry.bonus_rate, entry.leader_bonus_rate));
                }
                map
            })
            .unwrap_or_default();
        let leader_honor_by_char: [u16; 27] = {
            let mut result = [0u16; 27];
            if let Some(ctx) = event_ctx.as_ref() {
                if ctx.event_id == FINAL_CHAPTER_EVENT_ID && !ctx.honor_bonuses.is_empty() {
                    let owned_honors: std::collections::HashSet<i32> = user
                        .user_honors
                        .iter()
                        .map(|honor| honor.honor_id)
                        .collect();
                    for entry in &ctx.honor_bonuses {
                        if let Ok(ch) = usize::try_from(entry.leader_game_character_id) {
                            if ch < 27 && owned_honors.contains(&entry.honor_id) {
                                result[ch] =
                                    result[ch].wrapping_add(entry.bonus_rate.max(0) as u16);
                            }
                        }
                    }
                }
            }
            result
        };

        let mut prepare_card = |mut user_card: Cow<'a, types::UserCard>| {
            let Some(card_data) = indexes.card_data(user_card.card_id) else {
                return;
            };
            let master = &card_data.master;
            if !configs_are_noop
                && !apply_card_config(
                    user_card.to_mut(),
                    master,
                    &configs,
                    game.card_rarities,
                    game.card_episodes,
                )
            {
                return;
            }
            if card_data.unit_mask == 0 {
                return;
            }
            let Some(attr) = card_data.attr else {
                return;
            };
            let default_image_kind = default_image_kind(&user_card.default_image);
            let after_training = is_after_training(&user_card.special_training_status);
            let default_image =
                if matches!(default_image_kind, DefaultImage::SpecialTraining) || after_training {
                    DefaultImage::SpecialTraining
                } else {
                    DefaultImage::Original
                };
            let limited_entry = limited_bonus_by_card.get(&master.id).copied();
            let (event_bonus, has_char_bonus, has_attr_bonus) = event_ctx
                .as_ref()
                .map(|ctx| {
                    build_card_event_bonus(
                        user_card.as_ref(),
                        master,
                        attr,
                        card_data.primary_unit,
                        card_data.support_unit,
                        card_data.support_unit_unrestricted,
                        limited_entry
                            .map(|(bonus, _)| bonus.saturating_mul(10))
                            .unwrap_or(0),
                        ctx,
                    )
                })
                .unwrap_or((EventBonusExact::default(), false, false));
            let leader_honor_bonus = if event_ctx.is_some() {
                usize::try_from(master.character_id)
                    .ok()
                    .filter(|ch| *ch < 27)
                    .map(|ch| leader_honor_by_char[ch])
                    .unwrap_or(0)
            } else {
                0
            };
            let leader_limit_bonus = if event_ctx.is_some() {
                limited_entry
                    .map(|(_, leader)| leader.max(0) as u16)
                    .unwrap_or(0)
            } else {
                0
            };
            seeds.push(PreparedCardSeed {
                master,
                user_card,
                unit_mask: card_data.unit_mask,
                attr,
                default_image_kind,
                default_image,
                after_training,
                event_bonus,
                has_char_bonus,
                has_attr_bonus,
                leader_honor_bonus,
                leader_limit_bonus,
            });
        };
        if let Some(normalized_cards) = normalized_cards {
            for user_card in normalized_cards {
                prepare_card(Cow::Owned(user_card));
            }
        } else {
            for user_card in &user.user_cards {
                prepare_card(Cow::Borrowed(user_card));
            }
        }

        let is_final_chapter = event_ctx
            .as_ref()
            .is_some_and(|ctx| ctx.event_id == FINAL_CHAPTER_EVENT_ID);
        let can_prefilter_before_power = event_ctx.as_ref().is_some_and(|ctx| {
            ctx.support_deck_count == 0 && ctx.event_id != FINAL_CHAPTER_EVENT_ID
        }) && !matches!(
            params.target,
            crate::types::ScoreTarget::Power | crate::types::ScoreTarget::Skill
        );
        let mut ep_prefilter_applied = false;
        if can_prefilter_before_power {
            seeds.retain(|card| {
                prepared_keep_card(card, params)
                    && prepared_post_event_unit_filter(card, params, event_ctx.as_ref())
            });
        }
        if can_prefilter_before_power
            && seeds.len() > EP_PREFILTER_MIN_POOL
            && seeds
                .iter()
                .any(|card| prepared_ep_prefilter_keep(card, false, is_final_chapter))
        {
            seeds.retain(|card| {
                prepared_ep_prefilter_keep_with_params(card, params, false, is_final_chapter)
            });
            ep_prefilter_applied = true;
        }

        let mut cards = Vec::with_capacity(seeds.len());
        for seed in seeds {
            let character_rank = power_ctx.character_rank(seed.master.character_id);
            let (skill_states, skill_state_count) = skill_states_for_card(
                seed.default_image_kind,
                seed.after_training,
                seed.master,
                params,
            );
            let mut skill_options = [None, None];
            for (slot, &skill_state) in skill_options
                .iter_mut()
                .zip(skill_states.iter())
                .take(skill_state_count)
            {
                *slot = Some((
                    skill_state,
                    build_skill(
                        seed.user_card.as_ref(),
                        seed.master,
                        game,
                        indexes,
                        character_rank,
                        event_ctx.as_ref().and_then(|ctx| ctx.skill_score_up_limit),
                        skill_state,
                    ),
                ));
            }
            let skill_state_controls_image =
                collapse_non_bfes_skill_states(&mut skill_options, skill_state_count);
            cards.push(PreparedCardBuild {
                master: seed.master,
                user_card: seed.user_card,
                unit_mask: seed.unit_mask,
                attr: seed.attr,
                default_image: seed.default_image,
                after_training: seed.after_training,
                event_bonus: seed.event_bonus,
                has_char_bonus: seed.has_char_bonus,
                has_attr_bonus: seed.has_attr_bonus,
                leader_honor_bonus: seed.leader_honor_bonus,
                leader_limit_bonus: seed.leader_limit_bonus,
                skill_options,
                skill_state_controls_image,
            });
        }

        // 综合力只依赖冻结在 prepare 里的输入，一次算好缓存；build 阶段零重算。
        let mut powers = Vec::with_capacity(cards.len());
        build_power_batch_from_fn(
            cards.len(),
            |index| {
                let card = &cards[index];
                PowerInput {
                    user_card: card.user_card.as_ref(),
                    master: card.master,
                    unit_mask: card.unit_mask,
                    attr: card.attr,
                }
            },
            &power_ctx,
            indexes,
            &mut powers,
        );

        Ok(Self {
            params: params.clone(),
            event_ctx,
            music,
            powers,
            cards,
            ep_prefilter_applied,
            honor_bonus: compute_honor_bonus(user, indexes),
        })
    }
}

fn prepared_ep_prefilter_keep(
    card: &PreparedCardSeed<'_>,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    if is_world_bloom || is_final_chapter {
        return card.master.card_rarity_type >= 3 || card.event_bonus.total_x10() > 0;
    }
    if card.master.card_rarity_type >= 4 {
        return true;
    }
    card.event_bonus.total_x10() > 0 && card.has_char_bonus && card.has_attr_bonus
}

fn prepared_ep_prefilter_keep_with_params(
    card: &PreparedCardSeed<'_>,
    params: &types::BuildParams,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    if params.fixed_cards.contains(&card.master.id)
        || params.fixed_characters.contains(&card.master.character_id)
    {
        return true;
    }
    prepared_ep_prefilter_keep(card, is_world_bloom, is_final_chapter)
}

fn prepared_keep_card(card: &PreparedCardSeed<'_>, params: &types::BuildParams) -> bool {
    let is_fixed_card = params.fixed_cards.contains(&card.master.id);
    if params.excluded_cards.contains(&card.master.id) {
        return false;
    }
    if !is_fixed_card {
        if let Some(unit) = params
            .unit_filter
            .as_deref()
            .and_then(parse_unit_code)
            .and_then(types::unit_to_pool_index)
        {
            if card.unit_mask & (1u8 << unit) == 0 {
                return false;
            }
        }
        if let Some(attr) = params
            .attr_filter
            .as_deref()
            .and_then(parse_attr_code)
            .and_then(attr_to_pool_index)
        {
            if card.attr != attr {
                return false;
            }
        }
        if params.filter_other_unit {
            if let Some(unit) = params
                .event_unit
                .as_deref()
                .and_then(parse_unit_code)
                .and_then(types::unit_to_pool_index)
            {
                if card.unit_mask & (1u8 << unit) == 0 {
                    return false;
                }
            }
        }
    }
    params
        .challenge_live_character_id
        .is_none_or(|character_id| card.master.character_id == character_id)
}

fn prepared_post_event_unit_filter(
    card: &PreparedCardSeed<'_>,
    params: &types::BuildParams,
    event_ctx: Option<&EventContext>,
) -> bool {
    if !params.filter_other_unit {
        return true;
    }
    let Some(unit) = event_ctx.and_then(|ctx| ctx.filter_unit) else {
        return true;
    };
    let Some(unit_index) = types::unit_to_pool_index(unit) else {
        return true;
    };
    let wanted = 1u8 << unit_index;
    let piapro = types::unit_to_pool_index(crate::types::Unit::Piapro)
        .map(|index| 1u8 << index)
        .unwrap_or(0);
    card.unit_mask & wanted != 0 || card.unit_mask == piapro
}

fn ep_prefilter_keep(
    card: &CardIntermediate,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    // World Bloom / Final Chapter 跳过普通活动的双轴过滤，需要保留足够角色与属性覆盖。
    if is_world_bloom || is_final_chapter {
        return card.card_rarity_type >= 3 || card.event_bonus.total_x10() > 0;
    }
    // 4星+无条件保留
    if card.card_rarity_type >= 4 {
        return true;
    }
    // 3星：有 bonus + 双轴命中才保留
    if card.card_rarity_type == 3 {
        if card.event_bonus.total_x10() == 0 {
            return false;
        }
        return card.has_char_bonus && card.has_attr_bonus;
    }
    // 1-2星：有 bonus 且双轴同时命中才保留
    if card.event_bonus.total_x10() == 0 {
        return false;
    }
    card.has_char_bonus && card.has_attr_bonus
}

fn per_character_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    per_char_keep: usize,
) {
    if cards.len() <= EP_PREFILTER_MIN_POOL {
        return;
    }
    debug_assert!(per_char_keep <= FINAL_CHAPTER_PER_CHAR_KEEP);

    let compare = |a: &CardIntermediate, b: &CardIntermediate| {
        let a_bonus = a.event_bonus.total_x10();
        let b_bonus = b.event_bonus.total_x10();
        // 4星无bonus给虚拟bonus=1，排在有bonus卡之后但优于低星
        let a_effective_bonus = if a_bonus == 0 && a.card_rarity_type >= 4 {
            1
        } else {
            a_bonus
        };
        let b_effective_bonus = if b_bonus == 0 && b.card_rarity_type >= 4 {
            1
        } else {
            b_bonus
        };
        let a_key = a.power.power_max.max(0) as u64 * (256 + a.skill.skill_max as u64);
        let b_key = b.power.power_max.max(0) as u64 * (256 + b.skill.skill_max as u64);
        b_effective_bonus
            .cmp(&a_effective_bonus)
            .then_with(|| b.card_rarity_type.cmp(&a.card_rarity_type))
            .then_with(|| b_key.cmp(&a_key))
    };

    // Only the best K non-exempt cards per character are needed. Sorting the
    // entire vector moves large CardIntermediate values O(n log n) times.
    // Maintain 27 tiny, stable top-K index lists instead; equal keys stay in
    // their original order, matching stable-sort selection semantics.
    let mut selected = [[usize::MAX; FINAL_CHAPTER_PER_CHAR_KEEP]; 27];
    let mut counts = [0usize; 27];
    let mut keep = vec![false; cards.len()];
    for (index, card) in cards.iter().enumerate() {
        if params.fixed_cards.contains(&card.game_card_id)
            || params
                .fixed_characters
                .contains(&(card.character_id as i32))
            || card.event_bonus.total_x10() >= 300
        {
            keep[index] = true;
            continue;
        }
        let ch = (card.character_id as usize).min(26);
        let count = counts[ch];
        let mut insert = 0usize;
        while insert < count
            && compare(card, &cards[selected[ch][insert]]) != std::cmp::Ordering::Less
        {
            insert += 1;
        }
        if insert >= per_char_keep {
            continue;
        }
        let new_count = (count + 1).min(per_char_keep);
        if count == per_char_keep {
            keep[selected[ch][per_char_keep - 1]] = false;
        }
        let mut slot = new_count - 1;
        while slot > insert {
            selected[ch][slot] = selected[ch][slot - 1];
            slot -= 1;
        }
        selected[ch][insert] = index;
        counts[ch] = new_count;
        keep[index] = true;
    }
    let mut index = 0usize;
    cards.retain(|_| {
        let result = keep[index];
        index += 1;
        result
    });
}

fn ep_prefilter_keep_with_params(
    card: &CardIntermediate,
    params: &types::BuildParams,
    is_world_bloom: bool,
    is_final_chapter: bool,
) -> bool {
    if (!params.fixed_cards.is_empty() && params.fixed_cards.contains(&card.game_card_id))
        || (!params.fixed_characters.is_empty()
            && params
                .fixed_characters
                .contains(&(card.character_id as i32)))
    {
        return true;
    }
    ep_prefilter_keep(card, is_world_bloom, is_final_chapter)
}

fn general_per_character_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    per_char_keep: usize,
) {
    cards.sort_by(|a, b| {
        let a_key = a.power.power_max.max(0) as u64 * (256 + a.skill.skill_max as u64);
        let b_key = b.power.power_max.max(0) as u64 * (256 + b.skill.skill_max as u64);
        b_key.cmp(&a_key)
    });
    let mut counts = [0u8; 27];
    cards.retain(|card| {
        if params.fixed_cards.contains(&card.game_card_id) {
            return true;
        }
        let ch = (card.character_id as usize).min(26);
        if (counts[ch] as usize) < per_char_keep {
            counts[ch] += 1;
            true
        } else {
            false
        }
    });
}

fn target_per_character_trim(cards: &mut Vec<CardIntermediate>, params: &types::BuildParams) {
    // minimize（最弱组卡，仅 Power）时保留每角色最弱的若干张，否则保留最强。
    let minimize = params.minimize && matches!(params.target, crate::types::ScoreTarget::Power);
    cards.sort_by(|a, b| {
        let (a_key, b_key) = match params.target {
            crate::types::ScoreTarget::Power => (
                a.power.power_max.max(0) as u64,
                b.power.power_max.max(0) as u64,
            ),
            crate::types::ScoreTarget::Skill => {
                (a.skill.skill_max as u64, b.skill.skill_max as u64)
            }
            _ => {
                let a_key = a.power.power_max.max(0) as u64 * (256 + a.skill.skill_max as u64);
                let b_key = b.power.power_max.max(0) as u64 * (256 + b.skill.skill_max as u64);
                (a_key, b_key)
            }
        };
        // minimize 时按 power 升序（最弱优先保留），否则降序。次级键同向翻转。
        let primary = if minimize {
            a_key.cmp(&b_key)
        } else {
            b_key.cmp(&a_key)
        };
        let rarity = if minimize {
            a.card_rarity_type.cmp(&b.card_rarity_type)
        } else {
            b.card_rarity_type.cmp(&a.card_rarity_type)
        };
        primary
            .then(rarity)
            .then_with(|| a.game_card_id.cmp(&b.game_card_id))
    });

    let mut counts = [0u8; 27];
    cards.retain(|card| {
        if params.fixed_cards.contains(&card.game_card_id)
            || params
                .fixed_characters
                .contains(&(card.character_id as i32))
        {
            return true;
        }
        let ch = (card.character_id as usize).min(26);
        if (counts[ch] as usize) < GENERAL_PER_CHAR_KEEP {
            counts[ch] += 1;
            true
        } else {
            false
        }
    });
}

fn keep_card(card: &CardIntermediate, params: &types::BuildParams) -> bool {
    let is_fixed_card = params.fixed_cards.contains(&card.game_card_id);
    if params.excluded_cards.contains(&card.game_card_id) {
        return false;
    }

    if !is_fixed_card {
        if let Some(unit) = params
            .unit_filter
            .as_deref()
            .and_then(parse_unit_code)
            .and_then(types::unit_to_pool_index)
        {
            let wanted = 1u8 << unit;
            if card.unit_mask_raw & wanted == 0 {
                return false;
            }
        }

        if let Some(attr) = params
            .attr_filter
            .as_deref()
            .and_then(parse_attr_code)
            .and_then(attr_to_pool_index)
        {
            if card.attr != attr {
                return false;
            }
        }

        if params.filter_other_unit {
            if let Some(unit) = params
                .event_unit
                .as_deref()
                .and_then(parse_unit_code)
                .and_then(types::unit_to_pool_index)
            {
                let wanted = 1u8 << unit;
                if card.unit_mask_raw & wanted == 0 {
                    return false;
                }
            }
        }
    }

    if let Some(challenge_char_id) = params.challenge_live_character_id {
        if card.character_id != challenge_char_id as u8 {
            return false;
        }
    }

    true
}

fn normalize_boost_rate_pct(boost: Option<i32>) -> u32 {
    match boost {
        Some(value) if value <= 0 => 100,
        Some(value) if value <= 5 => (value * 500) as u32,
        Some(value) if value <= 10 => (2500 + (value - 5) * 200) as u32,
        _ => 100,
    }
}

fn validate_fixed_constraints(
    params: &types::BuildParams,
    full: &[CardIntermediate],
) -> Result<(Vec<u16>, Vec<u8>), BuildError> {
    if matches!(
        params.live_type,
        crate::types::LiveType::Challenge | crate::types::LiveType::ChallengeAuto
    ) && !params.fixed_characters.is_empty()
    {
        return Err(BuildError::InvalidConfig(
            "challenge live 不支持 fixed_characters".to_string(),
        ));
    }
    if params.fixed_cards.len() + params.fixed_characters.len() > crate::types::DECK_SIZE {
        return Err(BuildError::InvalidConfig("固定约束数量超过 5".to_string()));
    }

    let mut fixed_card_ids = Vec::with_capacity(params.fixed_cards.len());
    let mut seen_cards = BTreeSet::new();
    let mut seen_chars = BTreeSet::new();
    for &card_id in &params.fixed_cards {
        if !(1..=u16::MAX as i32).contains(&card_id) {
            return Err(BuildError::InvalidConfig(format!(
                "fixed card id 非法: {card_id}"
            )));
        }
        if !seen_cards.insert(card_id) {
            return Err(BuildError::InvalidConfig(format!(
                "fixed card 重复: {card_id}"
            )));
        }
        let Some(character_id) = full
            .iter()
            .find(|card| card.game_card_id == card_id)
            .map(|card| card.character_id)
        else {
            return Err(BuildError::EmptyPool);
        };
        if !seen_chars.insert(character_id) {
            return Err(BuildError::InvalidConfig(format!(
                "fixed card 角色重复: {character_id}"
            )));
        }
        fixed_card_ids.push(card_id as u16);
    }

    let mut fixed_character_ids = Vec::with_capacity(params.fixed_characters.len());
    for &character_id in &params.fixed_characters {
        if !(1..=26).contains(&character_id) {
            return Err(BuildError::InvalidConfig(format!(
                "fixed character id 非法: {character_id}"
            )));
        }
        let character_id = character_id as u8;
        if !seen_chars.insert(character_id) {
            return Err(BuildError::InvalidConfig(format!(
                "fixed character 角色重复: {character_id}"
            )));
        }
        if full.iter().all(|card| card.character_id != character_id) {
            return Err(BuildError::EmptyPool);
        }
        fixed_character_ids.push(character_id);
    }

    Ok((fixed_card_ids, fixed_character_ids))
}

fn skill_states_for_card(
    default_image_kind: DefaultImage,
    after_training: bool,
    master: &types::MasterCard,
    params: &types::BuildParams,
) -> ([SkillState; 2], usize) {
    if params.keep_after_training_state {
        if matches!(default_image_kind, DefaultImage::SpecialTraining) {
            ([SkillState::AfterTraining, SkillState::AfterTraining], 1)
        } else {
            ([SkillState::BeforeTraining, SkillState::BeforeTraining], 1)
        }
    } else if master.special_training_skill_id.is_some() {
        ([SkillState::AfterTraining, SkillState::BeforeTraining], 2)
    } else if matches!(default_image_kind, DefaultImage::SpecialTraining) || after_training {
        ([SkillState::AfterTraining, SkillState::AfterTraining], 1)
    } else {
        ([SkillState::BeforeTraining, SkillState::BeforeTraining], 1)
    }
}

fn card_can_special_train(master: &types::MasterCard) -> bool {
    master.special_training_skill_id.is_some()
        || master.special_training_power1_bonus_fixed > 0
        || master.special_training_power2_bonus_fixed > 0
        || master.special_training_power3_bonus_fixed > 0
        || matches!(master.card_rarity_type, 3 | 4)
}

fn collapse_non_bfes_skill_states(
    states: &mut [Option<(SkillState, SkillResult)>; 2],
    state_count: usize,
) -> bool {
    if state_count != 2 {
        return false;
    }
    let Some((_, first)) = states[0].as_ref() else {
        return false;
    };
    let Some((_, second)) = states[1].as_ref() else {
        return false;
    };
    let is_bfes = is_bfes_skill_pair(first, second);
    if !is_bfes {
        let keep_after = first.skill_max > second.skill_max;
        states[usize::from(keep_after)] = None;
    }
    is_bfes
}

pub struct WorldBloomSupportCard {
    pub card_id: i32,
    pub bonus: f64,
    pub skill_level: i32,
    pub master_rank: i32,
    pub level: i32,
    pub after_training: bool,
    pub default_image: DefaultImage,
}

/// Evaluate every owned card for a World Bloom support deck.
///
/// This operation deliberately does not build or mutate the DFS search pool.
pub fn world_bloom_support_cards(
    user: &types::UserProfile,
    game: &types::GameData<'_>,
    params: &types::BuildParams,
    support_master_max: bool,
    support_skill_max: bool,
    filter_other_unit: bool,
) -> Result<Vec<WorldBloomSupportCard>, BuildError> {
    let event_id = params.event_id.unwrap_or_default();
    let turn = params.world_bloom_event_turn.or_else(|| {
        params.event_id.and_then(|event_id| {
            if event_id == FINAL_CHAPTER_EVENT_ID {
                Some(2)
            } else if event_id > 1000 {
                Some((event_id / 100_000) % 10 + 1)
            } else if game
                .world_blooms
                .iter()
                .any(|entry| entry.event_id == event_id)
            {
                Some(if event_id <= 140 { 1 } else { 2 })
            } else {
                None
            }
        })
    });
    let Some(turn) = turn.filter(|turn| (1..=3).contains(turn)) else {
        return Err(BuildError::InvalidConfig(
            "world_bloom_event_turn or a World Bloom event_id is required".to_string(),
        ));
    };
    let special_character_id = params.world_bloom_character_id.or_else(|| {
        params.event_id.and_then(|event_id| {
            game.world_blooms
                .iter()
                .find(|entry| entry.event_id == event_id)
                .and_then(|entry| entry.game_character_id)
        })
    });
    let Some(special_character_id) = special_character_id.filter(|id| (1..=26).contains(id)) else {
        return Err(BuildError::InvalidConfig(
            "world_bloom_character_id is required".to_string(),
        ));
    };

    let mut result = Vec::with_capacity(user.user_cards.len());
    for original in &user.user_cards {
        let Some(master) = game.cards.iter().find(|card| card.id == original.card_id) else {
            return Err(BuildError::InvalidConfig(format!(
                "support deck card not found for card_id={}",
                original.card_id
            )));
        };
        let master = enrich_master(master, game);
        let mut card = original.clone();
        if support_master_max {
            card.master_rank = master.max_master_rank.unwrap_or(card.master_rank);
        }
        if support_skill_max {
            card.skill_level = master.max_skill_level.unwrap_or(card.skill_level);
        }
        let unit_mask_raw = resolve_unit_mask(&master, game);
        let bonus = calc_wb_support_bonus(
            game,
            event_id,
            Some(turn),
            Some(special_character_id),
            master.id.clamp(0, u16::MAX as i32) as u16,
            master.card_rarity_type,
            master.character_id.clamp(0, u8::MAX as i32) as u8,
            unit_mask_raw,
            !filter_other_unit,
            card.master_rank,
            card.skill_level,
        );
        result.push(WorldBloomSupportCard {
            card_id: card.card_id,
            bonus,
            skill_level: card.skill_level,
            master_rank: card.master_rank,
            level: card.level,
            after_training: is_after_training(&card.special_training_status),
            default_image: default_image_kind(&card.default_image),
        });
    }
    result.sort_by(|left, right| {
        right
            .bonus
            .total_cmp(&left.bonus)
            .then_with(|| left.card_id.cmp(&right.card_id))
    });
    Ok(result)
}

/// Slim per-card support-deck seed (deduped by card id).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SupportSeedSlim {
    card_id: u16,
    rarity: i32,
    character_id: u8,
    unit_mask: u8,
    master_rank: i32,
    skill_level: i32,
}

pub(crate) fn support_seed_from_intermediate(
    card: &CardIntermediate,
    indexes: &index::PoolIndexes,
    support_master_max: bool,
    support_skill_max: bool,
) -> SupportSeedSlim {
    let master = indexes
        .card_data(card.game_card_id)
        .map(|entry| &entry.master);
    let master_rank = if support_master_max {
        master
            .and_then(|master| master.max_master_rank)
            .unwrap_or(card.master_rank)
    } else {
        card.master_rank
    };
    let skill_level = if support_skill_max {
        master
            .and_then(|master| master.max_skill_level)
            .unwrap_or(card.skill_level)
    } else {
        card.skill_level
    };
    SupportSeedSlim {
        card_id: card.game_card_id.max(0).min(u16::MAX as i32) as u16,
        rarity: card.card_rarity_type,
        character_id: card.character_id,
        unit_mask: card.unit_mask_raw,
        master_rank,
        skill_level,
    }
}

/// Precomputed per-(event, turn, special-character) support bonus rate tables.
struct SupportRateTables {
    valid: bool,
    special_character_id: i32,
    special_unit_mask: u8,
    row_present: [bool; 6],
    char_specific: [f64; 6],
    char_others: [f64; 6],
    mr_bonus: [[f64; 8]; 6],
    sl_bonus: [[f64; 8]; 6],
    limited_by_card: std::collections::HashMap<i32, f64>,
}

impl SupportRateTables {
    fn new(
        game: &types::GameData<'_>,
        event_id: i32,
        turn: Option<i32>,
        special_character_id: Option<i32>,
    ) -> Self {
        let mut tables = Self {
            valid: false,
            special_character_id: 0,
            special_unit_mask: 0,
            row_present: [false; 6],
            char_specific: [0.0; 6],
            char_others: [0.0; 6],
            mr_bonus: [[0.0; 8]; 6],
            sl_bonus: [[0.0; 8]; 6],
            limited_by_card: std::collections::HashMap::new(),
        };
        let Some(special_character_id) = special_character_id.filter(|id| *id > 0) else {
            return tables;
        };
        let Some(special_unit) = game
            .game_character_units
            .iter()
            .find(|entry| entry.game_character_id == special_character_id)
            .and_then(|entry| parse_unit_code(&entry.unit))
            .and_then(types::unit_to_pool_index)
        else {
            return tables;
        };
        tables.valid = true;
        tables.special_character_id = special_character_id;
        tables.special_unit_mask = 1u8 << special_unit;

        let table = match turn {
            Some(1) => game.wb_support_deck_bonuses_wl1,
            Some(2) => game.wb_support_deck_bonuses_wl2,
            Some(3) => game.wb_support_deck_bonuses_wl3,
            _ => &[],
        };
        for rarity in 1..6usize {
            let Some(row) = table
                .iter()
                .find(|entry| support_rarity_matches(&entry.card_rarity_type, rarity as i32))
            else {
                continue;
            };
            tables.row_present[rarity] = true;
            tables.char_specific[rarity] = support_char_bonus(row, "specific");
            tables.char_others[rarity] = support_char_bonus(row, "others");
            for mr in 0..8i32 {
                tables.mr_bonus[rarity][mr as usize] = row
                    .world_bloom_support_deck_master_rank_bonuses
                    .iter()
                    .find(|entry| entry.master_rank == mr)
                    .map(|entry| entry.bonus_rate)
                    .unwrap_or(0.0);
            }
            for sl in 0..8i32 {
                tables.sl_bonus[rarity][sl as usize] = row
                    .world_bloom_support_deck_skill_level_bonuses
                    .iter()
                    .find(|entry| entry.skill_level == sl)
                    .map(|entry| entry.bonus_rate)
                    .unwrap_or(0.0);
            }
        }
        for bonus in game.world_bloom_support_deck_unit_event_limited_bonuses {
            if bonus.event_id == event_id && bonus.game_character_id == special_character_id {
                *tables.limited_by_card.entry(bonus.card_id).or_insert(0.0) += bonus.bonus_rate;
            }
        }
        tables
    }

    #[inline]
    fn bonus(&self, seed: &SupportSeedSlim) -> f64 {
        if !self.valid {
            return 0.0;
        }
        if seed.unit_mask & self.special_unit_mask == 0 {
            return 0.0;
        }
        let rarity = seed.rarity;
        if !(1..6).contains(&rarity) || !self.row_present[rarity as usize] {
            return 0.0;
        }
        let rarity = rarity as usize;
        let mut total = if seed.character_id as i32 == self.special_character_id {
            self.char_specific[rarity]
        } else {
            self.char_others[rarity]
        };
        if (0..8).contains(&seed.master_rank) {
            total += self.mr_bonus[rarity][seed.master_rank as usize];
        }
        if (0..8).contains(&seed.skill_level) {
            total += self.sl_bonus[rarity][seed.skill_level as usize];
        }
        if let Some(limited) = self.limited_by_card.get(&(seed.card_id as i32)) {
            total += *limited;
        }
        if !total.is_finite() || total <= 0.0 {
            0.0
        } else {
            total
        }
    }
}

fn support_rarity_matches(code: &str, card_rarity_type: i32) -> bool {
    let trimmed = code.trim();
    let matches_ascii = |target: &str| trimmed.eq_ignore_ascii_case(target);
    match card_rarity_type {
        1 => matches_ascii("rarity_1") || matches_ascii("1"),
        2 => matches_ascii("rarity_2") || matches_ascii("2"),
        3 => matches_ascii("rarity_3") || matches_ascii("3"),
        4 => matches_ascii("rarity_4") || matches_ascii("4"),
        5 => matches_ascii("rarity_birthday") || matches_ascii("birthday") || matches_ascii("5"),
        _ => false,
    }
}

fn support_char_bonus(table: &types::WBSupportDeckBonus, character_type: &str) -> f64 {
    table
        .world_bloom_support_deck_character_bonuses
        .iter()
        .find(|entry| {
            entry
                .world_bloom_support_deck_character_type
                .eq_ignore_ascii_case(character_type)
        })
        .map(|entry| entry.bonus_rate)
        .unwrap_or(0.0)
}

fn build_support_deck_fast(
    seeds: &[SupportSeedSlim],
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
    special_character_id: Option<i32>,
) -> SupportDeck {
    let Some(event_ctx) = event_ctx else {
        return SupportDeck::default();
    };
    if event_ctx.support_deck_count == 0 {
        return SupportDeck::default();
    }
    let special_character_id = special_character_id.or(event_ctx.world_bloom_character_id);
    let tables = SupportRateTables::new(
        game,
        event_ctx.event_id,
        event_ctx.world_bloom_event_turn,
        special_character_id,
    );
    let mut cards: Vec<(u16, f64)> = Vec::with_capacity(seeds.len());
    for seed in seeds {
        cards.push((seed.card_id, tables.bonus(seed)));
    }
    cards.sort_by(|left, right| right.1.total_cmp(&left.1));
    SupportDeck {
        cards,
        count: event_ctx.support_deck_count,
    }
}

fn build_final_chapter_support_decks_fast(
    seeds: &[SupportSeedSlim],
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
) -> Vec<SupportDeck> {
    let mut decks = vec![SupportDeck::default(); 27];
    let Some(event_ctx) = event_ctx else {
        return decks;
    };
    if event_ctx.event_id != FINAL_CHAPTER_EVENT_ID {
        return decks;
    }
    for character_id in 1..=26 {
        decks[character_id as usize] =
            build_support_deck_fast(seeds, game, Some(event_ctx), Some(character_id));
    }
    decks
}

fn build_search_context(
    gathered: GatheredContext,
    support_seeds: &[SupportSeedSlim],
    game: &types::GameData<'_>,
    params: &types::BuildParams,
    event_ctx: Option<&EventContext>,
    music: Option<&music::MusicParams>,
    fixed_card_ids: Vec<u16>,
    fixed_character_ids: Vec<u8>,
) -> SearchContext {
    let card_count = gathered.skill_max.len();
    let support_deck = build_support_deck_fast(support_seeds, game, event_ctx, None);
    let support_decks_by_character =
        build_final_chapter_support_decks_fast(support_seeds, game, event_ctx);
    let support_bonus_top_sum = support_deck
        .cards
        .iter()
        .take(support_deck.count as usize)
        .map(|(_, bonus)| *bonus)
        .sum::<f64>();
    let support_bonus_top_sum_by_character = support_decks_by_character
        .iter()
        .map(|deck| {
            deck.cards
                .iter()
                .take(deck.count as usize)
                .map(|(_, bonus)| *bonus)
                .sum::<f64>()
        })
        .fold(0.0_f64, f64::max)
        .ceil() as u32;
    let diff_attr_bonus = event_ctx.map(|ctx| ctx.diff_attr_bonus).unwrap_or([0; 6]);
    let mut top_skill_values = [0u8; 5];
    for &value in &gathered.skill_max {
        if value <= top_skill_values[4] {
            continue;
        }
        top_skill_values[4] = value;
        let mut slot = 4;
        while slot > 0 && top_skill_values[slot] > top_skill_values[slot - 1] {
            top_skill_values.swap(slot, slot - 1);
            slot -= 1;
        }
    }
    let skill_ub_global = top_skill_values.into_iter().map(u32::from).sum::<u32>();

    SearchContext {
        target: params.target,
        fixed_card_ids,
        fixed_character_ids,
        forced_leader_character_id: if event_ctx
            .is_some_and(|ctx| ctx.event_id == FINAL_CHAPTER_EVENT_ID)
        {
            params
                .forced_leader_character_id
                .filter(|id| (1..=26).contains(id))
                .map(|id| id as u8)
        } else {
            None
        },
        music_rate_pct: music.map(|music| music.event_rate_pct).unwrap_or(100),
        boost_rate_pct: normalize_boost_rate_pct(params.boost),
        base_score: music.map(|music| music.meta.base_score).unwrap_or(1.0),
        base_score_auto: music.map(|music| music.meta.base_score_auto).unwrap_or(1.0),
        fever_score: music.map(|music| music.meta.fever_score).unwrap_or(0.0),
        skill_scores: music
            .map(|music| {
                [
                    music.meta.solo_skill_scores,
                    music.meta.multi_skill_scores,
                    music.meta.auto_skill_scores,
                ]
            })
            .unwrap_or([[0.0; 6]; 3]),
        other_score: params.other_score.unwrap_or(0),
        life: params.life.unwrap_or(1000),
        diff_attr_bonus,
        support_deck,
        support_decks_by_character,
        is_world_bloom: event_ctx
            .is_some_and(|ctx| matches!(ctx.event_type, crate::types::EventType::WorldBloom)),
        is_final_chapter: event_ctx.is_some_and(|ctx| ctx.event_id == FINAL_CHAPTER_EVENT_ID),
        enforce_char_uniqueness: !matches!(
            params.live_type,
            crate::types::LiveType::Challenge | crate::types::LiveType::ChallengeAuto
        ),
        minimize: params.minimize,
        live_type: params.live_type,
        event_type: event_ctx.map(|ctx| ctx.event_type),
        keep_after_training_state: params.keep_after_training_state,
        skill_reference_strategy: params.skill_reference_strategy,
        best_skill_as_leader: params.best_skill_as_leader,
        live_skill_order: params.live_skill_order,
        specific_skill_order: params.specific_skill_order,
        multi_teammate_score_up: params.multi_teammate_score_up,
        multi_teammate_power: params.multi_teammate_power,
        multi_live_score_up_lower_bound: params.multi_live_score_up_lower_bound,
        extra_bonus_ub: diff_attr_bonus.into_iter().max().unwrap_or(0) as u32
            + support_bonus_top_sum.ceil() as u32
            + support_bonus_top_sum_by_character,
        w_power: 1.0,
        w_bonus: 1.0,
        skill_ub_global,
        card_bonus_count_limit: event_ctx.map(|ctx| ctx.card_bonus_count_limit).unwrap_or(5),
        honor_bonus: 0,
        power_total_cap: event_ctx.and_then(|ctx| {
            if matches!(ctx.event_type, crate::types::EventType::WorldBloom)
                && ctx.world_bloom_event_turn == Some(3)
            {
                Some(336_000)
            } else {
                None
            }
        }),
        leader_honor_bonus: gathered.leader_honor_bonus,
        leader_limit_bonus: gathered.leader_limit_bonus,
        final_chapter_member_keep: vec![true; card_count],
        skill_is_after_training: gathered.skill_is_after_training,
        trained_to_special_image: gathered.trained_to_special_image,
    }
}

fn compute_honor_bonus(user: &types::UserProfile, indexes: &index::PoolIndexes) -> u32 {
    user.user_honors
        .iter()
        .map(|honor| indexes.honor_bonus(honor.honor_id, honor.level))
        .sum()
}

fn resolve_fixture_bonus_limit(
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
) -> Option<i32> {
    let event_id = event_ctx?.event_id;
    game.event_mysekai_fixture_performance_bonus_limits
        .iter()
        .find(|entry| entry.event_id == event_id)
        .map(|entry| entry.bonus_rate_limit)
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

fn build_card_pool_fully_prepared_internal(
    prepared: &PreparedGameData<'_>,
    build: &PreparedPoolBuild<'_>,
    include_details: bool,
) -> Result<(crate::pool::CardPool, SearchContext, Vec<FullPrecisionCard>), BuildError> {
    let game = prepared.game();
    let indexes = prepared.indexes.as_ref();
    let params = &build.params;
    let event_ctx = build.event_ctx.as_ref();
    let music = build.music.as_ref();
    let prepared_cards = &build.cards;
    let mut cards = Vec::with_capacity(prepared_cards.len());
    let needs_support_cards = event_ctx
        .is_some_and(|ctx| ctx.support_deck_count > 0 || ctx.event_id == FINAL_CHAPTER_EVENT_ID);
    let mut support_seeds: Vec<SupportSeedSlim> = if needs_support_cards {
        Vec::with_capacity(prepared_cards.len())
    } else {
        Vec::new()
    };
    let mut support_seen: std::collections::HashSet<u16> = if needs_support_cards {
        std::collections::HashSet::with_capacity(prepared_cards.len())
    } else {
        std::collections::HashSet::new()
    };

    for (prepared_card, power) in prepared_cards.iter().zip(build.powers.iter().copied()) {
        let master = prepared_card.master;
        let user_card = prepared_card.user_card.as_ref();
        let unit_mask_raw = prepared_card.unit_mask;
        let attr = prepared_card.attr;
        let event_bonus = prepared_card.event_bonus;
        let has_char_bonus = prepared_card.has_char_bonus;
        let has_attr_bonus = prepared_card.has_attr_bonus;
        let leader_honor_bonus = prepared_card.leader_honor_bonus;
        let leader_limit_bonus = prepared_card.leader_limit_bonus;
        let skill_options = prepared_card.skill_options.clone();
        let skill_state_controls_image = prepared_card.skill_state_controls_image;

        for (skill_state, skill) in skill_options.into_iter().flatten() {
            let default_image = if skill_state_controls_image {
                match skill_state {
                    SkillState::BeforeTraining => DefaultImage::Original,
                    SkillState::AfterTraining => DefaultImage::SpecialTraining,
                }
            } else {
                prepared_card.default_image
            };

            let ep_sort_key =
                i64::from(power.power_max) + i64::from(event_bonus.total_ceil() as i32) * 1_000;
            let intermediate = CardIntermediate {
                game_card_id: master.id,
                card_rarity_type: master.card_rarity_type,
                character_id: master.character_id.clamp(0, u8::MAX as i32) as u8,
                attr,
                unit_mask_raw,
                default_image,
                after_training: prepared_card.after_training,
                skill_state_controls_image,
                master_rank: user_card.master_rank,
                skill_level: user_card.skill_level,
                power,
                skill,
                event_bonus,
                has_char_bonus,
                has_attr_bonus,
                leader_honor_bonus,
                leader_limit_bonus,
                ep_sort_key,
            };

            if needs_support_cards {
                let seed = support_seed_from_intermediate(
                    &intermediate,
                    indexes,
                    params.support_master_max,
                    params.support_skill_max,
                );
                if support_seen.insert(seed.card_id) {
                    support_seeds.push(seed);
                }
            }
            if keep_card(&intermediate, params) {
                cards.push(intermediate);
            }
        }
    }

    if params.filter_other_unit {
        if let Some(unit) = event_ctx.and_then(|ctx| ctx.filter_unit) {
            if let Some(unit_index) = types::unit_to_pool_index(unit) {
                let wanted = 1u8 << unit_index;
                let piapro = types::unit_to_pool_index(crate::types::Unit::Piapro)
                    .map(|index| 1u8 << index)
                    .unwrap_or(0);
                cards.retain(|card| {
                    card.unit_mask_raw & wanted != 0 || card.unit_mask_raw == piapro
                });
            }
        }
    }

    let is_world_bloom =
        event_ctx.is_some_and(|ctx| matches!(ctx.event_type, crate::types::EventType::WorldBloom));
    let is_final_chapter = event_ctx.is_some_and(|ctx| ctx.event_id == FINAL_CHAPTER_EVENT_ID);
    if build.ep_prefilter_applied {
        per_character_trim(&mut cards, params, PER_CHAR_KEEP);
    } else if event_ctx.is_some()
        && !matches!(
            params.target,
            crate::types::ScoreTarget::Power | crate::types::ScoreTarget::Skill
        )
        && cards.len() > EP_PREFILTER_MIN_POOL
        && cards
            .iter()
            .any(|card| ep_prefilter_keep(card, is_world_bloom, is_final_chapter))
    {
        cards.retain(|card| {
            ep_prefilter_keep_with_params(card, params, is_world_bloom, is_final_chapter)
        });
        if is_world_bloom || is_final_chapter {
            let keep = if is_final_chapter {
                FINAL_CHAPTER_PER_CHAR_KEEP
            } else {
                PER_CHAR_KEEP
            };
            per_character_trim(&mut cards, params, keep);
        } else {
            per_character_trim(&mut cards, params, PER_CHAR_KEEP);
        }
    }

    let is_challenge_live = matches!(
        params.live_type,
        crate::types::LiveType::Challenge | crate::types::LiveType::ChallengeAuto
    );
    let is_challenge_all = is_challenge_live && params.challenge_live_character_id.is_none();

    if is_challenge_live && cards.len() > CHALLENGE_ALL_PER_CHAR_KEEP {
        general_per_character_trim(&mut cards, params, CHALLENGE_ALL_PER_CHAR_KEEP);
    } else if !matches!(
        params.target,
        crate::types::ScoreTarget::Power | crate::types::ScoreTarget::Skill
    ) && !is_challenge_all
        && !is_final_chapter
        && !is_world_bloom
        && cards.len() > GENERAL_TRIM_THRESHOLD
    {
        general_per_character_trim(&mut cards, params, GENERAL_PER_CHAR_KEEP);
    }

    if matches!(
        params.target,
        crate::types::ScoreTarget::Power | crate::types::ScoreTarget::Skill
    ) && cards.len() > crate::pool::MASK_WORDS * 64
    {
        target_per_character_trim(&mut cards, params);
    }

    if cards.is_empty() {
        return Err(BuildError::EmptyPool);
    }
    let (fixed_card_ids, fixed_character_ids) = validate_fixed_constraints(params, &cards)?;
    if cards.len() > crate::pool::MASK_WORDS * 64 {
        return Err(BuildError::TooManyCards(cards.len()));
    }

    let effective_live_type = if matches!(params.live_type, crate::types::LiveType::Multi)
        && event_ctx
            .is_some_and(|ctx| matches!(ctx.event_type, crate::types::EventType::CheerfulCarnival))
    {
        crate::types::LiveType::Cheerful
    } else {
        params.live_type
    };
    let (pool, full, gathered) = sort_and_gather(
        cards,
        params.target,
        event_ctx.is_some(),
        effective_live_type,
        &fixed_card_ids,
        &fixed_character_ids,
        include_details,
    );
    let mut search_ctx = build_search_context(
        gathered,
        &support_seeds,
        game,
        params,
        event_ctx,
        music,
        fixed_card_ids,
        fixed_character_ids,
    );
    search_ctx.honor_bonus = build.honor_bonus;
    Ok((pool, search_ctx, full))
}

/// 返回应用养成配置（preset / 单卡覆盖）后的用户卡列表，养成口径与 `build_card_pool` 完全同源。
///
/// 渲染层需要展示「评分实际使用的养成值」（满级/满技能/满破/已读剧情/画布），而非玩家原始
/// 卡况；否则开养成开关（甚至默认 preset 就假设满级）时分数变了、卡面显示却没变。
/// 复用建池的 `normalize_user_cards` + `enrich_master` + `apply_card_config`，保证渲染显示与
/// 评分养成值零漂移。被 `config.disable` 过滤掉的卡不出现（与池一致；这类卡也进不了结果）。
/// 只建一张轻量 card-by-id 索引，不做 power/skill 计算，开销远小于一次完整建池。
pub fn cultivated_user_cards(
    user: &types::UserProfile,
    game: &types::GameData<'_>,
    params: &types::BuildParams,
) -> Vec<types::UserCard> {
    let configs = merged_configs(params);
    let normalized_cards = normalize_user_cards(user, params, game);
    let mut card_by_id =
        std::collections::HashMap::<i32, &types::MasterCard>::with_capacity(game.cards.len());
    for card in game.cards {
        card_by_id.entry(card.id).or_insert(card);
    }

    let mut cultivated = Vec::with_capacity(normalized_cards.len());
    for original_user_card in normalized_cards {
        let Some(master) = card_by_id.get(&original_user_card.card_id).copied() else {
            continue;
        };
        let master = enrich_master(master, game);
        let mut user_card = original_user_card.clone();
        if !apply_card_config(
            &mut user_card,
            &master,
            &configs,
            game.card_rarities,
            game.card_episodes,
        ) {
            continue;
        }
        cultivated.push(user_card);
    }
    cultivated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{RefSkill, SkillSlot};
    use crate::types::{LiveType, ScoreTarget, SkillReferenceStrategy};

    fn sample_game<'a>(
        cards: &'a [MasterCard],
        params: &'a [types::CardParameter],
        rarities: &'a [types::CardRarity],
        episodes: &'a [types::CardEpisode],
        lessons: &'a [types::MasterLesson],
        skills: &'a [types::Skill],
        effects: &'a [types::SkillEffect],
        area_items: &'a [types::AreaItemLevel],
        units: &'a [types::GameCharacterUnit],
    ) -> GameData<'a> {
        GameData {
            cards,
            card_parameters: params,
            card_rarities: rarities,
            card_episodes: episodes,
            master_lessons: lessons,
            skills,
            skill_effects: effects,
            area_item_levels: area_items,
            game_character_units: units,
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            mysekai_gates: &[],
            mysekai_gate_levels: &[],
            events: &[],
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            world_blooms: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &[],
            event_skill_score_up_limits: &[],
            music_metas: &[],
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        }
    }

    fn sample_user_card(card_id: i32) -> UserCard {
        UserCard {
            card_id,
            level: 1,
            skill_level: 1,
            master_rank: 0,
            special_training_status: "none".to_string(),
            default_image: "original".to_string(),
            episodes_read: Vec::new(),
            is_virtual: false,
            has_canvas_bonus_override: None,
        }
    }

    #[test]
    fn bonus_target_requires_non_final_event_context() {
        let game = sample_game(&[], &[], &[], &[], &[], &[], &[], &[], &[]);
        let user = UserProfile::default();

        let no_event = BuildParams {
            target: ScoreTarget::Bonus,
            ..BuildParams::default()
        };
        assert!(matches!(
            build_card_pool(&user, &game, &no_event),
            Err(BuildError::InvalidConfig(reason)) if reason.contains("活动")
        ));

        let final_chapter = BuildParams {
            target: ScoreTarget::Bonus,
            event_id: Some(crate::types::FINAL_CHAPTER_EVENT_ID),
            event_type: Some("world_bloom".to_string()),
            ..BuildParams::default()
        };
        assert!(matches!(
            build_card_pool(&user, &game, &final_chapter),
            Err(BuildError::InvalidConfig(reason)) if reason.contains("终章")
        ));
    }

    #[test]
    fn programmatic_build_params_enforce_compatibility_bounds() {
        let game = sample_game(&[], &[], &[], &[], &[], &[], &[], &[], &[]);
        let user = UserProfile::default();

        for (params, expected) in [
            (
                BuildParams {
                    limit: 0,
                    ..BuildParams::default()
                },
                "limit",
            ),
            (
                BuildParams {
                    timeout_ms: 0,
                    ..BuildParams::default()
                },
                "timeout",
            ),
            (
                BuildParams {
                    timeout_ms: 300_001,
                    ..BuildParams::default()
                },
                "timeout",
            ),
            (
                BuildParams {
                    target_bonus_list: vec![100; 33],
                    ..BuildParams::default()
                },
                "target_bonus_list",
            ),
            (
                BuildParams {
                    custom_bonus_character_ids: vec![0],
                    ..BuildParams::default()
                },
                "character",
            ),
            (
                BuildParams {
                    custom_bonus_character_ids: vec![1; 27],
                    ..BuildParams::default()
                },
                "character",
            ),
            (
                BuildParams {
                    custom_bonus_character_ids: vec![1, 1],
                    ..BuildParams::default()
                },
                "重复",
            ),
            (
                BuildParams {
                    custom_bonus_character_support_units: vec![
                        crate::types::CustomSupportUnit {
                            character_id: 21,
                            unit: crate::types::Unit::Idol,
                        },
                        crate::types::CustomSupportUnit {
                            character_id: 21,
                            unit: crate::types::Unit::Street,
                        },
                    ],
                    ..BuildParams::default()
                },
                "重复",
            ),
        ] {
            assert!(matches!(
                build_card_pool(&user, &game, &params),
                Err(BuildError::InvalidConfig(reason)) if reason.contains(expected)
            ));
        }
    }

    #[test]
    fn exact_card_config_values_validate_their_public_ranges() {
        for config in [
            types::CardRarityConfig {
                level: Some(0),
                ..Default::default()
            },
            types::CardRarityConfig {
                skill_level: Some(0),
                ..Default::default()
            },
            types::CardRarityConfig {
                master_rank: Some(6),
                ..Default::default()
            },
            types::CardRarityConfig {
                episode_read_count: Some(3),
                ..Default::default()
            },
        ] {
            let mut params = BuildParams::default();
            params.card_configs.rarity_4_config = config;
            assert!(matches!(
                validate_build_params(&params),
                Err(BuildError::InvalidConfig(_))
            ));
        }
    }

    #[test]
    fn boost_is_fire_count_piecewise_multiplier() {
        assert_eq!(normalize_boost_rate_pct(Some(0)), 100);
        assert_eq!(normalize_boost_rate_pct(Some(1)), 500);
        assert_eq!(normalize_boost_rate_pct(Some(5)), 2500);
        assert_eq!(normalize_boost_rate_pct(Some(10)), 3500);
        assert_eq!(normalize_boost_rate_pct(Some(11)), 100);
        assert_eq!(normalize_boost_rate_pct(None), 100);
    }

    #[test]
    fn hard_unit_filter_keeps_virtual_singer_support_unit_only_when_matching() {
        let cards = [MasterCard {
            id: 1,
            character_id: 21,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "rarity_4".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: Some("light_sound".to_string()),
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let params = [types::CardParameter {
            card_id: 1,
            level: 1,
            param1: 100,
            param2: 100,
            param3: 100,
        }];
        let units = [types::GameCharacterUnit {
            game_character_id: 21,
            unit: "piapro".to_string(),
        }];
        let game = sample_game(&cards, &params, &[], &[], &[], &[], &[], &[], &units);
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            ..UserProfile::default()
        };

        let ln_params = BuildParams {
            unit_filter: Some("light_sound".to_string()),
            ..BuildParams::default()
        };
        let (pool, _) = build_card_pool(&user, &game, &ln_params).unwrap();
        assert_eq!(pool.count(), 1);

        let mmj_params = BuildParams {
            unit_filter: Some("idol".to_string()),
            ..BuildParams::default()
        };
        assert_eq!(
            build_card_pool(&user, &game, &mmj_params).unwrap_err(),
            BuildError::EmptyPool
        );
    }

    #[test]
    fn handler_build_power_uses_f32_item_accumulation() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let params = [types::CardParameter {
            card_id: 1,
            level: 1,
            param1: 101,
            param2: 101,
            param3: 101,
        }];
        let area_items = [
            types::AreaItemLevel {
                area_item_id: 1,
                level: 1,
                unit: None,
                attr: None,
                character_id: None,
                power_rate: 1.0,
                power_all_match_rate: 1.0,
            },
            types::AreaItemLevel {
                area_item_id: 2,
                level: 1,
                unit: None,
                attr: None,
                character_id: None,
                power_rate: 1.0,
                power_all_match_rate: 1.0,
            },
        ];
        let game_units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let game = sample_game(
            &cards,
            &params,
            &[],
            &[],
            &[],
            &[],
            &[],
            &area_items,
            &game_units,
        );
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            user_area_items: vec![
                types::UserAreaItem {
                    area_item_id: 1,
                    level: 1,
                },
                types::UserAreaItem {
                    area_item_id: 2,
                    level: 1,
                },
            ],
            ..UserProfile::default()
        };

        let idx = index::PoolIndexes::build(&game);
        let power_ctx = PreparedPowerContext::new(&user, &game, &idx, None);
        let result = power::build_power(
            &sample_user_card(1),
            &cards[0],
            &power_ctx,
            &idx,
            idx.unit_mask(cards[0].id),
            idx.attr(cards[0].id).unwrap(),
        );
        let scalar = power::build_power_scalar_reference(
            &sample_user_card(1),
            &cards[0],
            &power_ctx,
            &idx,
            idx.unit_mask(cards[0].id),
            idx.attr(cards[0].id).unwrap(),
        );
        assert_eq!(result, scalar);
        assert_eq!(result.detail(1, 0).area_item_bonus, 6);
        assert_eq!(result.detail(1, 0).total, 309);
        assert_eq!(result.detail(0, 0), crate::types::PowerDetail::default());
        assert!(std::mem::size_of::<power::PowerResult>() <= 128);
    }

    #[test]
    fn handler_build_card_pool_only_clamps_fixture_bonus_for_matching_event() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let params = [types::CardParameter {
            card_id: 1,
            level: 1,
            param1: 100,
            param2: 100,
            param3: 100,
        }];
        let skills = [types::Skill {
            id: 10,
            level: 1,
            is_after_training: false,
        }];
        let effects = [types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 100,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        }];
        let units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let events = [types::Event {
            id: FINAL_CHAPTER_EVENT_ID,
            event_type: "marathon".to_string(),
        }];
        let fixture_limits = [types::EventFixtureBonusLimit {
            event_id: FINAL_CHAPTER_EVENT_ID,
            bonus_rate_limit: 20,
        }];
        let game = GameData {
            cards: &cards,
            card_parameters: &params,
            card_rarities: &[],
            card_episodes: &[],
            master_lessons: &[],
            skills: &skills,
            skill_effects: &effects,
            area_item_levels: &[],
            game_character_units: &units,
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            mysekai_gates: &[],
            mysekai_gate_levels: &[],
            events: &events,
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            world_blooms: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &fixture_limits,
            event_skill_score_up_limits: &[],
            music_metas: &[],
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        };
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            user_mysekai_fixture_bonuses: vec![types::UserFixtureBonus {
                character_id: 1,
                event_id: None,
                total_bonus_rate: 30,
            }],
            ..UserProfile::default()
        };

        let (pool, _) = build_card_pool(&user, &game, &BuildParams::default()).unwrap();
        let idx = pool.card_idx(0).unwrap();
        assert_eq!(pool.power_max(idx), 309);

        let params = BuildParams {
            event_id: Some(FINAL_CHAPTER_EVENT_ID),
            fixed_characters: vec![1],
            ..BuildParams::default()
        };
        let (pool, _) = build_card_pool(&user, &game, &params).unwrap();
        let idx = pool.card_idx(0).unwrap();
        assert_eq!(pool.power_max(idx), 306);
    }

    #[test]
    fn handler_final_chapter_allows_auto_leader_without_fixed_character() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let params = [types::CardParameter {
            card_id: 1,
            level: 1,
            param1: 100,
            param2: 100,
            param3: 100,
        }];
        let skills = [types::Skill {
            id: 10,
            level: 1,
            is_after_training: false,
        }];
        let effects = [types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 100,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        }];
        let units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let events = [types::Event {
            id: FINAL_CHAPTER_EVENT_ID,
            event_type: "marathon".to_string(),
        }];
        let game = sample_game(
            &cards,
            &params,
            &[],
            &[],
            &[],
            &skills,
            &effects,
            &[],
            &units,
        );
        let game = GameData {
            events: &events,
            ..game
        };
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            ..UserProfile::default()
        };
        let params = BuildParams {
            event_id: Some(FINAL_CHAPTER_EVENT_ID),
            ..BuildParams::default()
        };

        let (_, ctx) = build_card_pool(&user, &game, &params)
            .expect("终章无固定队长应允许进入自动 leader 搜索路径");
        assert!(ctx.is_final_chapter);
        assert!(!ctx.has_fixed_leader());
    }

    #[test]
    fn handler_build_skill_covers_normal_unit_count_diff_and_ref() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let game_units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let skills = [types::Skill {
            id: 10,
            level: 1,
            is_after_training: false,
        }];
        let empty_game = sample_game(&cards, &[], &[], &[], &[], &skills, &[], &[], &game_units);

        let normal_effects = [types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up".to_string(),
            value: 120,
            additional_value: None,
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        }];
        let game = GameData {
            skill_effects: &normal_effects,
            ..empty_game
        };
        let idx = index::PoolIndexes::build(&game);
        let normal = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            &idx,
            0,
            Some(140),
            SkillState::BeforeTraining,
        );
        assert_eq!(
            normal.slot,
            SkillSlot {
                skill_type: 0,
                value: 120
            }
        );

        let unit_count_effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up_unit_count".to_string(),
                value: 10,
                additional_value: None,
                unit_member_count: Some(1),
                unit: Some("idol".to_string()),
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up_unit_count".to_string(),
                value: 50,
                additional_value: None,
                unit_member_count: Some(5),
                unit: Some("idol".to_string()),
                activate_character_rank: None,
            },
        ];
        let game = GameData {
            skill_effects: &unit_count_effects,
            ..empty_game
        };
        let idx = index::PoolIndexes::build(&game);
        let unit_count = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            &idx,
            0,
            None,
            SkillState::BeforeTraining,
        );
        assert_eq!(unit_count.slot.skill_type, 1);
        assert_eq!(
            unit_count
                .unit_count
                .as_ref()
                .map(|entry| entry.score_up[0]),
            Some(10)
        );
        assert_eq!(
            unit_count
                .unit_count
                .as_ref()
                .map(|entry| entry.score_up[4]),
            Some(50)
        );

        let diff_effects = [types::SkillEffect {
            skill_id: 10,
            skill_level: 1,
            effect_type: "score_up_diff".to_string(),
            value: 20,
            additional_value: Some(5),
            unit_member_count: None,
            unit: None,
            activate_character_rank: None,
        }];
        let game = GameData {
            skill_effects: &diff_effects,
            ..empty_game
        };
        let idx = index::PoolIndexes::build(&game);
        let diff = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            &idx,
            0,
            None,
            SkillState::BeforeTraining,
        );
        assert_eq!(
            diff.diff,
            Some(crate::pool::DiffSkill {
                base: 20,
                increment: 5
            })
        );

        let ref_effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 100,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up_reference".to_string(),
                value: 20,
                additional_value: Some(60),
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
        ];
        let game = GameData {
            skill_effects: &ref_effects,
            ..empty_game
        };
        let idx = index::PoolIndexes::build(&game);
        let ref_skill = build_skill(
            &sample_user_card(1),
            &cards[0],
            &game,
            &idx,
            0,
            Some(140),
            SkillState::BeforeTraining,
        );
        assert_eq!(
            ref_skill.ref_skill,
            Some(crate::pool::RefSkill { rate: 20, max: 40 })
        );
        assert_eq!(ref_skill.skill_max, 140);
    }

    #[test]
    fn handler_build_card_pool_splits_bfes_reference_skill_cards() {
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: Some(11),
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let skills = [
            types::Skill {
                id: 10,
                level: 1,
                is_after_training: false,
            },
            types::Skill {
                id: 11,
                level: 1,
                is_after_training: true,
            },
        ];
        let effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 80,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up_reference".to_string(),
                value: 50,
                additional_value: Some(70),
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 11,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 120,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
        ];
        let units = [types::GameCharacterUnit {
            game_character_id: 1,
            unit: "idol".to_string(),
        }];
        let game = sample_game(&cards, &[], &[], &[], &[], &skills, &effects, &[], &units);
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            ..UserProfile::default()
        };
        let params = BuildParams {
            target: ScoreTarget::Skill,
            ..BuildParams::default()
        };

        let (pool, _, full) = build_card_pool_with_details(&user, &game, &params)
            .expect("dual-skill pool should build");
        assert_eq!(pool.count(), 2);
        assert!(full.iter().all(|card| card.skill_state_controls_image));
        assert_eq!(
            pool.skill_max(pool.card_idx(1).expect("before skill entry")),
            120
        );
    }

    #[test]
    fn specialized_unit_count_skill_pair_keeps_both_image_states() {
        let mut before = SkillResult::default();
        before.full.skill_id = 24;
        before.unit_count = Some(crate::pool::UnitCountSkill {
            unit: 1,
            score_up: [30, 60, 90, 120, 150],
        });
        let mut after = SkillResult::default();
        after.full.skill_id = 22;

        assert!(is_bfes_skill_pair(&before, &after));
    }

    #[test]
    fn handler_apply_card_config_supports_override_and_disable() {
        let master = MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        };
        let mut user_card = sample_user_card(1);
        let mut configs = CardConfigSet::default();
        configs.rarity_4_config.level_max = true;
        configs.single_card_configs.push(types::SingleCardConfig {
            card_id: 1,
            config: types::CardRarityConfig {
                disable: true,
                ..types::CardRarityConfig::default()
            },
        });
        assert!(!apply_card_config(
            &mut user_card,
            &master,
            &configs,
            &[],
            &[],
        ));

        let mut user_card = sample_user_card(1);
        let mut configs = CardConfigSet::default();
        configs.rarity_4_config.level_max = true;
        configs.rarity_4_config.skill_max = true;
        configs.rarity_4_config.master_max = true;
        assert!(apply_card_config(
            &mut user_card,
            &master,
            &configs,
            &[],
            &[],
        ));
        assert_eq!(user_card.level, 60);
        assert_eq!(user_card.skill_level, 4);
        assert_eq!(user_card.master_rank, 5);
    }

    #[test]
    fn handler_level_max_marks_trainable_cards_after_training() {
        let master = MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: Some(11),
            special_training_power1_bonus_fixed: 100,
            special_training_power2_bonus_fixed: 100,
            special_training_power3_bonus_fixed: 100,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        };
        let mut user_card = sample_user_card(1);
        user_card.level = 1;
        user_card.special_training_status = "not_doing".to_string();
        user_card.default_image = "original".to_string();
        let mut configs = CardConfigSet::default();
        configs.rarity_4_config.level_max = true;

        assert!(apply_card_config(
            &mut user_card,
            &master,
            &configs,
            &[types::CardRarity {
                card_rarity_type: 4,
                max_level: 60,
                normal_max_level: 50,
                max_skill_level: 4,
            }],
            &[],
        ));

        assert_eq!(user_card.level, 60);
        assert_eq!(user_card.special_training_status, "done");
        assert_eq!(user_card.default_image, "special_training");
    }

    #[test]
    fn handler_exact_card_config_overrides_max_flags_and_uses_episode_ids() {
        let master = MasterCard {
            id: 7,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "rarity_4".to_string(),
            asset_bundle_name: "card_000007".to_string(),
            skill_id: 10,
            special_training_skill_id: Some(11),
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        };
        let rarities = [types::CardRarity {
            card_rarity_type: 4,
            max_level: 60,
            normal_max_level: 50,
            max_skill_level: 4,
        }];
        let episodes = [
            types::CardEpisode {
                card_id: 7,
                episode_no: 702,
                power1_bonus_fixed: 0,
                power2_bonus_fixed: 0,
                power3_bonus_fixed: 0,
            },
            types::CardEpisode {
                card_id: 7,
                episode_no: 701,
                power1_bonus_fixed: 0,
                power2_bonus_fixed: 0,
                power3_bonus_fixed: 0,
            },
        ];
        let mut configs = CardConfigSet {
            rarity_4_config: types::CardRarityConfig {
                level_max: true,
                level: Some(51),
                skill_max: true,
                skill_level: Some(2),
                master_max: true,
                master_rank: Some(3),
                episode_read: true,
                episode_read_count: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut user_card = sample_user_card(7);

        assert!(apply_card_config(
            &mut user_card,
            &master,
            &configs,
            &rarities,
            &episodes,
        ));
        assert_eq!(user_card.level, 51);
        assert_eq!(user_card.skill_level, 2);
        assert_eq!(user_card.master_rank, 3);
        assert_eq!(user_card.episodes_read, vec![701]);
        assert_eq!(user_card.special_training_status, "done");
        assert_eq!(user_card.default_image, "special_training");

        configs.rarity_4_config.level = Some(50);
        let mut user_card = sample_user_card(7);
        assert!(apply_card_config(
            &mut user_card,
            &master,
            &configs,
            &rarities,
            &episodes,
        ));
        assert_eq!(user_card.special_training_status, "not_doing");
        assert_eq!(user_card.default_image, "original");
    }

    #[test]
    fn handler_cultivated_user_cards_matches_pool_cultivation() {
        // 渲染层养成卡况必须与建池同源：满级开关后 level 抬到 max，disable 的卡被剔除。
        let cards = [
            MasterCard {
                id: 1,
                character_id: 1,
                attr: "cool".to_string(),
                card_rarity_type: 4,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: 10,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
            MasterCard {
                id: 2,
                character_id: 2,
                attr: "cute".to_string(),
                card_rarity_type: 1,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: 11,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(20),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
        ];
        let game = sample_game(&cards, &[], &[], &[], &[], &[], &[], &[], &[]);
        let user = UserProfile {
            user_cards: vec![sample_user_card(1), sample_user_card(2)],
            ..UserProfile::default()
        };

        // rarity_4 满级，rarity_1 禁用 → 卡1 level=60、卡2 被剔除。
        let mut params = BuildParams::default();
        params.card_configs.rarity_4_config.level_max = true;
        params.card_configs.rarity_1_config.disable = true;

        let cultivated = cultivated_user_cards(&user, &game, &params);
        assert_eq!(cultivated.len(), 1, "disabled 稀有度应被剔除");
        assert_eq!(cultivated[0].card_id, 1);
        assert_eq!(cultivated[0].level, 60, "满级开关应把 level 抬到 max_level");
    }

    #[test]
    fn handler_cultivated_user_cards_canvas_sets_override() {
        // 画布开关应在养成卡况里置 has_canvas_bonus_override，渲染据此显示画布。
        let cards = [MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 0,
            special_training_power2_bonus_fixed: 0,
            special_training_power3_bonus_fixed: 0,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        }];
        let game = sample_game(&cards, &[], &[], &[], &[], &[], &[], &[], &[]);
        let user = UserProfile {
            user_cards: vec![sample_user_card(1)],
            ..UserProfile::default()
        };
        let mut params = BuildParams::default();
        params.card_configs.rarity_4_config.canvas = true;

        let cultivated = cultivated_user_cards(&user, &game, &params);
        assert_eq!(cultivated.len(), 1);
        assert_eq!(cultivated[0].has_canvas_bonus_override, Some(true));
    }

    #[test]
    fn handler_virtual_fixed_card_training_state_follows_master() {
        // 虚拟固定卡（用户未持有）的训练态应按 master.special_training_skill_id 判定：
        // 可特训卡 → done/special_training（否则渲染成花前、且漏掉特训固定 power 加成）；
        // 不可特训卡 → none/original。
        let cards = [
            MasterCard {
                id: 1,
                character_id: 1,
                attr: "cool".to_string(),
                card_rarity_type: 4,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: 10,
                special_training_skill_id: Some(11),
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
            MasterCard {
                id: 2,
                character_id: 2,
                attr: "cute".to_string(),
                card_rarity_type: 1,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: 20,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(20),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
        ];
        let game = sample_game(&cards, &[], &[], &[], &[], &[], &[], &[], &[]);
        // 用户一张都没有；两张都作为固定卡注入虚拟卡。
        let user = UserProfile::default();
        let params = BuildParams {
            fixed_cards: vec![1, 2],
            ..BuildParams::default()
        };

        let normalized = normalize_user_cards(&user, &params, &game);
        let trainable = normalized
            .iter()
            .find(|card| card.card_id == 1)
            .expect("可特训固定卡应存在");
        assert_eq!(trainable.special_training_status, "done");
        assert_eq!(trainable.default_image, "special_training");
        let untrainable = normalized
            .iter()
            .find(|card| card.card_id == 2)
            .expect("不可特训固定卡应存在");
        assert_eq!(untrainable.special_training_status, "none");
        assert_eq!(untrainable.default_image, "original");
    }

    #[test]
    fn handler_sort_and_gather_reindexes_dense_order() {
        let card = |game_card_id: i32, power_max: i32| CardIntermediate {
            game_card_id,
            card_rarity_type: 4,
            character_id: game_card_id as u8,
            attr: 0,
            unit_mask_raw: 1,
            default_image: crate::types::DefaultImage::Original,
            after_training: false,
            skill_state_controls_image: false,
            master_rank: 0,
            skill_level: 1,
            has_char_bonus: false,
            has_attr_bonus: false,
            power: power::PowerResult {
                power_min: power_max - 10,
                power_max,
                ..power::PowerResult::default()
            },
            skill: skill::SkillResult {
                slot: SkillSlot::default(),
                unit_count: None,
                diff: None,
                ref_skill: None,
                skill_min: 1,
                skill_max: 2,
                full: crate::types::SkillInfo::default(),
            },
            event_bonus: EventBonusExact::from_whole(1, 1),
            leader_honor_bonus: 0,
            leader_limit_bonus: 0,
            ep_sort_key: power_max as i64,
        };
        let (pool, _, _) = sort_and_gather(
            vec![card(1, 100), card(3, 300), card(2, 200)],
            ScoreTarget::Power,
            false,
            LiveType::Solo,
            &[],
            &[],
            false,
        );
        assert_eq!(pool.count(), 3);
        assert_eq!(pool.game_id(pool.card_idx(0).unwrap()), 3);
        assert_eq!(pool.game_id(pool.card_idx(1).unwrap()), 2);
        assert_eq!(pool.game_id(pool.card_idx(2).unwrap()), 1);
    }

    #[test]
    fn handler_sort_and_gather_moves_fixed_card_states_before_members() {
        let card =
            |game_card_id: i32, character_id: u8, power_max: i32, skill_max: u8, default_image| {
                CardIntermediate {
                    game_card_id,
                    card_rarity_type: 4,
                    character_id,
                    attr: 0,
                    unit_mask_raw: 1,
                    default_image,
                    after_training: matches!(
                        default_image,
                        crate::types::DefaultImage::SpecialTraining
                    ),
                    skill_state_controls_image: false,
                    master_rank: 0,
                    skill_level: 1,
                    has_char_bonus: false,
                    has_attr_bonus: false,
                    power: power::PowerResult {
                        power_min: power_max - 10,
                        power_max,
                        ..power::PowerResult::default()
                    },
                    skill: skill::SkillResult {
                        slot: SkillSlot::default(),
                        unit_count: None,
                        diff: None,
                        ref_skill: if game_card_id == 949
                            && matches!(default_image, crate::types::DefaultImage::Original)
                        {
                            Some(RefSkill { rate: 50, max: 70 })
                        } else {
                            None
                        },
                        skill_min: skill_max,
                        skill_max,
                        full: crate::types::SkillInfo {
                            skill_id: if game_card_id == 949 {
                                if matches!(
                                    default_image,
                                    crate::types::DefaultImage::SpecialTraining
                                ) {
                                    2
                                } else {
                                    1
                                }
                            } else {
                                0
                            },
                            is_after_training: matches!(
                                default_image,
                                crate::types::DefaultImage::SpecialTraining
                            ),
                            has_ref: game_card_id == 949
                                && matches!(default_image, crate::types::DefaultImage::Original),
                            ..crate::types::SkillInfo::default()
                        },
                    },
                    event_bonus: EventBonusExact::from_whole(1, 1),
                    leader_honor_bonus: 0,
                    leader_limit_bonus: 0,
                    ep_sort_key: power_max as i64,
                }
            };
        let (pool, full, _) = sort_and_gather(
            vec![
                card(121, 26, 90_000, 110, crate::types::DefaultImage::Original),
                card(949, 17, 70_000, 150, crate::types::DefaultImage::Original),
                card(
                    949,
                    17,
                    70_000,
                    148,
                    crate::types::DefaultImage::SpecialTraining,
                ),
                card(404, 21, 80_000, 120, crate::types::DefaultImage::Original),
            ],
            ScoreTarget::Score,
            true,
            LiveType::Multi,
            &[949],
            &[],
            true,
        );

        assert_eq!(pool.game_id(pool.card_idx(0).unwrap()), 949);
        assert_eq!(pool.game_id(pool.card_idx(1).unwrap()), 949);
        assert_eq!(full[0].game_card_id, 949);
        assert_eq!(full[1].game_card_id, 949);
        assert!(matches!(
            full[0].default_image,
            crate::types::DefaultImage::SpecialTraining
        ));
        assert!(full[0].after_training);
        assert!(!full[1].after_training);
    }

    #[test]
    fn handler_ordinary_trained_card_without_after_skill_uses_trained_art() {
        let mut user_card = sample_user_card(1);
        user_card.special_training_status = "done".to_string();
        user_card.default_image = "original".to_string();
        let master = MasterCard {
            id: 1,
            character_id: 1,
            attr: "cool".to_string(),
            card_rarity_type: 4,
            rarity: "rarity_4".to_string(),
            asset_bundle_name: "chara_000001".to_string(),
            skill_id: 10,
            special_training_skill_id: None,
            special_training_power1_bonus_fixed: 100,
            special_training_power2_bonus_fixed: 100,
            special_training_power3_bonus_fixed: 100,
            support_unit: None,
            max_level: Some(60),
            max_skill_level: Some(4),
            max_master_rank: Some(5),
        };

        assert_eq!(
            skill_states_for_card(
                DefaultImage::Original,
                is_after_training(&user_card.special_training_status),
                &master,
                &BuildParams::default(),
            ),
            ([SkillState::AfterTraining, SkillState::AfterTraining], 1)
        );
    }

    #[test]
    fn handler_non_bfes_before_after_skill_states_collapse_to_best_skill() {
        let before = SkillResult {
            skill_min: 120,
            skill_max: 120,
            full: crate::types::SkillInfo {
                skill_id: 10,
                is_after_training: false,
                base_score_up: 120.0,
                ..crate::types::SkillInfo::default()
            },
            ..SkillResult::default()
        };
        let after = SkillResult {
            full: crate::types::SkillInfo {
                skill_id: 11,
                is_after_training: true,
                ..before.full
            },
            ..before.clone()
        };

        let mut collapsed = [
            Some((SkillState::AfterTraining, after)),
            Some((SkillState::BeforeTraining, before)),
        ];
        assert!(!collapse_non_bfes_skill_states(&mut collapsed, 2));
        let collapsed = collapsed.into_iter().flatten().collect::<Vec<_>>();

        assert_eq!(collapsed.len(), 1);
        assert!(matches!(collapsed[0].0, SkillState::BeforeTraining));
    }

    #[test]
    fn handler_build_card_pool_end_to_end_minimal() {
        let cards = [
            MasterCard {
                id: 1,
                character_id: 1,
                attr: "cool".to_string(),
                card_rarity_type: 4,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: 10,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
            MasterCard {
                id: 2,
                character_id: 2,
                attr: "cute".to_string(),
                card_rarity_type: 4,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: 11,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
            MasterCard {
                id: 3,
                character_id: 3,
                attr: "happy".to_string(),
                card_rarity_type: 4,
                rarity: "".to_string(),
                asset_bundle_name: "chara_000001".to_string(),
                skill_id: 12,
                special_training_skill_id: None,
                special_training_power1_bonus_fixed: 0,
                special_training_power2_bonus_fixed: 0,
                special_training_power3_bonus_fixed: 0,
                support_unit: None,
                max_level: Some(60),
                max_skill_level: Some(4),
                max_master_rank: Some(5),
            },
        ];
        let params_table = [
            types::CardParameter {
                card_id: 1,
                level: 1,
                param1: 100,
                param2: 100,
                param3: 100,
            },
            types::CardParameter {
                card_id: 2,
                level: 1,
                param1: 110,
                param2: 110,
                param3: 110,
            },
            types::CardParameter {
                card_id: 3,
                level: 1,
                param1: 120,
                param2: 120,
                param3: 120,
            },
        ];
        let rarities = [types::CardRarity {
            card_rarity_type: 4,
            max_level: 60,
            normal_max_level: 50,
            max_skill_level: 4,
        }];
        let skills = [
            types::Skill {
                id: 10,
                level: 1,
                is_after_training: false,
            },
            types::Skill {
                id: 11,
                level: 1,
                is_after_training: false,
            },
            types::Skill {
                id: 12,
                level: 1,
                is_after_training: false,
            },
        ];
        let effects = [
            types::SkillEffect {
                skill_id: 10,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 100,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 11,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 110,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
            types::SkillEffect {
                skill_id: 12,
                skill_level: 1,
                effect_type: "score_up".to_string(),
                value: 120,
                additional_value: None,
                unit_member_count: None,
                unit: None,
                activate_character_rank: None,
            },
        ];
        let units = [
            types::GameCharacterUnit {
                game_character_id: 1,
                unit: "idol".to_string(),
            },
            types::GameCharacterUnit {
                game_character_id: 2,
                unit: "street".to_string(),
            },
            types::GameCharacterUnit {
                game_character_id: 3,
                unit: "themepark".to_string(),
            },
        ];
        let music = [types::MusicMeta {
            music_id: 99,
            difficulty: "master".to_string(),
            event_rate_solo: 100,
            event_rate_multi: 110,
            event_rate_auto: 90,
            base_score: 1.0,
            base_score_auto: 1.0,
            fever_score: 0.0,
            solo_skill_scores: [0.0; 6],
            multi_skill_scores: [0.0; 6],
            auto_skill_scores: [0.0; 6],
            music_time: 100.0,
            tap_count: 500,
        }];
        let game = GameData {
            cards: &cards,
            card_parameters: &params_table,
            card_rarities: &rarities,
            card_episodes: &[],
            master_lessons: &[],
            skills: &skills,
            skill_effects: &effects,
            area_item_levels: &[],
            game_character_units: &units,
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            mysekai_gates: &[],
            mysekai_gate_levels: &[],
            events: &[],
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            world_blooms: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &[],
            event_skill_score_up_limits: &[],
            music_metas: &music,
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        };
        let user = UserProfile {
            user_cards: vec![
                sample_user_card(1),
                sample_user_card(2),
                sample_user_card(3),
            ],
            ..UserProfile::default()
        };
        let params = BuildParams {
            music_id: Some(99),
            live_type: LiveType::Solo,
            target: ScoreTarget::Score,
            skill_reference_strategy: SkillReferenceStrategy::Average,
            ..BuildParams::default()
        };

        let (pool, ctx, details) = build_card_pool_with_details(&user, &game, &params).unwrap();
        let shared_indexes = PreparedGameIndexes::new(&game);
        let prepared = PreparedGameData::with_indexes(game, &shared_indexes);
        let (prepared_pool, prepared_ctx, prepared_details) =
            build_card_pool_with_details_prepared(&user, &prepared, &params).unwrap();
        let prepared_build = PreparedPoolBuild::new(&user, &prepared, &params).unwrap();
        let (fully_prepared_pool, fully_prepared_ctx, fully_prepared_details) =
            build_card_pool_with_details_fully_prepared(&prepared, &prepared_build).unwrap();
        assert_eq!(prepared_pool.count(), pool.count());
        assert_eq!(prepared_ctx, ctx);
        assert_eq!(prepared_details, details);
        assert_eq!(fully_prepared_pool.count(), pool.count());
        assert_eq!(fully_prepared_ctx, ctx);
        assert_eq!(fully_prepared_details, details);
        assert_eq!(pool.count(), 3);
        assert_eq!(details.len(), pool.count());
        assert!(details
            .iter()
            .enumerate()
            .all(|(index, detail)| detail.game_card_id
                == pool.game_id(crate::pool::CardIdx::new(index as u16))));
        assert_eq!(ctx.music_rate_pct, 100);
        assert_eq!(ctx.target, ScoreTarget::Score);
        assert_eq!(ctx.leader_honor_bonus.len(), 3);
    }

    fn make_card(
        game_card_id: i32,
        character_id: u8,
        power_max: i32,
        skill_max: u8,
    ) -> CardIntermediate {
        CardIntermediate {
            game_card_id,
            card_rarity_type: 4,
            character_id,
            attr: (character_id % 5),
            unit_mask_raw: 1u8 << (character_id % 6),
            default_image: crate::types::DefaultImage::Original,
            after_training: false,
            skill_state_controls_image: false,
            master_rank: 0,
            skill_level: 1,
            power: power::PowerResult {
                power_min: power_max - 10,
                power_max,
                ..power::PowerResult::default()
            },
            skill: skill::SkillResult {
                slot: SkillSlot::default(),
                unit_count: None,
                diff: None,
                ref_skill: None,
                skill_min: 1,
                skill_max,
                full: crate::types::SkillInfo::default(),
            },
            event_bonus: EventBonusExact::from_whole(0, 0),
            has_char_bonus: false,
            has_attr_bonus: false,
            leader_honor_bonus: 0,
            leader_limit_bonus: 0,
            ep_sort_key: power_max as i64,
        }
    }

    #[test]
    fn handler_target_trim_power_keeps_top_per_character() {
        // 530 卡：26 角色各 ~20 张，power_max 从高到低排列
        let mut cards = Vec::new();
        for ch in 0..26u8 {
            for i in 0..21i32 {
                cards.push(make_card(
                    (ch as i32) * 100 + i,
                    ch,
                    30000 - i * 100, // 第一张最高
                    20,
                ));
            }
        }
        // 530 卡 < 512 容量
        assert!(cards.len() > 512);

        let params = BuildParams {
            target: ScoreTarget::Power,
            ..BuildParams::default()
        };
        target_per_character_trim(&mut cards, &params);

        // 每角色最多 10 张
        let mut chars_seen = [0u8; 27];
        for card in &cards {
            let ch = card.character_id as usize;
            chars_seen[ch] += 1;
        }
        for (ch, &count) in chars_seen.iter().enumerate() {
            assert!(
                count <= GENERAL_PER_CHAR_KEEP as u8,
                "角色 {ch} 有 {count} 张卡，超过上限"
            );
        }
        // 总计 ≤ 260，远小于 512
        assert!(cards.len() <= 260, "裁剪后仍有 {} 张", cards.len());

        // 每角色的最高 power 卡应被保留
        for ch in 0..26u8 {
            let best_id = (ch as i32) * 100; // 该角色第一张（最高 power）
            assert!(
                cards.iter().any(|c| c.game_card_id == best_id),
                "角色 {ch} 最高 power 卡 {best_id} 未被保留"
            );
        }
    }

    #[test]
    fn handler_target_trim_skill_keeps_top_per_character() {
        let mut cards = Vec::new();
        for ch in 0..26u8 {
            for i in 0..21i32 {
                cards.push(make_card(
                    (ch as i32) * 100 + i,
                    ch,
                    30000,
                    100 - i as u8, // 第一张最高 skill
                ));
            }
        }
        assert!(cards.len() > 512);

        let params = BuildParams {
            target: ScoreTarget::Skill,
            ..BuildParams::default()
        };
        target_per_character_trim(&mut cards, &params);

        let mut chars_seen = [0u8; 27];
        for card in &cards {
            let ch = card.character_id as usize;
            chars_seen[ch] += 1;
        }
        for (ch, &count) in chars_seen.iter().enumerate() {
            assert!(
                count <= GENERAL_PER_CHAR_KEEP as u8,
                "角色 {ch} 有 {count} 张卡，超过上限"
            );
        }
        assert!(cards.len() <= 260, "裁剪后仍有 {} 张", cards.len());

        for ch in 0..26u8 {
            let best_id = (ch as i32) * 100;
            assert!(
                cards.iter().any(|c| c.game_card_id == best_id),
                "角色 {ch} 最高 skill 卡 {best_id} 未被保留"
            );
        }
    }

    #[test]
    fn handler_target_trim_preserves_fixed_cards_and_characters() {
        let mut cards = Vec::new();
        for ch in 0..26u8 {
            for i in 0..21i32 {
                cards.push(make_card((ch as i32) * 100 + i, ch, 30000 - i * 100, 20));
            }
        }

        // fixed_card: 角色 5 的第 20 张卡（power 较低）
        let fixed_card_id: i32 = 5 * 100 + 20;
        let params = BuildParams {
            target: ScoreTarget::Power,
            fixed_cards: vec![fixed_card_id],
            ..BuildParams::default()
        };
        target_per_character_trim(&mut cards, &params);

        assert!(
            cards.iter().any(|c| c.game_card_id == fixed_card_id),
            "fixed_card 未被保留"
        );

        // 再测 fixed_characters
        let mut cards2 = Vec::new();
        for ch in 0..26u8 {
            for i in 0..21i32 {
                cards2.push(make_card((ch as i32) * 100 + i, ch, 30000 - i * 100, 20));
            }
        }
        let params2 = BuildParams {
            target: ScoreTarget::Power,
            fixed_characters: vec![3],
            ..BuildParams::default()
        };
        target_per_character_trim(&mut cards2, &params2);

        // 角色 3 的所有卡应被保留（21 张 > 10）
        let role3_count = cards2.iter().filter(|c| c.character_id == 3).count();
        assert_eq!(role3_count, 21, "fixed_character=3 的卡未全部保留");
    }

    #[test]
    fn handler_build_power_large_pool_does_not_error() {
        // 模拟大账号：26 角色各 25 张卡 = 650 张 > 512
        let mut master_cards = Vec::new();
        let mut card_params = Vec::new();
        let mut skills = Vec::new();
        let mut effects = Vec::new();
        let mut units = Vec::new();
        let mut user_cards = Vec::new();

        for ch in 1i32..=26i32 {
            for i in 0i32..25i32 {
                let card_id = ch * 100 + i;
                master_cards.push(MasterCard {
                    id: card_id,
                    character_id: ch,
                    attr: "cool".to_string(),
                    card_rarity_type: 4,
                    rarity: "".to_string(),
                    asset_bundle_name: "chara_000001".to_string(),
                    skill_id: card_id * 10,
                    special_training_skill_id: None,
                    special_training_power1_bonus_fixed: 0,
                    special_training_power2_bonus_fixed: 0,
                    special_training_power3_bonus_fixed: 0,
                    support_unit: None,
                    max_level: Some(60),
                    max_skill_level: Some(4),
                    max_master_rank: Some(5),
                });
                card_params.push(types::CardParameter {
                    card_id,
                    level: 1,
                    param1: 100 + i,
                    param2: 100,
                    param3: 100,
                });
                skills.push(types::Skill {
                    id: card_id * 10,
                    level: 1,
                    is_after_training: false,
                });
                effects.push(types::SkillEffect {
                    skill_id: card_id * 10,
                    skill_level: 1,
                    effect_type: "score_up".to_string(),
                    value: 100,
                    additional_value: None,
                    unit_member_count: None,
                    unit: None,
                    activate_character_rank: None,
                });
                units.push(types::GameCharacterUnit {
                    game_character_id: ch,
                    unit: "idol".to_string(),
                });
                user_cards.push(sample_user_card(card_id));
            }
        }
        let rarities = [types::CardRarity {
            card_rarity_type: 4,
            max_level: 60,
            normal_max_level: 50,
            max_skill_level: 4,
        }];
        let game = GameData {
            cards: &master_cards,
            card_parameters: &card_params,
            card_rarities: &rarities,
            card_episodes: &[],
            master_lessons: &[],
            skills: &skills,
            skill_effects: &effects,
            area_item_levels: &[],
            game_character_units: &units,
            character_ranks: &[],
            card_mysekai_canvas_bonuses: &[],
            mysekai_gates: &[],
            mysekai_gate_levels: &[],
            events: &[],
            event_cards: &[],
            event_deck_bonuses: &[],
            event_card_bonus_limits: &[],
            event_honor_bonuses: &[],
            world_bloom_different_attribute_bonuses: &[],
            world_blooms: &[],
            wb_support_deck_bonuses_wl1: &[],
            wb_support_deck_bonuses_wl2: &[],
            wb_support_deck_bonuses_wl3: &[],
            world_bloom_support_deck_unit_event_limited_bonuses: &[],
            event_mysekai_fixture_performance_bonus_limits: &[],
            event_skill_score_up_limits: &[],
            music_metas: &[],
            music_difficulties: &[],
            event_rarity_bonus_rates: &[],
            honors: &[],
            bonds_honors: &[],
        };
        let user = UserProfile {
            user_cards,
            ..UserProfile::default()
        };
        let params = BuildParams {
            target: ScoreTarget::Power,
            ..BuildParams::default()
        };

        let result = build_card_pool(&user, &game, &params);
        assert!(result.is_ok(), "大卡池 Power 构建应成功，实际: {result:?}");
        let (pool, _) = result.unwrap();
        assert!(pool.count() <= 512, "池子大小应 ≤ 512");
    }
}
