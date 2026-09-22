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
        let single_player = !matches!(
            ctx.effective_live_type(),
            LiveType::Multi | LiveType::Cheerful | LiveType::Mysekai
        );
        let placement = if !ctx.is_final_chapter
            && single_player
            && ctx.live_skill_order == LiveSkillOrder::Specific
            && matches!(ctx.target, ScoreTarget::Score | ScoreTarget::Bonus)
        {
            PlacementModel::OrderedFreeSlots
        } else if !ctx.is_final_chapter
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
        }
    }

    pub fn needs_placement_search(self) -> bool {
        self.placement != PlacementModel::Exchangeable
    }
}
