//! Mathematical search shape derived once from the public semantic context.
//!
//! A combination frontier is sufficient only when exchanging its free slots does
//! not change the evaluated objective. Otherwise each leaf also solves the small
//! exact placement problem; dense candidate indices never define legal roles.
use super::SearchContext;
use crate::types::{LiveSkillOrder, LiveType, ScoreTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SolverFamily {
    UniqueCombinations,
    NumericObjective,
    SameCharacter,
    LeaderGroups,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlacementModel {
    Exchangeable,
    OrderedFreeSlots,
    SelectableLeader,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DeckProblem {
    pub family: SolverFamily,
    pub placement: PlacementModel,
    pub fixed_prefix: usize,
    /// Slot 0 of a leaf holds the Final Chapter leader: the Final Chapter
    /// solvers choose it for every target except the numeric ones, and for
    /// those a forced leader character is moved there.
    pub leader_slot_fixed: bool,
    /// The leaf's forced Final Chapter leader card is moved to slot 0.
    pub move_forced_leader: bool,
    /// A single-player live under a specific skill order weights every skill
    /// slot differently, so the objective reads which member sits where.
    pub weighted_slots: bool,
}

impl DeckProblem {
    pub fn from_context(ctx: &SearchContext) -> Self {
        let family = if !ctx.enforce_char_uniqueness {
            SolverFamily::SameCharacter
        } else if matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill) {
            SolverFamily::NumericObjective
        } else if ctx.is_final_chapter {
            SolverFamily::LeaderGroups
        } else {
            SolverFamily::UniqueCombinations
        };
        let fixed_prefix = (ctx.fixed_card_ids.len() + ctx.fixed_character_ids.len()).min(5);
        let solver_leader = ctx.is_final_chapter && family != SolverFamily::NumericObjective;
        let move_forced_leader = ctx.is_final_chapter
            && family == SolverFamily::NumericObjective
            && fixed_prefix == 0
            && ctx.forced_leader_character_id.is_some();
        // Only a single-player live scores its skill slots one by one; a
        // MySekai live has no live score at all.
        let single_player = !matches!(
            ctx.effective_live_type(),
            LiveType::Multi | LiveType::Cheerful | LiveType::Mysekai
        );
        let weighted_slots = single_player
            && ctx.live_skill_order == LiveSkillOrder::Specific
            && matches!(ctx.target, ScoreTarget::Score | ScoreTarget::Bonus);
        // The Final Chapter evaluator seats the members after the leader by
        // public id, so no placement is left to choose there.
        let placement = if !ctx.is_final_chapter && weighted_slots {
            PlacementModel::OrderedFreeSlots
        } else if !solver_leader
            && !move_forced_leader
            && fixed_prefix == 0
            && ctx.forced_leader_character_id.is_none()
            && !ctx.effective_best_skill_as_leader()
            && (matches!(
                ctx.target,
                ScoreTarget::Score | ScoreTarget::Bonus | ScoreTarget::Skill
            ) || ctx.multi_live_score_up_lower_bound.is_some())
        {
            PlacementModel::SelectableLeader
        } else {
            PlacementModel::Exchangeable
        };
        Self {
            family,
            placement,
            fixed_prefix,
            leader_slot_fixed: solver_leader || move_forced_leader,
            move_forced_leader,
            weighted_slots,
        }
    }

    pub fn needs_placement_search(self) -> bool {
        self.placement != PlacementModel::Exchangeable
    }

    /// Whether replacing a member by one that is at least as strong in every
    /// dimension can lower the objective: the placement is searched, or the
    /// members' seats are weighted and fixed by public id, which the
    /// replacement can reorder.
    pub fn position_sensitive(self) -> bool {
        self.needs_placement_search() || self.weighted_slots
    }
}
