//! Auxiliary calculations that do not participate in the DFS search path.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::handler::{
    build_card_pool_with_details, BuildParams, MusicMeta, UserAreaItem, UserProfile,
};
use crate::search::resolve_power_for_cards;
use crate::{EventType, LiveSkillOrder, LiveType, ScoreTarget, DECK_SIZE};

/// Additional masterdata used only by auxiliary calculations.
#[derive(Debug, Clone, Default)]
pub struct AuxiliaryData {
    areas: Vec<Area>,
    area_items: Vec<AreaItem>,
    shop_items: Vec<ShopItem>,
    ingame_notes: Vec<IngameNote>,
    ingame_combos: Vec<IngameCombo>,
}

impl AuxiliaryData {
    /// Parse auxiliary tables when they are present. Missing tables remain empty
    /// and are reported only by the operation that requires them.
    pub fn from_strings(tables: &BTreeMap<String, String>) -> Result<Self, String> {
        Ok(Self {
            areas: parse_optional(tables, "areas.json")?,
            area_items: parse_optional(tables, "areaItems.json")?,
            shop_items: parse_optional(tables, "shopItems.json")?,
            ingame_notes: parse_optional(tables, "ingameNotes.json")?,
            ingame_combos: parse_optional(tables, "ingameCombos.json")?,
        })
    }

    /// Recommend the next useful area-item upgrades for a fixed deck.
    pub fn recommend_area_items(
        &self,
        user: &UserProfile,
        game: &crate::handler::GameData<'_>,
        card_ids: &[i32],
    ) -> Result<Vec<AreaItemRecommendation>, String> {
        if !(1..=DECK_SIZE).contains(&card_ids.len()) {
            return Err("card_ids must contain 1 to 5 cards".to_string());
        }
        if self.areas.is_empty() {
            return Err("areas masterdata is not loaded".to_string());
        }
        if self.area_items.is_empty() {
            return Err("areaItems masterdata is not loaded".to_string());
        }
        if self.shop_items.is_empty() {
            return Err("shopItems masterdata is not loaded".to_string());
        }

        let current_power = fixed_deck_power(user, game, card_ids)?;
        let current_levels = user
            .user_area_items
            .iter()
            .map(|item| (item.area_item_id, item.level))
            .collect::<HashMap<_, _>>();
        let mut result = Vec::new();

        for area_item in &self.area_items {
            let max_level = game
                .area_item_levels
                .iter()
                .filter(|level| level.area_item_id == area_item.id)
                .map(|level| level.level)
                .max()
                .ok_or_else(|| {
                    format!(
                        "area item levels not found for area_item_id={}",
                        area_item.id
                    )
                })?;
            let current_level = current_levels.get(&area_item.id).copied();
            let next_level = current_level.map_or(1, |level| (level + 1).min(max_level));
            if current_level.is_some_and(|level| next_level <= level) {
                continue;
            }
            if !game
                .area_item_levels
                .iter()
                .any(|level| level.area_item_id == area_item.id && level.level == next_level)
            {
                return Err(format!(
                    "area item level not found for area_item_id={} level={next_level}",
                    area_item.id
                ));
            }

            let mut upgraded = user.clone();
            if let Some(item) = upgraded
                .user_area_items
                .iter_mut()
                .find(|item| item.area_item_id == area_item.id)
            {
                item.level = next_level;
            } else {
                upgraded.user_area_items.push(UserAreaItem {
                    area_item_id: area_item.id,
                    level: next_level,
                });
            }
            let power = fixed_deck_power(&upgraded, game, card_ids)? - current_power;
            if power <= 0 {
                continue;
            }

            let area = self
                .areas
                .iter()
                .find(|area| area.id == area_item.area_id)
                .ok_or_else(|| format!("area not found for area_id={}", area_item.area_id))?;
            let shop_item_id = if next_level <= 10 {
                1000 + (area_item.id - 1) * 10 + next_level
            } else {
                1540 + (area_item.id - 1) * 5 + next_level
            };
            let shop_item = self
                .shop_items
                .iter()
                .find(|item| item.id == shop_item_id)
                .ok_or_else(|| {
                    format!(
                        "shop item not found for area_item_id={} level={next_level}",
                        area_item.id
                    )
                })?;
            let cost = AreaItemCost {
                coin: find_cost(shop_item, "coin", 0),
                seed: find_cost(shop_item, "material", 17),
                szk: find_cost(shop_item, "material", 57),
            };
            result.push(AreaItemRecommendation {
                area_id: area.id,
                area_type: area.area_type.clone(),
                area_view_type: area.view_type.clone(),
                area_item_id: area_item.id,
                next_level,
                shop_item_id,
                cost,
                power,
                power_per_coin: if cost.coin > 0 {
                    power as f64 / cost.coin as f64
                } else {
                    0.0
                },
            });
        }

        result.sort_by(|left, right| {
            right
                .power_per_coin
                .total_cmp(&left.power_per_coin)
                .then_with(|| right.power.cmp(&left.power))
                .then_with(|| left.area_item_id.cmp(&right.area_item_id))
        });
        Ok(result)
    }

    /// Calculate note-level live score details.
    pub fn calculate_exact_live(
        &self,
        power: i32,
        skills: &[f64],
        live_type: LiveType,
        music_score_json: &str,
        multi_sum_power: i32,
        fever_music_score_json: Option<&str>,
    ) -> Result<ExactLiveDetail, String> {
        if self.ingame_notes.is_empty() {
            return Err("ingameNotes masterdata is not loaded".to_string());
        }
        if self.ingame_combos.is_empty() {
            return Err("ingameCombos masterdata is not loaded".to_string());
        }
        if matches!(live_type, LiveType::Mysekai) {
            return Err("invalid live type: mysekai".to_string());
        }
        let score: MusicScore = serde_json::from_str(music_score_json)
            .map_err(|error| format!("invalid music score JSON: {error}"))?;
        if score.notes.is_empty() {
            return Err("musicScore.notes must not be empty".to_string());
        }
        let fever_score = fever_music_score_json
            .filter(|value| !value.is_empty())
            .map(|value| {
                serde_json::from_str::<MusicScore>(value)
                    .map_err(|error| format!("invalid fever music score JSON: {error}"))
            })
            .transpose()?;

        let mut effects = skill_effects(skills, &score.skills);
        if is_multi(live_type) {
            effects.push(fever_effect(fever_score.as_ref().unwrap_or(&score)));
        }

        let note_coefficients = score
            .notes
            .iter()
            .map(|note| {
                self.ingame_notes
                    .iter()
                    .find(|item| item.id == note.note_type)
                    .map(|item| item.score_coefficient)
                    .ok_or_else(|| format!("ingame note not found for type={}", note.note_type))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let coefficient_total = note_coefficients.iter().sum::<f64>();
        if coefficient_total <= 0.0 {
            return Err("musicScore note coefficient total must be positive".to_string());
        }

        let mut detail = ExactLiveDetail::default();
        detail.notes.reserve(score.notes.len());
        for (index, note) in score.notes.iter().enumerate() {
            let combo = index as i32 + 1;
            let combo_coefficient = self
                .ingame_combos
                .iter()
                .find(|item| item.from_count <= combo && combo <= item.to_count)
                .map(|item| item.score_coefficient)
                .ok_or_else(|| format!("ingame combo not found for combo={combo}"))?;
            let effect_bonuses = effects
                .iter()
                .filter(|effect| effect.start_time <= note.time && note.time <= effect.end_time)
                .map(|effect| effect.effect)
                .collect::<Vec<_>>();
            let effect_coefficient = effect_bonuses
                .iter()
                .fold(1.0, |value, bonus| value * bonus / 100.0);
            let note_score = note_coefficients[index]
                * combo_coefficient
                * effect_coefficient
                * power as f64
                * 4.0
                / coefficient_total;
            detail.notes.push(ExactLiveNoteDetail {
                note_coefficient: note_coefficients[index],
                combo_coefficient,
                judge_coefficient: 1.0,
                effect_bonuses,
                score: note_score,
            });
            detail.total += note_score;
        }
        if is_multi(live_type) {
            let power_sum = if multi_sum_power > 0 {
                multi_sum_power
            } else {
                power * DECK_SIZE as i32
            };
            detail.active_bonus = DECK_SIZE as f64 * 0.015 * power_sum as f64;
            detail.total += detail.active_bonus;
        }
        Ok(detail)
    }
}

/// Minimal deck snapshot required to score all loaded music metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MusicDeck {
    pub total_power: i32,
    pub event_bonus_rate: f64,
    pub support_deck_bonus_rate: f64,
    pub cards: Vec<MusicDeckCard>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MusicDeckCard {
    pub skill_score_up: f64,
    pub skill_life_recovery: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MusicRecommendOptions {
    pub live_type: LiveType,
    pub event_type: EventType,
    pub skill_order: LiveSkillOrder,
    pub specific_skill_order: Option<Vec<usize>>,
    pub multi_teammate_score_up: Option<i32>,
    pub multi_teammate_power: Option<i32>,
}

/// One scored music/difficulty row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MusicRecommendation {
    pub music_id: i32,
    pub difficulty: String,
    pub live_score: i32,
    pub event_point: Option<i32>,
}

/// Score every loaded music row for an already materialized deck.
pub fn recommend_music(
    music_metas: &[MusicMeta],
    deck: &MusicDeck,
    options: &MusicRecommendOptions,
) -> Result<Vec<MusicRecommendation>, String> {
    if music_metas.is_empty() {
        return Err("music metas are not loaded".to_string());
    }
    if deck.cards.is_empty() || deck.cards.len() > DECK_SIZE {
        return Err("deck.cards must contain 1 to 5 cards".to_string());
    }
    if options.multi_teammate_score_up.is_some() && !is_multi(options.live_type) {
        return Err("multi_live_teammate_score_up is only valid for multi live".to_string());
    }
    if options.multi_teammate_power.is_some() && !is_multi(options.live_type) {
        return Err("multi_live_teammate_power is only valid for multi live".to_string());
    }
    if matches!(options.skill_order, LiveSkillOrder::Specific)
        && options.specific_skill_order.is_none()
    {
        return Err(
            "specific_skill_order is required when skill_order_choose_strategy is specific"
                .to_string(),
        );
    }

    let mut result = music_metas
        .iter()
        .map(|music| {
            let live_score = music_live_score(deck, music, options)?;
            let event_point = Some(event_point(
                options.live_type,
                options.event_type,
                live_score,
                music_event_rate(music, options.live_type),
                deck.event_bonus_rate + deck.support_deck_bonus_rate,
            )?);
            Ok(MusicRecommendation {
                music_id: music.music_id,
                difficulty: music.difficulty.clone(),
                live_score,
                event_point,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    result.sort_by(|left, right| {
        right
            .event_point
            .unwrap_or(-1)
            .cmp(&left.event_point.unwrap_or(-1))
            .then_with(|| right.live_score.cmp(&left.live_score))
            .then_with(|| left.music_id.cmp(&right.music_id))
            .then_with(|| {
                difficulty_order(&left.difficulty).cmp(&difficulty_order(&right.difficulty))
            })
    });
    Ok(result)
}

fn fixed_deck_power(
    user: &UserProfile,
    game: &crate::handler::GameData<'_>,
    card_ids: &[i32],
) -> Result<i32, String> {
    for card_id in card_ids {
        if !user.user_cards.iter().any(|card| card.card_id == *card_id) {
            return Err(format!("User card not found for cardId={card_id}"));
        }
    }
    let mut params = BuildParams {
        target: ScoreTarget::Power,
        ..BuildParams::default()
    };
    params.fixed_cards = card_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let (pool, context, _) =
        build_card_pool_with_details(user, game, &params).map_err(|error| error.to_string())?;
    let mut cards = Vec::with_capacity(card_ids.len());
    for card_id in card_ids {
        let card = pool
            .indices()
            .find(|card| pool.game_id(*card) as i32 == *card_id)
            .ok_or_else(|| format!("card not found for card_id={card_id}"))?;
        cards.push(card);
    }
    let power = resolve_power_for_cards(&pool, &cards)
        .saturating_add(context.honor_bonus)
        .min(i32::MAX as u32);
    Ok(power as i32)
}

fn music_live_score(
    deck: &MusicDeck,
    music: &MusicMeta,
    options: &MusicRecommendOptions,
) -> Result<i32, String> {
    let mut skills = if is_multi(options.live_type) {
        let self_score = deck
            .cards
            .iter()
            .enumerate()
            .map(|(index, card)| card.skill_score_up * if index == 0 { 1.0 } else { 0.2 })
            .sum::<f64>();
        let self_life = deck.cards[0].skill_life_recovery;
        let teammate_score = options
            .multi_teammate_score_up
            .map(f64::from)
            .unwrap_or(self_score);
        let mut values = vec![(self_score, self_life)];
        values.extend(std::iter::repeat_n((teammate_score, 0.0), DECK_SIZE - 1));
        values.push((self_score, self_life));
        values
    } else {
        let mut values = deck
            .cards
            .iter()
            .map(|card| (card.skill_score_up, card.skill_life_recovery))
            .collect::<Vec<_>>();
        values.push((
            deck.cards[0].skill_score_up,
            deck.cards[0].skill_life_recovery,
        ));
        values
    };
    let card_count = deck.cards.len();

    match options.skill_order {
        LiveSkillOrder::Specific => {
            let order = options.specific_skill_order.as_ref().ok_or_else(|| {
                "specific_skill_order is required when skill_order_choose_strategy is specific"
                    .to_string()
            })?;
            if order.len() != skills.len() - 1 {
                return Err("specific_skill_order size does not match skills size".to_string());
            }
            let returning = *skills.last().unwrap_or(&(0.0, 0.0));
            let mut ordered = Vec::with_capacity(skills.len());
            for index in order {
                ordered.push(
                    *skills.get(*index).ok_or_else(|| {
                        format!("specific_skill_order index out of range: {index}")
                    })?,
                );
            }
            ordered.push(returning);
            skills = ordered;
        }
        LiveSkillOrder::Best => {
            skills[..card_count].sort_by(|left, right| left.0.total_cmp(&right.0))
        }
        LiveSkillOrder::Worst => {
            skills[..card_count].sort_by(|left, right| right.0.total_cmp(&left.0))
        }
        LiveSkillOrder::Average => {
            let average = skills[..card_count]
                .iter()
                .map(|skill| skill.0)
                .sum::<f64>()
                / card_count as f64;
            for skill in &mut skills[..card_count] {
                skill.0 = average;
            }
        }
    }
    if card_count < DECK_SIZE {
        let insertion = skills.len() - 1;
        skills.splice(
            insertion..insertion,
            std::iter::repeat_n((0.0, 0.0), DECK_SIZE - card_count),
        );
    }

    let mut rates = match options.live_type {
        LiveType::Auto | LiveType::ChallengeAuto => music.auto_skill_scores,
        LiveType::Multi | LiveType::Cheerful => music.multi_skill_scores,
        _ => music.solo_skill_scores,
    };
    if matches!(
        options.skill_order,
        LiveSkillOrder::Best | LiveSkillOrder::Worst
    ) {
        rates[..card_count].sort_by(f64::total_cmp);
    }
    let base = match options.live_type {
        LiveType::Auto | LiveType::ChallengeAuto => music.base_score_auto,
        LiveType::Multi | LiveType::Cheerful => music.base_score + music.fever_score * 0.5,
        _ => music.base_score,
    };
    let rate = skills
        .iter()
        .zip(rates)
        .fold(base, |total, (skill, coefficient)| {
            total + skill.0 * coefficient / 100.0
        });
    let power_sum = options
        .multi_teammate_power
        .map_or(DECK_SIZE as i32 * deck.total_power, |teammate| {
            deck.total_power + teammate * (DECK_SIZE as i32 - 1)
        });
    let active_bonus = if is_multi(options.live_type) {
        DECK_SIZE as f64 * 0.015 * power_sum as f64
    } else {
        0.0
    };
    Ok((rate * deck.total_power as f64 * 4.0 + active_bonus) as i32)
}

fn event_point(
    live_type: LiveType,
    event_type: EventType,
    live_score: i32,
    music_rate: i32,
    deck_bonus: f64,
) -> Result<i32, String> {
    if matches!(live_type, LiveType::Challenge | LiveType::ChallengeAuto) {
        return Ok((100 + live_score / 20_000) * 120);
    }
    let music_rate = music_rate as f64 / 100.0;
    let deck_rate = deck_bonus / 100.0 + 1.0;
    if !is_multi(live_type) {
        return Ok(((100 + live_score / 20_000) as f64 * music_rate * deck_rate) as i32);
    }
    let base = 110 + (live_score as f64 / 17_000.0) as i32 + (live_score * 4 / 340_000).min(13);
    if matches!(live_type, LiveType::Multi) {
        if matches!(event_type, EventType::CheerfulCarnival) {
            return Err("multi live is not playable in cheerful event".to_string());
        }
        return Ok((base as f64 * music_rate * deck_rate) as i32);
    }
    if !matches!(event_type, EventType::CheerfulCarnival) {
        return Err("cheerful live is only playable in cheerful event".to_string());
    }
    let life_rate = 1.15 + (1_000.0_f64 / 5_000.0).clamp(0.1, 0.2);
    Ok(((base as f64 * music_rate * deck_rate) as i32 as f64 * life_rate) as i32)
}

fn music_event_rate(music: &MusicMeta, live_type: LiveType) -> i32 {
    match live_type {
        LiveType::Auto | LiveType::ChallengeAuto => music.event_rate_auto,
        LiveType::Multi | LiveType::Cheerful => music.event_rate_multi,
        _ => music.event_rate_solo,
    }
}

fn is_multi(live_type: LiveType) -> bool {
    matches!(live_type, LiveType::Multi | LiveType::Cheerful)
}

fn difficulty_order(value: &str) -> usize {
    match value.trim().to_ascii_lowercase().as_str() {
        "easy" => 0,
        "normal" => 1,
        "hard" => 2,
        "expert" => 3,
        "master" => 4,
        "append" => 5,
        _ => usize::MAX,
    }
}

fn parse_optional<T>(tables: &BTreeMap<String, String>, name: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de> + Default,
{
    tables.get(name).map_or_else(
        || Ok(T::default()),
        |text| {
            serde_json::from_str(text).map_err(|error| format!("failed to parse {name}: {error}"))
        },
    )
}

fn find_cost(item: &ShopItem, resource_type: &str, resource_id: i32) -> i32 {
    item.costs
        .iter()
        .find(|cost| {
            cost.cost.resource_type == resource_type && cost.cost.resource_id == resource_id
        })
        .map(|cost| cost.cost.quantity)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Area {
    id: i32,
    #[serde(default)]
    area_type: String,
    #[serde(default)]
    view_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AreaItem {
    id: i32,
    area_id: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct ShopItem {
    id: i32,
    #[serde(default)]
    costs: Vec<ShopItemCost>,
}

#[derive(Debug, Clone, Deserialize)]
struct ShopItemCost {
    #[serde(default)]
    cost: CommonResource,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommonResource {
    #[serde(default)]
    resource_id: i32,
    #[serde(default)]
    resource_type: String,
    #[serde(default)]
    quantity: i32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngameNote {
    id: i32,
    score_coefficient: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngameCombo {
    #[serde(default)]
    from_count: i32,
    #[serde(default)]
    to_count: i32,
    #[serde(default)]
    score_coefficient: f64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct MusicNoteBase {
    #[serde(default)]
    time: f64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct MusicNote {
    #[serde(default)]
    time: f64,
    #[serde(default, rename = "type")]
    note_type: i32,
    #[allow(dead_code)]
    #[serde(default, rename = "longId")]
    long_id: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct MusicScore {
    #[serde(default)]
    notes: Vec<MusicNote>,
    #[serde(default)]
    skills: Vec<MusicNoteBase>,
    #[serde(default)]
    fevers: Vec<MusicNoteBase>,
}

#[derive(Debug, Clone, Copy, Default)]
struct EffectDetail {
    start_time: f64,
    end_time: f64,
    effect: f64,
}

fn skill_effects(skills: &[f64], timings: &[MusicNoteBase]) -> Vec<EffectDetail> {
    skills
        .iter()
        .zip(timings)
        .map(|(effect, timing)| EffectDetail {
            start_time: timing.time,
            end_time: timing.time + 5.0,
            effect: *effect,
        })
        .collect()
}

fn fever_effect(score: &MusicScore) -> EffectDetail {
    if score.fevers.is_empty() || score.notes.is_empty() {
        return EffectDetail::default();
    }
    let start_time = score
        .fevers
        .iter()
        .map(|fever| fever.time)
        .fold(0.0, f64::max);
    let notes = score
        .notes
        .iter()
        .filter(|note| note.time >= start_time)
        .collect::<Vec<_>>();
    if notes.is_empty() {
        return EffectDetail::default();
    }
    let count = notes.len().min(score.notes.len() / 10).max(1);
    EffectDetail {
        start_time,
        end_time: notes[count - 1].time,
        effect: 50.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AreaItemCost {
    pub coin: i32,
    pub seed: i32,
    pub szk: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AreaItemRecommendation {
    pub area_id: i32,
    pub area_type: String,
    pub area_view_type: String,
    pub area_item_id: i32,
    pub next_level: i32,
    pub shop_item_id: i32,
    pub cost: AreaItemCost,
    pub power: i32,
    pub power_per_coin: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ExactLiveDetail {
    pub total: f64,
    pub active_bonus: f64,
    pub notes: Vec<ExactLiveNoteDetail>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExactLiveNoteDetail {
    pub note_coefficient: f64,
    pub combo_coefficient: f64,
    pub judge_coefficient: f64,
    pub effect_bonuses: Vec<f64>,
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auxiliary_tables() -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "ingameNotes.json".to_string(),
                r#"[{"id":1,"scoreCoefficient":1.0},{"id":2,"scoreCoefficient":2.0}]"#
                    .to_string(),
            ),
            (
                "ingameCombos.json".to_string(),
                r#"[{"fromCount":1,"toCount":1,"scoreCoefficient":1.0},{"fromCount":2,"toCount":99,"scoreCoefficient":1.1}]"#
                    .to_string(),
            ),
        ])
    }

    #[test]
    fn exact_live_reports_per_note_details_and_multi_active_bonus() {
        let data = AuxiliaryData::from_strings(&auxiliary_tables()).unwrap();
        let score = r#"{"notes":[{"time":1.0,"type":1},{"time":7.0,"type":2}],"skills":[{"time":1.0}],"fevers":[]}"#;
        let detail = data
            .calculate_exact_live(10_000, &[100.0], LiveType::Multi, score, 60_000, None)
            .unwrap();

        assert_eq!(detail.notes.len(), 2);
        assert_eq!(detail.notes[0].effect_bonuses, vec![100.0]);
        assert!(detail.notes[1].effect_bonuses.is_empty());
        assert_eq!(detail.active_bonus, 4_500.0);
        let expected =
            1.0 * 1.0 * 10_000.0 * 4.0 / 3.0 + 2.0 * 1.1 * 10_000.0 * 4.0 / 3.0 + 4_500.0;
        assert!((detail.total - expected).abs() < 1e-9);
    }

    #[test]
    fn music_recommendation_scores_and_sorts_each_difficulty() {
        let deck = MusicDeck {
            total_power: 100_000,
            event_bonus_rate: 100.0,
            cards: vec![
                MusicDeckCard {
                    skill_score_up: 100.0,
                    skill_life_recovery: 0.0
                };
                5
            ],
            ..MusicDeck::default()
        };
        let music = vec![
            MusicMeta {
                music_id: 2,
                difficulty: "master".to_string(),
                event_rate_solo: 100,
                event_rate_multi: 100,
                event_rate_auto: 100,
                base_score: 1.0,
                base_score_auto: 1.0,
                fever_score: 0.0,
                solo_skill_scores: [0.0; 6],
                multi_skill_scores: [0.0; 6],
                auto_skill_scores: [0.0; 6],
                music_time: 0.0,
                tap_count: 0,
            },
            MusicMeta {
                music_id: 1,
                difficulty: "expert".to_string(),
                base_score: 2.0,
                ..music_meta_default()
            },
        ];
        let options = MusicRecommendOptions {
            live_type: LiveType::Solo,
            event_type: EventType::Marathon,
            skill_order: LiveSkillOrder::Average,
            specific_skill_order: None,
            multi_teammate_score_up: None,
            multi_teammate_power: None,
        };

        let result = recommend_music(&music, &deck, &options).unwrap();
        assert_eq!(result[0].music_id, 1);
        assert!(result[0].live_score > result[1].live_score);
        assert!(result.iter().all(|item| item.event_point.is_some()));
    }

    #[test]
    fn cheerful_event_point_matches_default_life_rounding_order() {
        let live_score = 2_806_976;
        let music_rate = 110;
        let deck_bonus = 320.75;
        let base = 110 + (live_score as f64 / 17_000.0) as i32 + (live_score * 4 / 340_000).min(13);
        let before_life = (base as f64 * 1.10 * (deck_bonus / 100.0 + 1.0)) as i32;
        let life_rate = 1.15 + (1_000.0_f64 / 5_000.0).clamp(0.1, 0.2);
        let expected = (before_life as f64 * life_rate) as i32;

        assert_eq!(
            event_point(
                LiveType::Cheerful,
                EventType::CheerfulCarnival,
                live_score,
                music_rate,
                deck_bonus,
            )
            .unwrap(),
            expected,
        );
    }

    fn music_meta_default() -> MusicMeta {
        MusicMeta {
            music_id: 0,
            difficulty: String::new(),
            event_rate_solo: 100,
            event_rate_multi: 100,
            event_rate_auto: 100,
            base_score: 1.0,
            base_score_auto: 1.0,
            fever_score: 0.0,
            solo_skill_scores: [0.0; 6],
            multi_skill_scores: [0.0; 6],
            auto_skill_scores: [0.0; 6],
            music_time: 0.0,
            tap_count: 0,
        }
    }
}
