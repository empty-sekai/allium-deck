# Allium Deck exactness proof

This document states the end-to-end exactness argument for the recommendation
searches. It is intentionally stronger than a benchmark claim: heuristic
procedures may choose visit order or seed incumbents, but only mathematically
admissible rules may remove search space. Detailed proofs for every pruning
rule are collected in [pruning-proof.md](pruning-proof.md).

The exactness guarantee is:

- pool construction either preserves the complete supported candidate set or
  returns an explicit capacity / representation error;
- if a search returns `SearchCompletion::Complete`, its results are exactly
  the canonical Top-K of that supported feasible set;
- if a search returns `SearchCompletion::TimedOut`, every returned incumbent
  is legal and exactly evaluated, but the Top-K claim is deliberately
  unproven;
- completion is solver-derived from the sticky `SearchStats.deadline_hit`
  bit. It is never reconstructed from elapsed wall-clock time.

An explicit capacity error and a timed-out partial result are visible API
outcomes, not approximate answers mislabeled as exact.

## 1. Candidate-pool completeness

The pool builder applies only hard user constraints (excluded cards, unit /
attribute filters, challenge character, fixed-card semantics) before search.

The historical rarity / event-point prefilter, per-character quotas, target
quality prefixes, and representative-only bonus-class deduplication are not
used by the production exact path.  Those transforms can remove a legal Top-K
set before the card obtains a dense CardIdx, so search-layer alternative
recovery cannot repair them.

The one extra reduction for exact bonus tiers is safe because all event-bonus
contributions are non-negative. The pool builder compares the highest requested
tier with a per-card **unavoidable lower bound**: base bonus when limited bonus
may be omitted by the event's counting cap, or total bonus only when limited
bonus is unconditional. If that unavoidable contribution alone exceeds the
highest requested tier, every deck containing the card exceeds every requested
tier, so the card cannot occur in a feasible tier solution.

The metadata mask remains `MASK_WORDS = 8`, i.e. 512 bits. Pools above that
width retain every candidate in the SoA columns and expose complete index
iterators; fixed-width mask accessors return `None` instead of a partial set.
If hard filtering leaves more than 65,535 candidates, the dense `CardIdx`
cannot represent the pool and construction returns `TooManyCards`. Other packed
metadata / special-skill representation limits are likewise checked before
lossy conversion; overflow is an explicit build error rather than saturation
or truncation.

### Placement and role completeness

Fixed cards, fixed characters, leader constraints, character uniqueness and
ordered / slot-sensitive roles are feasibility constraints, not ranking hints.
A solver specialization is used only when its state represents those
constraints completely. In particular, grouped Final Chapter search is entered
only when there are no fixed cards, at most one fixed character, and bonus
ordering is not observable; multiple fixed slots or position-sensitive bonus
semantics fall back to the slot-aware DFS.

Challenge-family dispatch happens before the generic numeric-target dispatch,
because Challenge requires five cards of one character whereas ordinary Power /
Skill search requires character uniqueness. Specific / role-order cases that
make placement observable retain the complete placement frontier instead of
using exchangeable-slot dominance.

Cultivation variants that share one public game-card ID remain representable as
distinct dense variants until canonical result collection. Public Top-K
distinctness is defined on the public card-ID set, while the deterministic
representative is selected by the canonical total order below.

## 2. Canonical Top-K, distinctness and threshold pruning

Every solver inserts legal leaves through the same `TopKTracker` total order.
In better-first order the key is:

1. objective value: descending for ordinary maximizing objectives, ascending
   only for Power minimization;
2. for MySekai ties, resolved and clamped deck power descending;
3. sorted public game-card ID set ascending;
4. ordered game-card IDs (placement / leader order) ascending;
5. dense card variants ascending.

The public set in item 3 is also the deduplication identity: two concrete
variants / placements with the same five public game-card IDs contribute one
Top-K set, and the best canonical representative is retained. The public
`compare_deck_results` wrapper exposes this same order for aggregation paths;
callers must not replace it with a score-only comparator.

Numeric branch-and-bound thresholds deliberately ignore later tie-break fields.
For maximizing searches, a branch is discarded only when its admissible upper
bound is strictly below the current K-th score. Equality is retained, including
the SIMD mask. For minimization, the dual rule discards only when the
admissible lower bound is strictly above the K-th value.

This strict relation is required even when K incumbents already exist: an
equal-objective branch may contain a distinct public set or a canonically better
representative.

## 3. Ordinary score / event score / MySekai

The generic DFS evaluates every surviving leaf with the production leaf
evaluator.  Pruning uses SuffixBound relaxations:

- remaining power is bounded by independently maximizing per-character power;
- remaining skill is bounded by independently maximizing per-character skill;
- leader skill uses the best possible remaining or already selected leader
  value;
- event bonus is independently maximized;
- constraints omitted by the bound (cross-card coupling, fixed-slot details,
  support occupancy beyond the modeled state) only enlarge the feasible set.

Therefore every suffix value is greater than or equal to the value of any legal
completion.

### MySekai with a uniform bonus

When every card of the pool has the same exact event bonus and the event is
neither World Bloom nor Final Chapter, every deck has the same total bonus $B$.
The MySekai value $m(P, B)$ is non-decreasing in the resolved power $P$ for
fixed $B$, and resolved power is the first MySekai tie-break. For two decks,
$P_1 > P_2$ gives $m_1 \ge m_2$ and the first deck ranks higher under either
target; $P_1 = P_2$ gives $m_1 = m_2$ and both targets fall through to the same
public-set and placement keys. The canonical MySekai order therefore equals the
canonical Power order under the same constraints, so the search runs the exact
Power solver and re-evaluates each returned deck with the MySekai evaluator.

### Event-point ceilings

For event Score targets, the independent suffix first forms admissible ceilings
for power, bonus, total skill and leader skill. The production live-score and
event-point formulas are monotone in those non-negative inputs over the
supported domain (including the capped teammate-score term in Multi/Cheerful),
so evaluating the formula at independently relaxed maxima cannot underestimate
a legal completion. The packed objective keeps event point in the high 32 bits
and live score in the low 32 bits, so this ceiling is directly comparable with
the canonical numeric threshold while equality remains unpruned.

Unique-character searches are additionally split by area-item composition
regime (pruning-proof Section 13): each regime searches only the cards it
admits, with a per-card power bound valid for every deck of that regime, and
all regimes share one canonical tracker whose K-th value floors the later ones.

### Score / no-event exact ordering and pre-division comparison

When the target is Score and there is no event context, every legal leaf uses
the public packed objective `(live_score, live_score)`. Its total ordering is
therefore exactly the ordering of `live_score`; rebuilding an event-point-style
packed key cannot change a prune decision.

The live-score ceiling is computed as a non-negative integer numerator `N` with
scale `D = 1_000_000`. Instead of evaluating `floor(N / D)` at every bound
check, production compares `N` with `T * D`, where `T` is the incumbent live
threshold. For non-negative integers:

    floor(N / D) < T  iff  N < T * D.

This is an algebraic identity, not an approximation. The optimization removes a
hot integer division without changing the set of pruned branches. Dedicated
tests additionally check that each numerator ceiling straddles the exact
integer live-score threshold correctly.

### Correlated Score bound

For supported no-event Solo / Auto Average contexts, production may intersect
the independent suffix bound with a tighter role-aware power/skill envelope.
Let `P` be power, `S` total skill and `L` leader skill. For each positive slope
`lambda`, a per-character linear relaxation bounds

    B*R*P + lambda*(B*S + D*L)

while allowing exactly one remaining character to take the leader role. Member
and leader maxima are computed separately per character, so assigning a leader
replaces that character's member contribution rather than counting both.

Combining this linear envelope with the live-score product yields the concave
quadratic ceiling implemented by `CorrelatedBound`; coefficient construction
and the final division are rounded upward, with an additional integer safety
unit for bounded floating-point preparation noise. Unsupported contexts simply
fall back to the independent admissible bound.

Because the correlated value is itself an upper bound on every legal completion,
taking the tighter of it and another admissible upper bound remains admissible.
The exhaustive correlated-bound auditor is a verification aid for this
derivation, not the derivation itself.

### Dominance, substitutability and Top-K recovery

Search-layer dominance is an exchangeability proof, not merely a numeric
ranking. A card may be deleted only when a same-character replacement preserves
every completion-relevant condition represented by the mode:

- every precomputed power context is no worse;
- resolved skill type / bounds and training-state semantics are substitutable;
- exact base / limited event-bonus dimensions are no worse;
- attribute and unit-membership semantics needed by the remaining deck are
  identical;
- fixed public identities are never removed;
- for World Bloom, worst-case support-deck displacement is bounded and any
  deficit is paid for by guaranteed base event-bonus surplus;
- for Final member-only dominance, leader-only numeric benefits are excluded
  from the comparison and the appropriate leader support profile is used.

Position-sensitive objectives do not inherit the exchangeable-slot proof:
when placement is observable, the general dominance pass preserves the complete
candidate frontier.

Canonical ties are also part of substitutability. A numerically dominating
replacement is not allowed to delete a card if it would worsen the canonical
public-ID / dense-variant tie order. This is why the dominance predicate rejects
a replacement whose `(game_id, dense_index)` is lexicographically later.

For a real Top-K deck D containing a dominated card, replace every dominated
card by its surviving dominance root to obtain D'. By substitutability,

    objective(D') >= objective(D),

and when objectives tie, D' is not canonically worse. If D belongs to the true
Top-K, D' is therefore at or above the K-th threshold in the compacted pool.
Production search discovers the root set. Post-search substitution expansion
walks the inverse dominance chains, re-evaluates every substituted deck exactly,
and merges legal distinct public sets through the canonical tracker. Multi-slot
substitutions are recursive, so chains involving more than one dominated card
are covered.

## 4. Ordinary World Bloom attribute bound

World Bloom's different-attribute bonus is not separable per card.  The dense
suffix stores, for each attribute, the set of characters that still have at
least one card of that attribute.

Consider any legal completion and only the attributes not already represented
in the prefix.  Each such novel attribute is supplied by one selected card, and
character uniqueness gives each of those cards a distinct character.  Thus the
completion induces a matching from novel attributes to unused characters in
the suffix attribute/character graph.

Consequently:

    novel attributes in any completion
        <= maximum bipartite matching size.

The matching size is therefore an admissible upper bound on how many new
attributes can still be added.  The bound maximizes diff_attr_bonus over all
attribute counts up to that reachable size.  Other score dimensions remain
independently relaxed, so composing them cannot make the total bound smaller
than a feasible completion.

The previous looser bound remains useful only as an A/B verification knob.
Controlled equivalence tests require the canonical result sequence to remain
identical when the tighter matching bound is enabled.

## 5. Final Chapter

### 5.1 Auto leader completeness

The proof-carrying auto-leader search enumerates every surviving card as a
leader candidate.  There is no per-character leader count cap.

Beam search, one-swap improvement, leader-key ranking, and other limited
prefixes are incumbent generators only.  They may improve the lower threshold
but cannot remove a leader job.

A historical 3-leaders-per-character cap is provably unsound.  The permanent
regression constructs five characters with four mutually non-dominating
variants each.  The first three variants are stronger by the leader ranking but
occupy valuable World Bloom support slots; the fourth variants are absent from
support and together form the global optimum.  Extra high-key decoys outrank
those fourth variants in every leader ranking.  Before removing the cap, production
returned 9,401,683,454,275 while the independent leader-by-leader oracle
returned 9,861,244,954,337.

After the fix, every leader is either searched or rejected by an admissible
leader/job ceiling.

### 5.2 Character-group attribute DP

A Final Chapter member position is first represented by a character group.
Each group records the bitmask of attributes available to that character.

For a suffix of character groups, the implementation stores `attr_bonus[k][s]`:
the maximum `diff_attr_bonus` reachable after selecting exactly `k` groups,
starting from an already selected 5-bit attribute set `s`.  The transition
keeps both skipping the current group and OR-ing each attribute available from
it into the starting set.  This is an equivalent memoized form of the complete
OR-product and remains exact for the isolated character-group/attribute
dimension, including nonmonotone bonus tables.

At a character-search prefix, production combines:

- the leader attribute,
- every mandatory selected group,
- every exact k-group union reachable from the remaining suffix,

and looks up the maximum bonus for each selected-prefix union. Power, skill,
limited bonus, and support are still independently relaxed. The combined score
ceiling is thus admissible.

At the card-within-group level, the analogous DP over the remaining selected
group masks is used.

The character-level ceiling independently takes the best remaining per-group
power, skill and base bonus values. Limited bonuses are not blindly summed: the
selected and suffix limited values are merged and only the largest values up to
the remaining `card_bonus_count_limit` are admitted. These maxima may come from
different concrete cards inside a character group, which only enlarges the
relaxation. The exact isolated attribute-union DP and the support ceiling are
then added as independent extra-bonus relaxations before calling the common
suffix evaluator.

After characters are chosen, the card-level plan applies the same argument to
the concrete group suffix: `rem_power`, `rem_skill`, `rem_base_bonus` and the
per-depth limited-value lists dominate every remaining card choice. When all
positive rounded limited values equal $v$, their top-$r$ sum is evaluated
equivalently as $\min(\text{sum}, rv)$; mixed values use the general sorted
merge. The attribute and support terms are bounded independently. Candidate-specific
support ceilings may tighten this value, but the looser pre-candidate ceiling
remains the fallback. Thus both character-job pruning and card-within-group
pruning use upper bounds on the same exact leaf objective.

### 5.3 Support-deck upper bound

Support decks are ordered from highest to lowest bonus.

For a fixed leader and selected prefix, production computes the exact current
support sum after removing selected main-deck cards and filling each vacated
slot with the next available support card.  Adding another main-deck card can
only leave that sum unchanged or replace an occupied support card by a
not-better later card.  Hence the current support sum is an upper bound for
every extension of the prefix.

The leader-only root bound similarly sums the best support entries after
excluding the leader.  Subsequent main-deck choices can only reduce it.

### 5.4 Member dominance and Top-K

Final Chapter member dominance excludes leader-only bonuses and includes the
support-displacement dimension.  Removed member variants are recorded, mapped
back through the first dominance pass, and restored after exact search.

Because a dominance root can appear only as the best leader representative of
its card set, Top-K recovery also rotates each returned set through legal leader
positions before applying member substitutions.  Every generated arrangement
is exact-leaf evaluated.

Controlled A/B verification treats this DP as a bound-only optimization:
canonical results must remain identical with the optimization disabled.

## 6. Power target

### 6.1 Unconstrained maximizing Power

The existing 49-scenario DP is exact.

A card's power resolution depends on two deck-wide binary conditions for each
unit/attribute scenario: whether all five members satisfy a unit condition and
whether all five members share an attribute.  Enumerating the no-unit case plus
six unit choices, crossed with no-all-attribute plus the attribute choices,
covers every power-resolution scenario.

Inside a fixed scenario each card contributes an additive value.  Processing
characters independently enforces character uniqueness.  Keeping the best K
partial distinct states for each cardinality is safe: all future additive
choices are independent of the discarded prefix, so a prefix already below K
better prefixes can never re-enter the final Top-K.

### 6.2 Fixed constraints and Power minimization

These modes no longer use a quality prefix.  The full candidate pool is
searched.

For maximization, let M be the largest per-card power_max.  For a prefix with
selected power-max sum P and r free slots,

    UB = clamp(P + r*M + honor).

Each real selected card contributes no more than its power_max, and allowing
the same global maximum to fill every remaining slot ignores uniqueness and
fixed-slot restrictions.  It is therefore a relaxation and an admissible upper
bound.

For minimization, let m be the smallest value over all cards and all eight
precomputed power contexts.  For selected cards use each card's own minimum.
Then

    LB = clamp(P_min + r*m + honor)

is no greater than any legal completion.  The branch is pruned only if LB is
strictly worse than the K-th incumbent.

## 7. Skill target

The full candidate pool is searched; no skill-quality prefix is used.

The public Skill objective, after the x10 encoding used by the evaluator, is

    10*leader + 2*sum(other four)
  = 2*sum(all five) + 8*leader.

For every card, skill_max is constructed as an upper bound on the resolved
skill:

- normal skill: its exact score-up;
- unit-count skill: maximum table entry;
- different-unit skill: its maximum possible base + increments;
- reference skill: base/reference contribution capped by the stored maximum.

The search tightens these per-card maxima with the deck composition: a
unit-count skill is bounded by its best table entry for the largest member
count its unit can still reach, and a different-unit skill by the largest
number of units it can still count (pruning-proof.md, Section 20).  Let S be
the sum of the selected cards' ceilings, L the largest of them, G the global
largest skill_max and r the remaining slots.  Every node satisfies

    UB = 2*(S + r*G) + 8*max(L, G),

which permits card reuse, ignores character uniqueness and fixed slots, and
chooses the best possible leader, so it can only overestimate a real
completion.  Nodes whose remaining slots are all free also use the
per-character frontier of those ceilings.  Randomized exhaustive tests
additionally cover unit-count, different-unit and reference skills, all
reference strategies, cultivation variants sharing a public game id, and fixed
card/character constraints.

## 8. Challenge Live

Challenge search restricts the feasible set to five cards of one character.
The specialized search uses admissible suffix bounds and exact leaf evaluation.
Challenge-all runs the exact per-character solver independently and merges the
per-character Top-K lists.  A global Top-K member must occur in its own
character's Top-K list, so this merge is exact.

## 9. Exact bonus tiers

Tier identity and leaf evaluation use exact tenths (`x10`), not the rounded
half-percent ranking key. Each requested tier keeps its own canonical Top-K;
the grouped branch and bound and its reachability and live-score pruning are
proved in pruning-proof Section 17.

The candidate-pool over-target removal described in section 1 is independently
safe by non-negativity and its unavoidable per-card lower bound. Each per-tier
canonical Top-K is independently compared with full enumeration in the test
suite.

## 10. Seed / incumbent generation

Warm starts, Final Chapter group seeds, leader-key ranking and local improvement are
not part of the feasible-set proof. They may only:

- choose traversal order; or
- contribute exactly evaluated incumbents that raise the pruning threshold.

A seed is canonicalized and legality-checked before it can enter the tracker.
No production branch may use seed membership or a Top-K seed
buffer as a feasibility predicate.

The unlimited seed-on / seed-off property test is the direct regression for
this separation: disabling Final seeds must leave the complete canonical result
sequence unchanged. Performance measurements may differ, but the proof
frontier and its admissibility do not depend on seed success.

## 11. Completion, timeout and request-scoped budget

`SearchBudget` is created once per public operation and is passed through
preparation-sensitive search phases, warm starts, specialized solvers and
alternative reconstruction. Challenge-all shares one budget across all
characters; it does not start a fresh timeout for each character.

At the low-level solver contract (`SearchParams`), `timeout_ms = 0` means
unlimited and that path does not read the clock. The public JSON/build-parameter
surface used by engine / CLI / WASM validates `timeout_ms` in `1..=300_000`, so
those public JSON calls are exact-with-timeout rather than an unbounded request.
This input-range distinction does not create a second completion rule: timed
searches still use sampled checks in hot loops and exact checks at phase / job
boundaries, and once expiry is observed the shared budget's hit flag is sticky.

`SearchStats.deadline_hit` is the single completion source of truth:

    deadline_hit == false  => SearchCompletion::Complete
    deadline_hit == true   => SearchCompletion::TimedOut

Server, CLI, engine and WASM surfaces propagate this solver-derived completion.
Elapsed wall-clock duration is diagnostic only and must never be used to infer
completion. In Challenge-all, a character whose turn begins after the shared
budget is already exhausted is reported TimedOut, not as an empty / infeasible
character.

A TimedOut result contains only legal, exactly evaluated incumbents, but neither
its cardinality nor ordering is a proof of canonical Top-K completeness.

## 12. Capacity and representation errors

The 512-bit metadata mask is available only when the entire pool fits. The
513-card HTTP regression now returns a complete result and agrees with the
engine entry point. A request whose hard-filtered candidate set exceeds the
65,535-card dense-index capacity fails explicitly with `TooManyCards`. The
engine must never trim the pool to make it fit.

Other packed metadata limits are checked before conversion as well. Distinct
special-skill tables are interned where identities are equal, but genuinely
unrepresentable metadata is an explicit error rather than a clamp, saturation
or silent alias.

Capacity errors are therefore outside the exact-result domain: no result set is
returned and no completeness claim is made.

## 13. Numeric precision invariants

Every proof predicate must use a representation at least as exact as the leaf
evaluator for the dimension it constrains.

For event bonus this means:

    evaluator bonus == tier-membership bonus == reachability-proof bonus

The implementation therefore does not use rounded display percentages,
integer-ceil approximations or per-card event-point proxies to decide exact tier
membership.

Final Chapter leader bonus is preserved in tenths through pool construction,
bounds and leaf evaluation. Dynamic skill upper bounds are derived from the
same resolved skill tables / semantics used by evaluation. A bound may relax
correlations upward, but it may not silently change numeric precision in a way
that can underestimate a legal completion.

The floating-point side of these invariants is proved in
[pruning-proof.md, Section 29](pruning-proof.md#29-numeric-admissibility): every
integer or fixed-point ceiling dominates the `f64` leaf evaluator after its
truncations, because the ceilings' integer grids are coarser than the
evaluator's accumulated rounding on the numeric domain that pool construction
enforces (`numeric_domain` in handler/capacity.rs). Inputs outside that domain
are rejected as capacity or configuration errors, like the representation
limits of Section 12.

## 14. SearchStats contract

The stable cross-solver work / termination fields are:

- `visited_nodes`
- `leaf_nodes`
- `bound_prunes`
- `feasibility_prunes`
- `dominance_prunes`
- `deadline_hit`

Phase diagnostics are recorded separately:

- `seed_states`
- `seed_leaves`
- `proof_leaves`
- `alternative_states`
- `alternative_leaves`
- `power_scenarios_completed`
- `leader_jobs`

Compatibility counters such as `ub_prunes`, `leader_prunes`,
`correlated_prunes` and event-point counters remain useful for profiling, but
their units are solver-specific and must not be presented as a single
cross-solver node metric.

`SearchOutcome::new` finalizes the counters before exposure, and completion is
derived from the finalized stats rather than duplicated in mutable search
state.

## 15. Verification evidence and what each class proves

The runnable matrix and evidence boundaries are documented in
[search-validation.md](search-validation.md). Final Top-K reconstruction must
keep a forced leader in the concrete first slot: event bonuses and support
selection use that slot before display materialization. Reordering a returned
display cannot repair an incorrectly evaluated leader role.

The mathematical invariants in this document are the correctness argument.
Tests and benchmarks have different evidentiary roles and must not be conflated:

- **formal invariant / code contract**: establishes why a pruning, dominance,
  dispatch, completion or capacity rule is admissible;
- **exhaustive small oracle**: independently enumerates a bounded feasible set
  and compares the complete canonical Top-K;
- **randomized property**: searches deterministic generated boxes for violations
  of the invariant or oracle equivalence;
- **regression fixture**: permanently preserves a previously demonstrated
  counterexample, including the historical `case7` whose old oracle was itself
  incomplete;
- **performance A/B**: measures cost and may additionally assert result /
  traversal identity for an optimization, but timing data is never a proof of
  exactness.

Current correctness gates include the full Rust unit / integration / doctest
suite; deterministic randomized Power, Skill, ordinary, World Bloom and Final
comparisons; special-skill and fractional-bonus checks; canonical Top-K and
same-game-ID variant regressions; completion and capacity regressions; and the
ignored all-scene release matrix.

Release acceptance additionally exercises the same locked candidate through
native, server, MSRV, wasm32, generated wasm-bindgen bindings, Node and a real
browser runtime. Cross-runtime checks compare canonical results and completion
semantics, including a solver-observed timeout. Repeated browser calls are a
runtime / stack-stability gate. These checks can detect ABI, clock, serialization
or platform regressions, but they are empirical release evidence rather than a
substitute for the admissibility and completeness arguments above.

The all-scene matrix covers Solo, Auto, Multi, Cheerful, MySekai, Power DP,
constrained Power, Power minimization, Skill, World Bloom, Final Chapter
fixed/auto leader, exact bonus tiers, Challenge and ChallengeAuto over hundreds
of deterministic small boxes. The current release gate is 256 cases × 15
comparisons = 3840 checks.

The permanent `case7` fixture is especially important: the historical
production result was 285069864403781, the historical oracle reported
276690383207334, while the independently established optimum is
288690521835152. The lesson is part of the correctness argument: agreement with
a heuristic or incomplete oracle is not evidence of exactness.
