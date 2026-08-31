//! 卡池构建管线。
//!
//! 输入 `(&UserProfile, &GameData, &BuildParams)`，输出 `(CardPool, SearchContext)`：
//! - `PreparedPoolBuild`：逐卡准备（活动加成、技能状态、综合力）与候选 seed；
//! - `build_search_context`：搜索上下文装配（支援 deck、荣誉加成、终章语义）；
//! - `build_card_pool*` 公开入口与 `cultivated_user_cards` 展示态卡况。

use std::borrow::Cow;
use std::collections::BTreeSet;

use crate::pool::EventBonusExact;
use crate::search::SearchContext;
use crate::types::DefaultImage;

use super::card_config::apply_card_config;
use super::event_bonus::{build_card_event_bonus, build_event_context, EventContext};
use super::filter::{
    ep_prefilter_keep, ep_prefilter_keep_with_params, general_per_character_trim, keep_card,
    per_character_trim, prepared_ep_prefilter_keep, prepared_ep_prefilter_keep_with_params,
    prepared_keep_card, prepared_post_event_unit_filter, target_per_character_trim,
    CHALLENGE_ALL_PER_CHAR_KEEP, EP_PREFILTER_MIN_POOL, FINAL_CHAPTER_PER_CHAR_KEEP,
    GENERAL_PER_CHAR_KEEP, GENERAL_TRIM_THRESHOLD, PER_CHAR_KEEP,
};
use super::gather::{sort_and_gather, CardIntermediate, FullPrecisionCard, GatheredContext};
use super::index;
use super::music::{self, build_music_params};
use super::power::{
    build_power_batch_from_fn, PowerInput, PowerResult, PreparedPowerContext,
};
use super::skill::{build_skill, is_bfes_skill_pair, SkillResult, SkillState};
use super::types::{
    self, default_image_kind, is_after_training,
};
use super::validate::validate_build_params;
use super::world_bloom::{
    build_final_chapter_support_decks_fast, build_support_deck_fast, support_seed_from_intermediate,
    SupportSeedSlim,
};
use super::{BuildError, PreparedGameData};

pub(super) fn enrich_master(master: &types::MasterCard, game: &types::GameData<'_>) -> types::MasterCard {
    let mut master = master.clone();
    if (master.max_level.is_none() || master.max_skill_level.is_none())
        && let Some(rarity) = game
            .card_rarities
            .iter()
            .find(|entry| entry.card_rarity_type == master.card_rarity_type)
        {
            master.max_level.get_or_insert(rarity.max_level);
            master.max_skill_level.get_or_insert(rarity.max_skill_level);
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

pub(super) fn merged_configs(params: &types::BuildParams) -> types::CardConfigSet {
    let mut configs = params.card_configs.clone();
    configs
        .single_card_configs
        .extend(params.single_card_configs.iter().cloned());
    configs
}

pub(super) fn normalize_user_cards(
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

pub(super) struct PreparedCardSeed<'a> {
    pub(super) master: &'a types::MasterCard,
    pub(super) user_card: Cow<'a, types::UserCard>,
    pub(super) unit_mask: u8,
    pub(super) attr: u8,
    pub(super) default_image_kind: DefaultImage,
    pub(super) default_image: DefaultImage,
    pub(super) after_training: bool,
    pub(super) event_bonus: EventBonusExact,
    pub(super) has_char_bonus: bool,
    pub(super) has_attr_bonus: bool,
    pub(super) leader_honor_bonus: u16,
    pub(super) leader_limit_bonus: u16,
}

pub(super) struct PreparedCardBuild<'a> {
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
        let event_ctx = build_event_context(game, params)?;
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
            if params.event_id == Some(crate::types::FINAL_CHAPTER_EVENT_ID)
                || params.world_bloom_finale_turn.is_some()
            {
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
            if let Some(ctx) = event_ctx.as_ref()
                && crate::types::is_world_bloom_finale_event(ctx.event_id)
                && !ctx.honor_bonuses.is_empty()
            {
                    let owned_honors: std::collections::HashSet<i32> = user
                        .user_honors
                        .iter()
                        .map(|honor| honor.honor_id)
                        .collect();
                    for entry in &ctx.honor_bonuses {
                        if let Ok(ch) = usize::try_from(entry.leader_game_character_id)
                            && ch < 27 && owned_honors.contains(&entry.honor_id) {
                                result[ch] =
                                    result[ch].wrapping_add(entry.bonus_rate.max(0) as u16);
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
            .is_some_and(|ctx| crate::types::is_world_bloom_finale_event(ctx.event_id));
        let can_prefilter_before_power = event_ctx.as_ref().is_some_and(|ctx| {
            ctx.support_deck_count == 0
                && !crate::types::is_world_bloom_finale_event(ctx.event_id)
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

pub(super) fn normalize_boost_rate_pct(boost: Option<i32>) -> u32 {
    match boost {
        Some(value) if value <= 0 => 100,
        Some(value) if value <= 5 => (value * 500) as u32,
        Some(value) if value <= 10 => (2500 + (value - 5) * 200) as u32,
        _ => 100,
    }
}

pub(super) fn validate_fixed_constraints(
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

pub(super) fn skill_states_for_card(
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

pub(super) fn card_can_special_train(master: &types::MasterCard) -> bool {
    master.special_training_skill_id.is_some()
        || master.special_training_power1_bonus_fixed > 0
        || master.special_training_power2_bonus_fixed > 0
        || master.special_training_power3_bonus_fixed > 0
        || matches!(master.card_rarity_type, 3 | 4)
}

pub(super) fn collapse_non_bfes_skill_states(
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
pub(super) fn build_search_context(
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
            .is_some_and(|ctx| crate::types::is_world_bloom_finale_event(ctx.event_id))
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
        is_final_chapter: event_ctx
            .is_some_and(|ctx| crate::types::is_world_bloom_finale_event(ctx.event_id)),
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

pub(super) fn compute_honor_bonus(user: &types::UserProfile, indexes: &index::PoolIndexes) -> u32 {
    user.user_honors
        .iter()
        .map(|honor| indexes.honor_bonus(honor.honor_id, honor.level))
        .sum()
}

pub(super) fn resolve_fixture_bonus_limit(
    game: &types::GameData<'_>,
    event_ctx: Option<&EventContext>,
) -> Option<i32> {
    let event_id = event_ctx?.event_id;
    game.event_mysekai_fixture_performance_bonus_limits
        .iter()
        .find(|entry| entry.event_id == event_id)
        .map(|entry| entry.bonus_rate_limit)
        // 终章（legacy 180 与模拟 WL3 终章）固定 20。
        .or_else(|| {
            crate::types::is_world_bloom_finale_event(event_id).then_some(20)
        })
}
pub(super) fn build_card_pool_fully_prepared_internal(
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
        .is_some_and(|ctx| {
            ctx.support_deck_count > 0 || crate::types::is_world_bloom_finale_event(ctx.event_id)
        });
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

    if params.filter_other_unit
        && let Some(unit) = event_ctx.and_then(|ctx| ctx.filter_unit)
            && let Some(unit_index) = types::unit_to_pool_index(unit) {
                let wanted = 1u8 << unit_index;
                let piapro = types::unit_to_pool_index(crate::types::Unit::Piapro)
                    .map(|index| 1u8 << index)
                    .unwrap_or(0);
                cards.retain(|card| {
                    card.unit_mask_raw & wanted != 0 || card.unit_mask_raw == piapro
                });
            }

    let is_world_bloom =
        event_ctx.is_some_and(|ctx| matches!(ctx.event_type, crate::types::EventType::WorldBloom));
    let is_final_chapter = event_ctx
        .is_some_and(|ctx| crate::types::is_world_bloom_finale_event(ctx.event_id));
    if build.ep_prefilter_applied {
        per_character_trim(&mut cards, params, PER_CHAR_KEEP, 0);
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
            // WL turn-3 的 336k cap 与异色加成让高练度低加成卡同样可能进最优解，
            // 额外保留综合力专家。
            let (keep, power_keep) = if is_final_chapter {
                (FINAL_CHAPTER_PER_CHAR_KEEP - 8, 8)
            } else {
                (PER_CHAR_KEEP, 8)
            };
            per_character_trim(&mut cards, params, keep, power_keep);
        } else {
            per_character_trim(&mut cards, params, PER_CHAR_KEEP, 0);
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
