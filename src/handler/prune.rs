//! 建池期的精确支配淘汰。
//!
//! `CardPool` 的位图宽度固定为 `MASK_WORDS * 64`，候选数超过它就无法建池。
//! 常规活动靠 `filter` 里的逐角色裁剪收敛，但那些裁剪按单卡标量排序，
//! 而 World Bloom 的加成含 `diff_attr_bonus`（按卡组不同属性数给档）、支援挤占、
//! limited 计数上限与队长专属加成，都不是单卡可分解的量，用标量裁剪会掉解。
//!
//! 这里只做一件有证明的事：**淘汰能被同角色同属性的另一张卡支配的卡**。
//! 支配者顶替被淘汰者入队时，角色唯一性仍成立、`diff_attr_bonus` 档位不变、
//! 战力/技能/加成各维都不更劣，因此卡组分数不会下降。
//!
//! 判据必须**不弱于** `search::dominance::dominates`（宁可少淘汰、不可多淘汰）：
//! 战力比较覆盖全部 (unit × member) 组合，而池内只保留其中 8 个槽位，
//! 因此这里的条件更严。改动本文件时保持这个方向。
//!
//! 只在候选装不下时启用。装得下时由搜索期的 `eliminate_dominated` 处理——
//! 它做同样的压缩，但会记录 alternatives 并在搜索后把次优解换回来。
//! 本模块淘汰的卡进不了池子、拿不到 CardIdx，救不回来：Top-1 仍精确，
//! Top-K 会少解。这是「有结果」对「报 TooManyCards」的取舍，不是默认策略。

use std::collections::HashMap;

use crate::search::SupportDeck;
use crate::types::DECK_SIZE;

use super::gather::CardIntermediate;
use super::types;

/// 支援维度：逐队长角色的支援加成（×100）与顶替下界。
///
/// 支援表内的卡编入队伍会让出支援位，由表中下一张未入队的卡顶替，
/// 损失「自身加成 − 顶替者加成」。队伍另外 4 张也可能同时占位，
/// 顶替者最差落到第 `count + DECK_SIZE - 1` 位，取这一位才是差额的安全下界。
pub(super) struct SupportBonusTable {
    bonus_x100: HashMap<u16, [i32; 27]>,
    replacement_floor_x100: [i32; 27],
}

impl SupportBonusTable {
    /// 从逐队长支援卡组构建；全为 0 时返回 `None`（无支援维度需要考虑）。
    pub(super) fn build(
        decks_by_character: &[SupportDeck],
        fallback: &SupportDeck,
    ) -> Option<Self> {
        let mut bonus_x100: HashMap<u16, [i32; 27]> = HashMap::new();
        let mut replacement_floor_x100 = [0i32; 27];
        let mut any = false;
        for (char_id, floor) in replacement_floor_x100.iter_mut().enumerate() {
            let deck = decks_by_character
                .get(char_id)
                .filter(|deck| deck.count > 0)
                .unwrap_or(fallback);
            let count = deck.count as usize;
            if count == 0 {
                continue;
            }
            // 顶替下界向下取整、卡自身加成向上取整：差额只会被高估，不会被低估。
            *floor = deck
                .cards
                .get(count + DECK_SIZE - 1)
                .map(|(_, bonus)| (bonus * 100.0).floor() as i32)
                .unwrap_or(0);
            for &(game_id, bonus) in deck.cards.iter().take(count) {
                let value = (bonus * 100.0).ceil() as i32;
                if value == 0 {
                    continue;
                }
                bonus_x100.entry(game_id).or_insert([0i32; 27])[char_id] = value;
                any = true;
            }
        }
        any.then_some(Self {
            bonus_x100,
            replacement_floor_x100,
        })
    }

    /// `lhs` 顶替 `rhs` 入队时，支援加成最多多损失多少（×100，非负）。
    fn deficit_x100(&self, lhs: u16, rhs: u16) -> i32 {
        let zero = [0i32; 27];
        let lhs_bonus = self.bonus_x100.get(&lhs).unwrap_or(&zero);
        let rhs_bonus = self.bonus_x100.get(&rhs).unwrap_or(&zero);
        let mut worst = 0i32;
        for char_id in 0..27usize {
            let floor = rhs_bonus[char_id].max(self.replacement_floor_x100[char_id]);
            worst = worst.max(lhs_bonus[char_id] - floor);
        }
        worst
    }
}

/// 把候选收敛到 `capacity` 以内，只丢弃可证明被支配的卡。
///
/// 返回实际丢弃的数量。仍然装不下时保持原样，由调用方报 `TooManyCards`——
/// 本函数不做任何近似裁剪。
///
/// 丢弃顺序为「最弱的先丢」：所有可丢的卡都同样安全，先丢弱的能把
/// Top-K 的影响压到最小。
pub(super) fn dominance_trim(
    cards: &mut Vec<CardIntermediate>,
    params: &types::BuildParams,
    support: Option<&SupportBonusTable>,
    capacity: usize,
) -> usize {
    if cards.len() <= capacity {
        return 0;
    }

    let power_slots: Vec<[i32; 24]> = cards.iter().map(power_profile).collect();

    // 同角色同属性分桶：跨属性不比较，`diff_attr_bonus` 的档位因此不会被改变。
    let mut buckets: HashMap<(u8, u8), Vec<usize>> = HashMap::new();
    for (index, card) in cards.iter().enumerate() {
        buckets
            .entry((card.character_id, card.attr))
            .or_default()
            .push(index);
    }

    let mut alive = vec![true; cards.len()];
    // 被谁支配：用于末尾复核「淘汰者的支配者仍然存活」。
    let mut dominated_by: Vec<Option<usize>> = vec![None; cards.len()];

    for indices in buckets.values() {
        for &a in indices {
            if !alive[a] {
                continue;
            }
            for &b in indices {
                if a == b || !alive[b] || is_exempt(&cards[b], params) {
                    continue;
                }
                if dominates(
                    &cards[a],
                    &cards[b],
                    &power_slots[a],
                    &power_slots[b],
                    support,
                ) {
                    alive[b] = false;
                    dominated_by[b] = Some(a);
                }
            }
        }
    }

    // 复核：支配关系叠加支援差额后不保证传递，因此不能依赖「支配者的支配者」。
    // 凡是支配者已被淘汰的卡，重新找一个仍存活的支配者，找不到就救回。
    for index in 0..cards.len() {
        if alive[index] {
            continue;
        }
        let root_alive = dominated_by[index].is_some_and(|root| alive[root]);
        if root_alive {
            continue;
        }
        let bucket = &buckets[&(cards[index].character_id, cards[index].attr)];
        let replacement = bucket.iter().copied().find(|&other| {
            other != index
                && alive[other]
                && dominates(
                    &cards[other],
                    &cards[index],
                    &power_slots[other],
                    &power_slots[index],
                    support,
                )
        });
        match replacement {
            Some(root) => dominated_by[index] = Some(root),
            None => alive[index] = true,
        }
    }

    let mut removable = (0..cards.len()).filter(|&i| !alive[i]).collect::<Vec<_>>();
    // 最弱的先丢：加成升序、稀有度升序、综合力×技能升序。
    removable.sort_by(|&left, &right| {
        let (lhs, rhs) = (&cards[left], &cards[right]);
        lhs.event_bonus
            .total_x10()
            .cmp(&rhs.event_bonus.total_x10())
            .then_with(|| lhs.card_rarity_type.cmp(&rhs.card_rarity_type))
            .then_with(|| power_rank(lhs).cmp(&power_rank(rhs)))
            .then_with(|| rhs.game_card_id.cmp(&lhs.game_card_id))
    });

    let target = cards.len().saturating_sub(capacity);
    let mut drop = vec![false; cards.len()];
    let dropped = removable.len().min(target);
    for &index in removable.iter().take(dropped) {
        drop[index] = true;
    }
    if dropped == 0 {
        return 0;
    }

    let mut index = 0usize;
    cards.retain(|_| {
        let keep = !drop[index];
        index += 1;
        keep
    });
    dropped
}

/// 固定卡与固定角色永不淘汰。
fn is_exempt(card: &CardIntermediate, params: &types::BuildParams) -> bool {
    params.fixed_cards.contains(&card.game_card_id)
        || params
            .fixed_characters
            .contains(&(card.character_id as i32))
}

/// 全部 (unit × member) 组合下的综合力。池内只保留其中 8 个槽位，
/// 这里比较全部 24 个，条件严于池级判据。
fn power_profile(card: &CardIntermediate) -> [i32; 24] {
    let mut out = [0i32; 24];
    let common = card.power.base
        + card.power.character_bonus
        + card.power.fixture_bonus
        + card.power.gate_bonus;
    let mut slot = 0usize;
    for unit in 0..6usize {
        for member in 0..4usize {
            out[slot] = common + card.power.area_item_bonus[unit][member];
            slot += 1;
        }
    }
    out
}

fn power_rank(card: &CardIntermediate) -> i64 {
    card.power.power_max.max(0) as i64 * (256 + card.skill.skill_max as i64)
}

/// `lhs` 是否支配 `rhs`：同属性、unit 覆盖、技能同型且不劣、各维不劣，
/// 且 World Bloom 下支援差额付得起。
fn dominates(
    lhs: &CardIntermediate,
    rhs: &CardIntermediate,
    lhs_power: &[i32; 24],
    rhs_power: &[i32; 24],
    support: Option<&SupportBonusTable>,
) -> bool {
    debug_assert_eq!(lhs.character_id, rhs.character_id);
    debug_assert_eq!(lhs.attr, rhs.attr);

    // rhs 能上的每个 unit，lhs 也要能上。
    if (rhs.unit_mask_raw & lhs.unit_mask_raw) != rhs.unit_mask_raw {
        return false;
    }
    if lhs_power.iter().zip(rhs_power.iter()).any(|(l, r)| l < r) {
        return false;
    }
    if lhs.power.power_min < rhs.power.power_min || lhs.power.power_max < rhs.power.power_max {
        return false;
    }
    if !skill_dominates(lhs, rhs) {
        return false;
    }
    if lhs.event_bonus.base_x10() < rhs.event_bonus.base_x10()
        || lhs.event_bonus.limited_x10() < rhs.event_bonus.limited_x10()
    {
        return false;
    }
    // 终章的队长专属加成只在 0 号位生效，取不劣即可覆盖该位。
    if lhs.leader_honor_bonus < rhs.leader_honor_bonus
        || lhs.leader_limit_bonus < rhs.leader_limit_bonus
    {
        return false;
    }

    let Some(support) = support else {
        return true;
    };
    // 支援差额用 lhs 多出的 base 加成抵扣：两者在活动加成总和里同为百分比加项。
    let deficit = support.deficit_x100(
        lhs.game_card_id.clamp(0, u16::MAX as i32) as u16,
        rhs.game_card_id.clamp(0, u16::MAX as i32) as u16,
    );
    if deficit <= 0 {
        return true;
    }
    let surplus_x10 = lhs.event_bonus.base_x10() as i32 - rhs.event_bonus.base_x10() as i32;
    deficit <= surplus_x10 * 10
}

fn skill_dominates(lhs: &CardIntermediate, rhs: &CardIntermediate) -> bool {
    if lhs.skill.slot.skill_type != rhs.skill.slot.skill_type {
        return false;
    }
    if lhs.skill.skill_min < rhs.skill.skill_min || lhs.skill.skill_max < rhs.skill.skill_max {
        return false;
    }
    match (&lhs.skill.unit_count, &rhs.skill.unit_count) {
        (Some(left), Some(right)) => {
            if left.unit != right.unit
                || left
                    .score_up
                    .iter()
                    .zip(right.score_up.iter())
                    .any(|(l, r)| l < r)
            {
                return false;
            }
        }
        (None, None) => {}
        _ => return false,
    }
    match (&lhs.skill.diff, &rhs.skill.diff) {
        (Some(left), Some(right)) => {
            if left.base < right.base || left.increment < right.increment {
                return false;
            }
        }
        (None, None) => {}
        _ => return false,
    }
    match (&lhs.skill.ref_skill, &rhs.skill.ref_skill) {
        (Some(left), Some(right)) => {
            if left.rate < right.rate || left.max < right.max {
                return false;
            }
        }
        (None, None) => {}
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::BuildParams;
    use crate::pool::{EventBonusExact, SkillSlot};
    use crate::search::SupportDeck;

    fn card(
        game_card_id: i32,
        character_id: u8,
        attr: u8,
        power: i32,
        skill: u8,
    ) -> CardIntermediate {
        CardIntermediate {
            game_card_id,
            card_rarity_type: 4,
            character_id,
            attr,
            unit_mask_raw: 1,
            default_image: crate::types::DefaultImage::Original,
            after_training: false,
            skill_state_controls_image: false,
            master_rank: 0,
            skill_level: 1,
            power: super::super::power::PowerResult {
                unit_mask: 1,
                base: power,
                power_min: power,
                power_max: power,
                ..Default::default()
            },
            skill: super::super::skill::SkillResult {
                slot: SkillSlot {
                    skill_type: 0,
                    value: skill,
                },
                skill_min: skill,
                skill_max: skill,
                ..Default::default()
            },
            event_bonus: EventBonusExact::from_x10(0, 0),
            has_char_bonus: false,
            has_attr_bonus: false,
            leader_honor_bonus: 0,
            leader_limit_bonus: 0,
            ep_sort_key: 0,
        }
    }

    #[test]
    fn dominance_trim_only_drops_dominated_cards_down_to_capacity() {
        // 同角色同属性下 power/skill 递增，后一张支配前一张；
        // 另一角色单卡不可淘汰。容量 3 时只允许丢 1 张，且必须是最弱的那张。
        let mut cards = vec![
            card(1, 1, 0, 100, 1),
            card(2, 1, 0, 200, 2),
            card(3, 1, 0, 300, 3),
            card(4, 2, 0, 400, 4),
        ];
        let dropped = dominance_trim(&mut cards, &BuildParams::default(), None, 3);

        assert_eq!(dropped, 1);
        assert_eq!(cards.len(), 3);
        assert!(
            !cards.iter().any(|card| card.game_card_id == 1),
            "最弱且被支配的卡应最先丢弃",
        );
    }

    #[test]
    fn dominance_trim_keeps_every_attribute_available() {
        // 同角色不同属性：互不比较，任何一个属性都不会被清空。
        let mut cards = vec![
            card(1, 1, 0, 100, 1),
            card(2, 1, 1, 900, 9),
            card(3, 1, 2, 900, 9),
            card(4, 1, 0, 800, 8),
        ];
        dominance_trim(&mut cards, &BuildParams::default(), None, 1);

        for attr in [0u8, 1, 2] {
            assert!(
                cards.iter().any(|card| card.attr == attr),
                "属性 {attr} 不得被清空：diff_attr_bonus 按卡组不同属性数给档",
            );
        }
    }

    #[test]
    fn dominance_trim_reports_nothing_when_no_card_is_dominated() {
        // 各卡互有长短（战力高者技能低），谁也不支配谁，容量再小也不丢。
        let mut cards = vec![
            card(1, 1, 0, 900, 1),
            card(2, 1, 0, 100, 9),
            card(3, 1, 0, 500, 5),
        ];
        let dropped = dominance_trim(&mut cards, &BuildParams::default(), None, 1);

        assert_eq!(dropped, 0, "无可证明被支配的卡时不得丢弃");
        assert_eq!(cards.len(), 3);
    }

    #[test]
    fn support_deficit_blocks_domination_without_bonus_surplus() {
        // 支援表内的卡入队会让出支援位。加成盈余为 0 时差额付不起，不得支配。
        let deck = SupportDeck {
            cards: vec![(2, 5.0), (99, 1.0)],
            count: 1,
        };
        let support = SupportBonusTable::build(&[], &deck).expect("support table");
        let mut cards = vec![card(2, 1, 0, 200, 2), card(1, 1, 0, 100, 1)];
        let dropped = dominance_trim(&mut cards, &BuildParams::default(), Some(&support), 1);

        assert_eq!(dropped, 0, "支援差额无加成盈余可抵扣时不得淘汰");
    }
}
