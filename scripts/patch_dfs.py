p='src/search/dfs.rs'
s=open(p,encoding='utf-8').read()

# A) entry signature
s=s.replace("""pub fn dfs_search_bonus_targets(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    targets: &[i32],
) -> (Vec<DeckResult>, SearchStats) {
    dfs_search_seeded_inner(pool, ctx, suffix, params, Vec::new(), Some(targets))
}""",
"""pub fn dfs_search_bonus_targets(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    targets: &[i32],
    bonus_reach: &BonusReach,
) -> (Vec<DeckResult>, SearchStats) {
    dfs_search_seeded_inner(pool, ctx, suffix, params, Vec::new(), Some(targets), Some(bonus_reach))
}""")

# B) inner signature + other callers pass None
s=s.replace("""    seeds: Vec<DeckResult>,
    bonus_targets: Option<&[i32]>,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }""",
"""    seeds: Vec<DeckResult>,
    bonus_targets: Option<&[i32]>,
    bonus_reach: Option<&BonusReach>,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }""")
s=s.replace("dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None)",
            "dfs_search_seeded_inner(pool, ctx, suffix, params, seeds, None, None)")

# C) SearchState field + construction
s=s.replace("""pub(crate) struct SearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    deadline: Option<Instant>,
    tracker: &'a mut SearchTracker,""",
"""pub(crate) struct SearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    deadline: Option<Instant>,
    tracker: &'a mut SearchTracker,
    /// Bonus-target reachability bitsets; `None` on every non-bucket path.
    bonus_reach: Option<&'a BonusReach>,""")
s=s.replace("""    let mut state = SearchState {
        pool,
        ctx,
        suffix,
        deadline,
        tracker: &mut tracker,""",
"""    let mut state = SearchState {
        pool,
        ctx,
        suffix,
        deadline,
        tracker: &mut tracker,
        bonus_reach,""")

# D) recurse signature + bonus prune block
s=s.replace("""    fn recurse(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
    ) {""",
"""    fn recurse(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        bonus_x10: u32,
    ) {""")
s=s.replace("""        if self.tracker.is_bonus() {
            let upper = self.suffix.upper_bound_with_depth(depth, &used, &partial);
            // partial.bonus 是逐卡 ceil 百分比；每张卡至多高估 0.5%，
            // 因此 2*ceil-depth 是精确 x2 bonus 的安全下界。
            let lower_bonus_x2 = partial.bonus.saturating_mul(2).saturating_sub(depth as u32);
            if self.tracker.bonus_can_prune(lower_bonus_x2, upper) {
                self.stats.ub_prunes += 1;
                return;
            }
        }""",
"""        if self.tracker.is_bonus() {
            let upper = self.suffix.upper_bound_with_depth(depth, &used, &partial);
            // partial.bonus 是逐卡 ceil 百分比；每张卡至多高估 0.5%，
            // 因此 2*ceil-depth 是精确 x2 bonus 的安全下界。
            let lower_bonus_x2 = partial.bonus.saturating_mul(2).saturating_sub(depth as u32);
            // World Bloom 的加成合计走 limited-count 分支，与逐卡求和模型不一致，
            // 该场景保持旧行为（不做可达性剪枝）。
            let reach = if self.ctx.is_world_bloom {
                None
            } else {
                self.bonus_reach
            };
            if self
                .tracker
                .bonus_can_prune(lower_bonus_x2, upper, reach, start, depth, bonus_x10)
            {
                self.stats.ub_prunes += 1;
                return;
            }
        }""")

# E) recurse main-loop recursive call passes precise x10
s=s.replace("""            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_score_noevent_monotonic(""",
"""            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
                bonus_x10 + self.pool.event_bonus(card).total_x10() as u32,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_score_noevent_monotonic(""")

# F) recurse_simple signature + dispatch + its recursive call
s=s.replace("""    fn recurse_simple(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
    ) {""",
"""    fn recurse_simple(
        &mut self,
        depth: usize,
        start: usize,
        deck: &mut [CardIdx; 5],
        used: UsedSet,
        partial: PartialDeck,
        fixed_leader: Option<CardIdx>,
        bonus_x10: u32,
    ) {""")
s=s.replace("""                } else {
                    self.recurse_simple(depth, start, deck, used, partial, fixed_leader);
                }""",
"""                } else {
                    self.recurse_simple(depth, start, deck, used, partial, fixed_leader, bonus_x10);
                }""")
s=s.replace("""            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
            );
        }
    }

    #[inline(always)]
    fn timed_out""",
"""            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
                bonus_x10 + self.pool.event_bonus(card).total_x10() as u32,
            );
        }
    }

    #[inline(always)]
    fn timed_out""")

# G) non-bonus variant call sites pass 0
s=s.replace("""            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_ep(""",
"""            self.recurse(
                depth + 1,
                dense,
                deck,
                next_used,
                next_partial,
                fixed_leader,
                0,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn recurse_ep(""")
s=s.replace("""        self.recurse(
            depth + 1,
            dense,
            deck,
            next_used,
            next_partial,
            fixed_leader,
        );
    }""",
"""        self.recurse(
            depth + 1,
            dense,
            deck,
            next_used,
            next_partial,
            fixed_leader,
            0,
        );
    }""")

# H) bootstrap call sites
s=s.replace("            state.recurse(1, 0, &mut deck, used, partial, Some(leader));",
            "            state.recurse(1, 0, &mut deck, used, partial, Some(leader), 0);")
s=s.replace("""        state.recurse(
            0,
            0,
            &mut deck,
            UsedSet::new(),
            PartialDeck::default(),
            None,
        );""",
"""        state.recurse(
            0,
            0,
            &mut deck,
            UsedSet::new(),
            PartialDeck::default(),
            None,
            0,
        );""")

# I) tracker plumbing
s=s.replace("""    fn bonus_can_prune(&self, lower_bonus_x2: u32, upper: u64) -> bool {
        match self {
            Self::Bonus(tracker) => tracker.can_prune(lower_bonus_x2, upper),
            Self::TopK(_) => false,
        }
    }""",
"""    fn bonus_can_prune(
        &self,
        lower_bonus_x2: u32,
        upper: u64,
        bonus_reach: Option<&BonusReach>,
        start: usize,
        depth: usize,
        bonus_x10: u32,
    ) -> bool {
        match self {
            Self::Bonus(tracker) => tracker.can_prune(
                lower_bonus_x2,
                upper,
                bonus_reach,
                start,
                depth,
                bonus_x10,
            ),
            Self::TopK(_) => false,
        }
    }""")
s=s.replace("""    fn can_prune(&self, lower_bonus_x2: u32, upper: u64) -> bool {
        let max_bonus_x2 = (upper >> 32) as u32;
        let live_upper = upper as u32 as u64;
        for (target, tracker) in &self.buckets {
            if *target < lower_bonus_x2 {
                continue;
            }
            if *target > max_bonus_x2 {
                break;
            }
            let threshold = tracker.threshold();
            if threshold == 0 || live_upper >= (threshold as u32 as u64) {
                return false;
            }
        }
        true
    }""",
"""    fn can_prune(
        &self,
        lower_bonus_x2: u32,
        upper: u64,
        bonus_reach: Option<&BonusReach>,
        start: usize,
        depth: usize,
        bonus_x10: u32,
    ) -> bool {
        let max_bonus_x2 = (upper >> 32) as u32;
        let live_upper = upper as u32 as u64;
        let remaining = DECK_SIZE - depth.min(DECK_SIZE);
        // The subtree can only ever produce buckets that are (a) already
        // populated and still under their live threshold, or (b) empty but
        // reachable by some combination of the remaining cards. Anything else
        // is provably dead weight and gets pruned.
        let mut satisfiable = false;
        for (target, tracker) in &self.buckets {
            if *target < lower_bonus_x2 {
                continue;
            }
            if *target > max_bonus_x2 {
                break;
            }
            let threshold = tracker.threshold();
            if threshold == 0 {
                let Some(reach) = bonus_reach else {
                    satisfiable = true;
                    continue;
                };
                // round(x10 / 5) == x2 holds exactly for
                // x10 in [5*x2 - 2, 5*x2 + 2].
                let center = target.saturating_mul(5);
                let lo = center.saturating_sub(2);
                let hi = center.saturating_add(2);
                if reach.any_in_range(
                    start,
                    remaining,
                    bonus_x10.saturating_add(lo),
                    bonus_x10.saturating_add(hi),
                ) {
                    satisfiable = true;
                }
                continue;
            }
            if live_upper >= threshold as u64 {
                satisfiable = true;
            }
        }
        !satisfiable
    }""")
open(p,'w',encoding='utf-8',newline='\n').write(s)
print('done')
