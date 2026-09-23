//! Exact Top-K of the unconstrained maximizing Power target.
//!
//! A card's resolved power depends on the rest of the deck only through two
//! deck-wide facts: the set of units carried by all five members and whether
//! all five share one attribute. A scenario fixes both facts; inside it every
//! card has one additive power ceiling, and a branch and bound over
//! character-distinct decks runs against the one canonical tracker.
use crate::pool::{CardIdx, CardPool};
use crate::search::budget::SearchBudget;
use crate::search::{
    DeckResult, SearchContext, SearchParams, SearchStats, TopKTracker, evaluate, placement,
};
use crate::types::DECK_SIZE;

/// Unit bits the power evaluator reads from a unit mask.
const UNIT_BITS: u8 = 6;
const UNIT_MASK: u8 = (1 << UNIT_BITS) - 1;
/// Attribute ids of pool cards.
const ATTRS: usize = 6;
/// Scenario attribute slots: no shared attribute, then one slot per attribute.
const ATTR_SLOTS: usize = ATTRS + 1;

/// One card inside a scenario.
#[derive(Clone, Copy)]
struct Entry {
    /// Upper bound on the card's resolved power in every deck of the scenario.
    power: u32,
    card: CardIdx,
    character: u8,
}

/// A scenario's admitted cards, by descending `power`, then pool index.
struct Scenario {
    entries: Vec<Entry>,
    /// Largest scenario sum of five character-distinct entries.
    ceiling: u32,
    order: usize,
}

pub(super) fn search_power_scenarios(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    budget: &mut SearchBudget,
) -> (Vec<DeckResult>, SearchStats) {
    let mut search = PowerSearch {
        pool,
        ctx,
        tracker: TopKTracker::new(params.top_k),
        stats: SearchStats::default(),
        budget,
        deck: [CardIdx::new(0); DECK_SIZE],
        game_ids: [u16::MAX; DECK_SIZE],
    };
    let mut scenarios = build_scenarios(pool);
    // Any order is exact; the most promising scenarios raise the cutoff first.
    scenarios.sort_unstable_by(|left, right| {
        right
            .ceiling
            .cmp(&left.ceiling)
            .then(left.order.cmp(&right.order))
    });
    for scenario in &scenarios {
        if search.budget.expired() {
            break;
        }
        if search.below_cutoff(scenario.ceiling) {
            search.stats.ub_prunes += 1;
        } else if !search.descend(&scenario.entries, 0, 0, 0, 0) {
            break;
        }
        search.stats.diagnostics.power_scenarios_completed += 1;
    }
    search.stats.deadline_hit = search.budget.hit;
    search.stats.finalize();
    (search.tracker.into_vec(), search.stats)
}

/// Unit sets that can be carried by all five members of a deck: every such
/// set is the intersection of its members' unit masks, so the distinct masks
/// closed under intersection, with the empty set, contain all of them.
fn unit_sets(pool: &CardPool) -> Vec<u8> {
    let mut sets = vec![0u8];
    for card in pool.indices() {
        let mask = pool.unit_mask_raw(card) & UNIT_MASK;
        if !sets.contains(&mask) {
            sets.push(mask);
        }
    }
    let mut index = 1;
    while index < sets.len() {
        for other in 0..index {
            let both = sets[index] & sets[other];
            if !sets.contains(&both) {
                sets.push(both);
            }
        }
        index += 1;
    }
    sets
}

/// Upper bound on `card`'s resolved power in every deck whose members all
/// carry exactly the units of `units` in common and share an attribute iff
/// `attr_all`; exact when `units` has at most one unit.
fn scenario_power(pool: &CardPool, card: CardIdx, units: u8, attr_all: bool) -> u32 {
    if units == 0 {
        return evaluate::resolve_card_power_scenario(pool, card, None, attr_all);
    }
    (0..usize::from(UNIT_BITS))
        .filter(|&unit| units & (1 << unit) != 0)
        .map(|unit| evaluate::resolve_card_power_scenario(pool, card, Some(unit), attr_all))
        .max()
        .unwrap_or(0)
}

/// Every scenario that admits at least five characters. A scenario with unit
/// set `U` and attribute slot `a` admits the cards whose mask contains `U` and,
/// for an attribute slot, whose attribute is that attribute.
fn build_scenarios(pool: &CardPool) -> Vec<Scenario> {
    let sets = unit_sets(pool);
    let mut lists = vec![Vec::<Entry>::new(); sets.len() * ATTR_SLOTS];
    for card in pool.indices() {
        let mask = pool.unit_mask_raw(card) & UNIT_MASK;
        let attr = usize::from(pool.attr(card));
        let character = pool.char_id(card);
        for (set_index, &units) in sets.iter().enumerate() {
            if units & !mask != 0 {
                continue;
            }
            let base = set_index * ATTR_SLOTS;
            lists[base].push(Entry {
                power: scenario_power(pool, card, units, false),
                card,
                character,
            });
            if attr < ATTRS {
                lists[base + 1 + attr].push(Entry {
                    power: scenario_power(pool, card, units, true),
                    card,
                    character,
                });
            }
        }
    }
    lists
        .into_iter()
        .enumerate()
        .filter_map(|(order, mut entries)| {
            entries.sort_unstable_by(|left, right| {
                right
                    .power
                    .cmp(&left.power)
                    .then(left.card.raw().cmp(&right.card.raw()))
            });
            let ceiling = best_completion(&entries, 0, 0, DECK_SIZE)?;
            Some(Scenario {
                entries,
                ceiling,
                order,
            })
        })
        .collect()
}

/// Largest sum of `slots` entries of `entries[pos..]` from distinct characters
/// outside `used`: the first entry of each character is its largest, and the
/// best `slots` characters are taken. `None` when fewer characters remain.
#[inline(always)]
fn best_completion(entries: &[Entry], pos: usize, used: u32, slots: usize) -> Option<u32> {
    let mut seen = used;
    let mut sum = 0u32;
    let mut taken = 0usize;
    for entry in &entries[pos..] {
        if taken == slots {
            break;
        }
        let bit = 1u32 << entry.character;
        if seen & bit != 0 {
            continue;
        }
        seen |= bit;
        sum += entry.power;
        taken += 1;
    }
    (taken == slots).then_some(sum)
}

struct PowerSearch<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    tracker: TopKTracker,
    stats: SearchStats,
    budget: &'a mut SearchBudget,
    deck: [CardIdx; DECK_SIZE],
    game_ids: [u16; DECK_SIZE],
}

impl PowerSearch<'_> {
    /// Whether decks whose summed scenario powers are at most `sum` all fall
    /// strictly below the current K-th objective.
    #[inline(always)]
    fn below_cutoff(&self, sum: u32) -> bool {
        self.tracker.cutoff().is_some_and(|cutoff| {
            let upper = self
                .ctx
                .clamp_power_total(sum.saturating_add(self.ctx.honor_bonus));
            u64::from(upper) < cutoff
        })
    }

    /// Branch and bound over `entries[pos..]` below a prefix of `depth` cards
    /// from the characters in `used`, whose scenario powers sum to `sum`.
    /// Returns `false` once the deadline expires.
    fn descend(
        &mut self,
        entries: &[Entry],
        depth: usize,
        pos: usize,
        used: u32,
        sum: u32,
    ) -> bool {
        self.stats.visited_nodes += 1;
        if depth == DECK_SIZE {
            self.stats.leaf_nodes += 1;
            if let Some(candidate) = placement::evaluate_candidate(self.pool, self.ctx, &self.deck)
            {
                self.tracker.insert(self.pool, self.ctx, candidate);
            }
            return true;
        }
        let slots = DECK_SIZE - depth;
        let Some(completion) = best_completion(entries, pos, used, slots) else {
            self.stats.feasibility_prunes += 1;
            return true;
        };
        if self.below_cutoff(sum + completion) {
            self.stats.ub_prunes += 1;
            return true;
        }
        for (offset, entry) in entries[pos..].iter().enumerate() {
            if self.budget.expired_sampled() {
                return false;
            }
            // Later entries are worth no more than this one.
            if self.below_cutoff(sum + entry.power * slots as u32) {
                self.stats.mono_break_prunes += 1;
                break;
            }
            let bit = 1u32 << entry.character;
            if used & bit != 0 {
                continue;
            }
            let game_id = self.pool.game_id(entry.card);
            if self.game_ids[..depth].contains(&game_id) {
                self.stats.feasibility_prunes += 1;
                continue;
            }
            self.deck[depth] = entry.card;
            self.game_ids[depth] = game_id;
            if !self.descend(
                entries,
                depth + 1,
                pos + offset + 1,
                used | bit,
                sum + entry.power,
            ) {
                return false;
            }
        }
        true
    }
}
