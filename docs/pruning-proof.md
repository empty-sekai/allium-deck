# Formal proof of pruning correctness

This document proves the correctness of every mechanism in the current search
implementation that can remove a candidate, a partial state, or an entire
subtree before exact leaf evaluation.

It is a mathematical proof tied to the implementation invariants. It is not a
machine-checked Lean/Coq development. Tests and exhaustive oracles are listed
at the end as independent attempts to falsify the assumptions used by the
proofs; they are not substitutes for the proofs themselves.

The end-to-end solver argument, canonical result ordering, timeout semantics,
and representation limits are described separately in
[exactness-proof.md](exactness-proof.md).

## 1. Definitions

Let a search node be $N$, and let $F(N)$ be the set of legal complete decks
that extend $N$.

For a complete deck $D$, let $v(D)$ be the primary numeric objective used by
the current search:

- the packed event-point/live-score key for event Score;
- live score for no-event Score;
- MySekai score;
- total Power;
- encoded Skill value;
- the per-tier live-score key for exact Bonus tiers.

The public result order contains additional deterministic tie-break fields.
Therefore equal values of $v$ are **not interchangeable**.

For a maximizing Top-K search whose tracker already contains K distinct public
card sets, let $\tau$ be the K-th primary objective value. For minimizing
Power, $\tau$ is the K-th value in the reversed numeric order.

### Definition 1 — admissible upper bound

A function $U$ is an admissible upper bound at node $N$ iff

$$
\forall D \in F(N),\quad v(D) \le U(N).
$$

### Definition 2 — admissible lower bound

For a minimizing search, $L$ is an admissible lower bound iff

$$
\forall D \in F(N),\quad L(N) \le v(D).
$$

### Theorem 1 — threshold pruning

For maximization, if $U(N) < \tau$, no completion of $N$ can enter the
current canonical Top-K. The subtree may be discarded.

For minimization, if $L(N) > \tau$, the subtree may be discarded.

**Proof.** In the maximizing case every $D \in F(N)$ satisfies
$v(D) \le U(N) < \tau$. At least K already-known distinct public sets have
primary objective at least $\tau$, so no $D$ can displace them. The
minimizing case is the order dual. ∎

### Corollary 1 — equality must survive

A branch with $U(N)=\tau$ cannot be discarded from the numeric bound alone.
It may contain a distinct public card set, a better legal placement, or a better
deterministic cultivation representative with the same primary objective.

This is why all maximizing comparisons in the exact path use strict
upper < threshold, and why the SIMD candidate mask retains
upper >= threshold.

### Lemma 1 — tighter admissible bounds stay admissible

If $U_1,\ldots,U_m$ are admissible upper bounds, then

$$
U(N)=\min_i U_i(N)
$$

is also admissible.

This justifies intersecting the generic suffix bound with dense-suffix,
correlated, World Bloom, or joint event-point bounds.

### Arithmetic invariant — rounding must be outward

A mathematically valid real-valued upper bound is useful only if its machine
representation does not round below the real bound. Throughout the search code:

- exact discrete state such as card power, x10 bonus, character masks, and
  public ids remains integer;
- conversion of a real-valued quantity into an integer upper bound uses
  ceiling/outward rounding;
- support sums used as upper bounds are rounded with ceil;
- integer division in an upper-bound expression uses div_ceil / ceil division;
- the correlated bound upper-rounds its prepared coefficients and adds an
  explicit final safety unit;
- a lower bound used for minimizing Power is built from actual per-card minima,
  so ordinary integer arithmetic cannot round it upward.

If a specialized bound cannot establish its required domain or arithmetic
assumptions, its constructor returns None and the search falls back to a looser
already-proved bound. Disabling a tighter bound cannot remove a legal branch.

The end-to-end precision invariants, including x10 leader/event bonus handling,
are also recorded in [exactness-proof.md](exactness-proof.md).

## 2. What is and is not pruning

The code contains three different kinds of early rejection. They must not be
confused.

1. **Feasibility rejection.** A partial assignment violates the requested
   problem itself: duplicate public card id, forbidden character, fixed-slot
   mismatch, character uniqueness, or too few remaining candidates. Removing
   it is exact by definition of the feasible set.
2. **Proof-based pruning.** A legal partial state is omitted because a theorem
   proves that no completion can enter Top-K. These are the mechanisms proved
   below.
3. **Ordering only.** Warm starts, one-swap neighborhoods, beams, candidate
   ranking, correlated-plane selection, and Final ranked buffers choose which
   nodes are visited first or seed an incumbent. They do not remove an
   otherwise unproved branch.

A deadline is also not an exact pruning rule. Observing a deadline changes the
reported completion state to TimedOut; no completeness theorem is claimed for
that run.

## 3. Candidate-set reductions before search

### 3.1 Hard filters

handler may remove a card because the request explicitly excludes it or because
it violates a hard unit, attribute, challenge-character, or cultivation
constraint.

These cards are not members of the requested feasible set, so this is
feasibility reduction rather than objective pruning.

Fixed cards and fixed characters are not treated as quality hints. Their role
constraints are carried into placement-aware search.

### 3.2 Exact-bonus over-target reduction

For exact bonus-tier queries, all event-bonus contributions are non-negative.
The builder computes an **unavoidable** contribution $b(c)$ for each card:

- base bonus when limited bonus may be omitted by a count cap;
- total bonus only when the limited part is unconditional.

Let $T_{\max}$ be the highest requested exact tier. If

$$
b(c) > T_{\max},
$$

every deck containing $c$ has total bonus greater than every requested tier.
Therefore $c$ occurs in no feasible exact-tier solution and can be removed.

This reduction does not use rounded display bonus.

### 3.3 Capacity is not truncation

The metadata mask remains 8 × 64 = 512 card bits. Pools beyond that width keep
all candidates in the SoA columns, and the mask accessors return `None` rather
than exposing a partial set. If hard filtering leaves more than 65,535 cards,
the dense `CardIdx` cannot represent the pool and the builder returns
`TooManyCards`. Other compact metadata limits are checked in the same way.

There is deliberately no theorem that “the best 512 are enough”; no such
truncation exists in the exact path.

Implementation:
src/handler/build.rs, src/handler/filter.rs, src/handler/capacity.rs.

## 4. Same-character dominance

Implementation: src/search/dominance.rs.

For two cards $A$ and $B$ of the same character, the search may delete
$B$ only when $A$ is a certified substitute for $B$.

The implementation requires all of the following:

1. every one of the eight precomputed power contexts of $A$ is at least the
   corresponding value of $B$;
2. skill type and all completion-observable skill semantics are substitutable,
   including skill_min, skill_max, unit-count / different-unit / reference
   tables;
3. required training-state semantics are equal;
4. base and limited event bonus of $A$ are componentwise no worse;
5. limited-bonus zero/nonzero class is preserved when the event has a count cap;
6. attributes are equal;
7. unit membership masks are equal, preserving both formation-power contexts
   and other cards' unit-count/different-unit skill state;
8. fixed public card identities are never deleted;
9. Final leader-only bonus dimensions are not worsened when relevant;
10. the pair $(game\_id,dense\_index)$ of $A$ is not canonically later than
    that of $B$ when the numeric objective can tie.

Position-sensitive searches disable the exchangeable-slot dominance pass.

### Theorem 2 — ordinary substitution

For every legal completion $D$ containing $B$, replacing $B$ by $A$
produces a legal completion $D'$ with

$$
v(D') \ge v(D),
$$

and if equality holds, $D'$ is not canonically worse solely because of the
replacement.

**Proof.** The two cards have the same character, attribute, and unit mask, so
the replacement preserves character feasibility, attribute-state transitions,
unit-membership predicates, and the skill state of the other four cards. Every
power context, resolved-skill bound, and bonus component used by the evaluator
is componentwise no smaller for $A$. The objective formulas are monotone in
those non-negative components over the supported domain. The explicit
canonical-id check prevents a numerically equal replacement from moving
backward in the deterministic tie order. ∎

## 5. World Bloom support-aware dominance

A World Bloom main-deck card may also occupy a support-deck position. Replacing
a main card can therefore change support opportunity cost.

SupportDimension records outward-rounded support values for every public card
identity and cultivation variant. Let $q$ be the number of counted support
slots. After removing the other four main cards, let $t$ be the value that
would refill the $q$-th support position.

For support values $a$ and $b$ of replacement $A$ and removed card $B$,

$$
(a-t)^+-(b-t)^+
\le \max(0,\,a-\max(b,f)),
$$

where $f$ is the globally safe replacement floor at support rank $q+5$.
Removing at most five main cards cannot make the refill value worse than this
floor.

The code rounds $a$ upward and $b,f$ downward before computing this maximum,
so the resulting support deficit is conservative.

The replacement is accepted only when $A$'s guaranteed **base** event-bonus
surplus pays this worst-case deficit. Limited bonus is not used as payment
because a count cap may omit it.

### Theorem 3 — World Bloom substitution

When support_deficit_affordable succeeds, the combined
main-deck-bonus + support-deck contribution after replacing $B$ with $A$
cannot decrease. Together with Theorem 2, $A$ is a valid substitute for every
legal completion containing $B$. ∎

## 6. Top-K recovery after dominance

Dominance alone is sufficient for Top-1 but not for Top-K: a deck containing a
dominated card may be a legitimate runner-up public card set.

Implementation: src/search/alternatives.rs.

Let $D$ be any true Top-K deck containing one or more dominated cards. Replace
each dominated card by its surviving dominance root, obtaining $R(D)$.
By Theorems 2 and 3,

$$
v(R(D)) \ge v(D),
$$

and an equal primary objective is not canonically worsened by the replacement.

If $D$ is in the true Top-K, $v(D)\ge\tau^*$, where $\tau^*$ is the true
K-th primary value. Therefore $R(D)$ is at or above the same threshold and
must be represented among the compacted search's K best public sets, modulo
deduplication of roots.

The post-search alternatives pass recursively substitutes every recorded
inverse dominance edge back into every slot and exact-evaluates the result.
Multiple simultaneous substitutions are covered by recursion. Final Chapter
also rotates legal leaders before member substitution so that a root that was
canonical only as leader can still be recovered in a member slot.

### Lemma 2 — pruning inside alternative expansion

Inverse substitution can only move from a dominance root to a card it
dominates. Its primary score is therefore monotone non-increasing down the
alternative tree. If the current node already has

$$
v(N)<\tau,
$$

every descendant is also below $\tau$, so the expansion subtree may be
discarded. Equality is retained for tie-break recovery.

## 7. Character-aware suffix maxima

Implementation: src/search/suffix.rs.

For a numeric component $x$ (power, skill, or bonus), define

$$
m_c^x = \max\{x(card): card \text{ belongs to unused character }c\}.
$$

If $r$ slots remain and the mode enforces character uniqueness, any legal
completion chooses at most one card from each character. Therefore

$$
\sum_{card\in completion}x(card)
\le \sum_{i=1}^{r} \operatorname{Top}_i\{m_c^x\}.
$$

The suffix tables store exactly this top-r character relaxation for power,
skill, and bonus. The maximizing cards may be different in each dimension;
allowing those incompatible choices only increases the bound.

For Skill, the leader component is additionally bounded by the maximum of the
best already-selected skill and the best remaining per-card skill.

The evaluator's supported Score / event-point / MySekai formulas are monotone
in the non-negative relaxed inputs used here. Substituting these independent
maxima therefore yields an admissible objective upper bound.

This proves SuffixBound::upper_bound_for_slots and the generic
SuffixBound::ceiling prune under Theorem 1.

## 8. Exclusion deltas and dense suffix tails

The global character relaxation can be tightened after tentatively selecting
character $c$.

suffix_compact_u32/u16 stores:

- the sum of the current top-r unused character maxima;
- which characters supplied those maxima;
- for each selected character, the exact loss after replacing it by the next
  unused maximum.

Subtracting power_delta(c), skill_delta(c), or bonus_delta(c) therefore
computes the same relaxation with $c$ excluded; it does not subtract more
than the amount contributed by the old relaxation.

Dense suffix tables repeat the same construction over the set
cards[dense..]. If $j>i$, then

$$
cards[j..] \subseteq cards[i..].
$$

The maximum of any relaxation over the smaller set cannot be larger. Thus the
dense-suffix ceiling is monotone non-increasing with the scan index.

### Corollary 2 — monotone break

When a dense-suffix ceiling at scan position $i$ is below $\tau$, all later
positions $j>i$ are also below $\tau$, so the whole remainder of that loop
may break, not merely continue.

Fixed-role slots deliberately restart their free-card frontier at zero; the
monotone argument is never applied across a fixed slot that would otherwise
hide earlier legal cards.

## 9. Monotone Power / Skill candidate breaks

For the general monotone path, the pool ordering is descending by power_max for
Power and skill_max for Skill. At a fixed recursion layer the precomputed
relaxation for the other $r-1$ slots is constant.

Consequently the candidate-specific ceiling is non-increasing as the current
candidate advances through that sorted layer. Once it falls below $\tau$, all
later candidates have an equal or smaller first-card component and the same
relaxed tail, so recurse_monotonic may break.

The modern unconstrained Power fast path and constrained Power / Skill solver
have stronger specialized proofs in Sections 19–21; this lemma covers the
generic monotone implementation wherever it is exercised.

## 10. No-event Score numerator pruning

For no-event Score, the public packed objective is

$$
(live\_score, live\_score),
$$

so ordering by the packed value is exactly ordering by live_score.

The upper-bound calculation has a non-negative integer numerator $N$ and
fixed denominator

$$
D=1{,}000{,}000.
$$

For an integer live threshold $T$,

$$
\left\lfloor\frac ND\right\rfloor < T
\iff N < TD.
$$

Therefore comparing the pre-division upper numerator directly with
threshold * 1,000,000 is algebraically identical to computing the floored
live-score upper bound first. It removes a hot integer division but does not
change a single pruning decision.

Implementation:
score_noevent_live_numerator_ceiling,
score_noevent_threshold_numerator,
recurse_score_noevent_monotonic.

## 11. Correlated no-event Score bound

Implementation: src/search/correlated.rs.

This bound is enabled only for supported no-event Solo / Auto Average contexts.

Let $P$ be deck power, $S$ the sum of the five member skill values, and
$L$ the leader skill. The implementation prepares the following outward-rounded
non-negative constants:

- $Q=10^{12}$, the fixed coefficient scale;
- $R=256$, the power/skill balancing factor used by the linear envelope;
- $C=\lceil base\_score\cdot Q\rceil+1$;
- $B=\lceil (\sum_{i=1}^{5} rate_i/500)\cdot Q\rceil+1$;
- $D=\lceil (rate_{leader}/100)\cdot Q\rceil+1$.

The supported no-event live-score formula is therefore bounded by

$$
F(P,S,L) \le \frac{4P(C+BS+DL)}{Q}.
$$

For any positive plane slope $\lambda$, define
$X=BS+DL$ and $H=BR$. The implementation forms the linear relaxation

$$
HP+\lambda X \le T_\lambda .
$$

Each candidate contributes separately to a per-character **member** term and a
per-character **leader** term. The role envelope takes the best $k$ unused
member terms, then also considers replacing exactly one chosen member term by
that character's leader term. Hence it covers both cases: the leader is already
in the prefix, or one remaining character becomes leader. A character is never
counted in both roles.

Thus every legal completion satisfies

$$
X \le \frac{T_\lambda-HP}{\lambda}
$$

for $0\le P\le T_\lambda/H$. Substituting this into the score bound gives

$$
F(P) \le
\frac{4P(\lambda C+T_\lambda-HP)}{\lambda Q}.
$$

This is a concave quadratic in $P$. Put
$A=\lambda C+T_\lambda$. Its unconstrained vertex is

$$
P^*=\frac{A}{2H}.
$$

If $A<2T_\lambda$, the vertex lies inside the feasible interval and the
maximum is

$$
F \le \frac{A^2}{\lambda H Q}.
$$

If $A\ge2T_\lambda$, the vertex lies at or beyond
$T_\lambda/H$, so the maximum over the feasible interval is attained at the
right boundary:

$$
F \le \frac{4CT_\lambda}{H Q}.
$$

These are exactly the two branches of **quadratic** in
src/search/correlated.rs. Coefficient preparation uses ceilings, the final
division is rounded upward, and the implementation adds one further integer
safety unit for bounded floating-point preparation error.

Hence each correlated plane is admissible. By Lemma 1, taking the minimum of
several planes and the ordinary suffix bound remains admissible.

The auto plane selector only decides which already-admissible planes are worth
evaluating. Discarding a *bound plane* cannot discard a search branch; it only
makes the remaining bound looser.

## 12. Event-score independent bound

For event Score, the generic suffix first obtains admissible maxima for power,
total event bonus, total skill, and leader skill.

The live-score and event-point formulas are monotone in these non-negative
inputs over the supported domains. This includes the capped teammate-score term
in Multi/Cheerful: replacing a quantity by a larger value cannot lower the cap
result.

Therefore evaluating the event formula at independently relaxed component
maxima cannot underestimate a legal completion.

Free-role traversal does not prematurely enforce a limited-bonus count cap.
For pruning, partial_bonus_add counts every selected limited amount; the exact
evaluator applies the event cap only at a leaf. Because limited contributions
are non-negative, ignoring the cap is an upper relaxation. Final Chapter,
whose bound explicitly models the cap, instead keeps the largest remaining
limited values up to the available count.

Leader-only and extra-event contributions entering this generic bound are
likewise represented by exact values or outward-rounded upper values.

The encoded objective uses event point in the high 32 bits and live score in
the low 32 bits. The resulting packed ceiling can therefore be compared
directly with the tracker's numeric threshold.

## 13. Joint Multi event-point power/bonus bound

Implementation:
dense_candidate_joint_ceiling_multi_score_event,
build_joint_ep_table, joint_event_point_upper,
maximize_joint_event_numerator in src/search/suffix.rs.

For $w\in\{512,1024\}$, dense suffix preprocessing provides

$$
P + wB \le S_w
$$

for every legal completion, where $P$ is power and $B$ event bonus. The suffix
table itself is built by the same top-per-character relaxation as Section 7, but
on the additive quantity $power+w\cdot bonus$, so this inequality is
admissible.

For a fixed bonus $b$, power is therefore relaxed to

$$
P(b) \le \min(P_{\max},\,S_w-wb).
$$

After the independent skill/leader relaxation is folded into the Multi
coefficients, each supported event-point numerator evaluated by
**maximize_joint_event_numerator** has the form

$$
E(b)=\bigl(A+K\min(P_{\max},S_w-wb)\bigr)(b+100),
$$

where $A\ge0$ is the power-independent constant and $K\ge0$ is the live-score
power coefficient for that event formula.

There are only two regions:

1. while $P_{\max}\le S_w-wb$, the min is constant and $E(b)$ is linear;
2. afterwards,
   $E(b)=(A+KS_w-Kwb)(b+100)$, a concave quadratic because the coefficient of
   $b^2$ is $-Kw\le0$.

Therefore the integer maximum on the valid bonus interval can occur only at an
interval endpoint, at the two integer points around the region transition, or
at an integer adjacent to the quadratic vertex. The code evaluates the global
endpoints, **flat_end** / **flat_end + 1**, and
**vertex - 1, vertex, vertex + 1**, exactly covering those cases.

The enclosing **joint_event_point_upper** constructs both capped and
uncapped/fixed-opponent forms with independently relaxed skill and leader
coefficients, applies upward integer division, and where two formula-derived
upper bounds describe the same true event value takes their minimum. That last
step is safe by Lemma 1.

Therefore each $w$ produces an admissible event-point upper bound.

The table key uses div_ceil on the support value, so table lookup rounds to a
not-smaller support bucket. Finally, taking the minimum of the $w=512$ and
$w=1024$ upper bounds is safe by Lemma 1.

## 14. SIMD event candidate mask

Implementation: src/simd.rs and recurse_ep_multi_shadow.

The scalar rule for a candidate with upper bound $u_i$ is

$$
u_i \ge \tau \quad\Longleftrightarrow\quad \text{candidate survives}.
$$

upper_bound_mask_16_scalar and the AVX-512 implementation both return exactly
one bit per lane satisfying unsigned >= threshold.

Thus SIMD changes only how 16 scalar comparisons are evaluated. It does not
change the predicate. In particular, equality is retained as required by
Corollary 1.

The final partial AVX-512 block builds character-legality bits lane by lane
rather than pretending all tail lanes are legal.

## 15. World Bloom attribute matching bound

World Bloom different-attribute bonus is not additive per card.

For the remaining suffix, form a bipartite graph:

- left vertices: attributes not yet represented by the prefix;
- right vertices: unused characters;
- edge $(a,c)$: the suffix contains a card of attribute $a$ for character
  $c$.

Every legal completion that adds $q$ new attributes induces a matching of
size $q$: each new attribute is supplied by a selected card, and character
uniqueness makes their characters distinct.

Therefore

$$
q \le \nu(G),
$$

where $\nu(G)$ is the maximum bipartite matching size.

The bound maximizes diff_attr_bonus over all attribute counts reachable up to
that matching size. This is an upper bound on the extra attribute bonus of any
legal completion.

Implementation:
reachable_novel_attr_ub and augment_attr_matching in
src/search/suffix.rs.

## 16. World Bloom support-deck upper bound

Support entries are stored in non-increasing bonus order.

For a selected main-deck prefix, the implementation computes the support sum
after excluding those main-card identities and filling the counted support
slots with the next available entries.

Adding another main-deck card can only leave the counted support prefix
unchanged, or remove a currently counted support card and replace it with an
entry no larger than the removed one.

Hence support bonus is monotone non-increasing as the main-deck prefix grows.
The current support sum is therefore an upper bound for every completion.

The same argument proves the leader-only Final root support ceiling.

Generic Final DFS may visit several leader profiles. Its suffix preparation
forms one support envelope: each public card receives the maximum bonus across
the feasible leader profiles, and the slot count is their maximum count.
After any selected-ID exclusion, every counted card in any one profile is
still bounded by that card's envelope value. The envelope has enough slots to
include all of them, so its largest remaining values upper-bound that profile's
sum. This holds for every leader, including profiles that fall back to the
ordinary support deck. A fixed leader needs only its effective profile. The
global extra-bonus fallback and exclusion-aware bounds use the same envelope.

Combined World Bloom ceilings may independently maximize power, skill,
attribute bonus, and support bonus. Incompatibility between these maxima only
makes the bound larger, never smaller.

## 17. Exact bonus-tier reachability

Implementation: src/search/bonus_reach.rs and the Bonus tracker in
src/search/dfs.rs.

In the additive bonus model, every candidate has an exact integer bonus in
0.1% units. BonusReach[pos][r] is constructed by the recurrence

$$
R_{pos,r}
= R_{pos+1,r}
\cup
\{x+b_{pos}:x\in R_{pos+1,r-1}\}.
$$

By induction on pos, this is exactly the set of sums obtainable by choosing
$r$ cards from the raw suffix.

The DP intentionally ignores character uniqueness and other deck constraints,
so its reachable set is a **superset** of legal-completion sums. Therefore, if
no relaxed sum lies in the exact interval still needed for a requested tier, no
legal completion can hit that tier and the bucket is safely pruned.

When limited-count, World Bloom, or Final semantics make raw card bonus
non-additive, the implementation disables this reachability proof.

In the additive model the already-selected exact bonus is accumulated as
bonus_x10. floor(bonus_x10 / 5) is therefore a conservative lower bound in the
tracker's half-percent x2 coordinate, while the high 32 bits of the generic
suffix ceiling are an admissible upper bound. A requested tier outside that
closed interval is unreachable.

For an empty tier bucket, the subset-sum test above is additionally required
before the whole subtree may be declared irrelevant. For a bucket that already
contains K results, the remaining question is only whether the branch can beat
that bucket's K-th **live-score** value. The branch therefore compares the low
32-bit live ceiling with the low 32-bit bucket threshold, not with the full
bonus<<32 | live encoded key. Equal-live branches remain searchable for
canonical tie resolution.

## 18. Final Chapter bounds

Implementation: src/search/solver/final_chapter.rs.

### 18.1 Leader jobs

Every surviving leader card becomes a search job. There is no heuristic
per-character leader cap.

A leader job is skipped only if character_ceiling(...) < threshold.

The warm auto-leader beam seeds incumbents only; it does not define the job
set.

### 18.2 Exact attribute-union DP

For each character group, attr_mask records every attribute available from that
character.

attr_bonus[k][s] stores the maximum `diff_attr_bonus` obtainable by selecting
exactly $k$ groups from a suffix, starting from attribute union `s`. Its
transition keeps the skip-current-group value and, for every attribute in the
current group's mask, takes the value for `s | attribute` in the `k-1` row.
This is the OR-product DP with the final bonus lookup memoized. Induction on
suffix length proves that the table is the exact maximum for the isolated
attribute dimension, including nonmonotone `diff_attr_bonus` tables.

Combining the selected-prefix union and leader attribute into `s`, then looking
up `attr_bonus[remaining][s]`, is therefore an exact maximum for that dimension.

### 18.3 Character-level numeric ceiling

For the remaining character groups, the suffix independently takes the largest
possible per-group power, skill, base bonus, and limited bonus.

For limited bonus, only the largest values up to the remaining
card_bonus_count_limit are admitted. Any legal completion can contribute no
more than this top-cap sum.

These maxima may come from mutually incompatible card choices inside a group.
That is a relaxation, so their combination can only overestimate.

Adding the attribute-union maximum and the support upper bound from Section 16
therefore yields an admissible character_ceiling.

### 18.4 Character-loop break

Character groups are sorted by a descending group key, but the correctness of
the break does not depend on that heuristic key. character_ceiling at position
i reads GroupCeilingTail[i], which was built from the exact suffix
groups[i..]. Moving to i+1 removes a group from that suffix. Every per-component
top list and every reachable attribute-union set can therefore only stay equal
or shrink.

Thus character_ceiling is non-increasing with the start index. Once it is below
the threshold, the rest of the group loop can safely break.

### 18.5 Card-level plan

After four member characters are fixed, CardGroupPlan stores suffix sums of
each selected group's own best power, skill, base bonus, and sorted limited
bonuses.

At card depth $d$, every legal remaining card from group $g$ is
componentwise bounded by that group's stored maxima. Summing the remaining
group maxima and merging the top limited values therefore bounds every
card-level completion.

If every positive rounded limited contribution equals $v$, the top-$r$ sum
is exactly $\min(S, rv)$, where $S$ is the sum of all contributions. The
card plan detects this property over the complete pool and caches suffix
sums. This replaces only the calculation of the same bound; mixed rounded
amounts retain the sorted merge. The zero-only pool has $v=0$.

selected_card_ceiling_with_candidate_support_ub keeps the current support
total as an optimistic bound. A surviving candidate then updates its exact
support displacement before a second bound is checked. The updated support
value still bounds all later extensions by Section 16.

### 18.6 Ranked card buffer and its monotone break

The fixed-size RANKED_CAP buffer only reorders the first candidates by their
already-proved upper bound. Insertion keeps this buffer in non-increasing upper
bound order.

During exploration the tracker threshold can only stay equal or increase. If
the next ranked entry has upper bound (U<	au), every later ranked entry has
upper bound at most (U) and is also below the current (or any future) threshold.
The ranked-loop break is therefore an instance of Theorem 1 plus sorted
monotonicity.

Cards beyond the buffer are **not truncated**: they are still iterated and
tested individually against the same admissible candidate and partial bounds.
Therefore RANKED_CAP changes visit order only and never removes an unproved
candidate.

### 18.7 Final member-only dominance

Final Chapter applies an additional dominance relation to cards used in member
slots. The comparison is performed in a member context that removes
leader-only numeric benefits but preserves the card's real skill/training
semantics and World Bloom support opportunity cost.

Therefore Theorems 2 and 3 apply to **member positions**: replacing a dominated
member by its root cannot worsen a completion in any dimension visible to that
member role. The relation is never used to assert that the dominated card is
also safe to remove as a leader.

For Top-K recovery, member alternatives are substituted only into member slots.
Because the canonical result for a root set may keep that root in the leader
slot, Final additionally rotates every legal leader position, exact-evaluates
the rotation, and then performs member substitution. This makes the member-only
dominance reduction complete for Top-K without assuming leader exchangeability.

The Top-1 leader-specific variant computes the same member relation against the
actual leader's support profile; this is a tightening of the support dimension,
not a change in the substitution theorem.

## 19. Constrained Power bounds

Implementation: src/search/solver/numeric.rs.

For maximizing Power, let:

- $P$ be the sum of power_max for selected cards;
- $M=\max_c power\_max(c)$;
- $r$ be the remaining slots;
- $H$ be honor power.

Every completion satisfies

$$
Power(D)
\le \operatorname{clamp}(P+rM+H).
$$

This bound even permits reuse of the same global maximum card and ignores
character/fixed-slot restrictions, so it is a relaxation.

For minimizing Power, let power_min(c) be the minimum of card $c$'s eight
precomputed power contexts and let

$$
m=\min_c power\_min(c).
$$

If $P_{\min}$ is the selected-card minimum sum, every completion satisfies

$$
Power(D)
\ge \operatorname{clamp}(P_{\min}+rm+H).
$$

This deliberately allows each future slot to use the global minimum, so it
cannot exceed the true minimum completion. The subtree is pruned only when this
lower bound is strictly worse than the K-th minimizing threshold.

## 20. Skill upper bound

The encoded Skill objective is

$$
10L + 2\sum_{\text{other 4}} s_i
= 2\sum_{i=1}^5 s_i + 8L.
$$

For every card, skill_max is an upper bound on its resolved value:

- ordinary score-up: exact maximum;
- unit-count: maximum table entry;
- different-unit: maximum base plus increments;
- reference skill: maximum allowed reference contribution.

Let $S$ be the selected skill_max sum,
$G=\max_c skill\_max(c)$, and $r$ remaining slots. Let $L_p$ be the
largest selected leader-capable value. Then

$$
U = 2(S+rG) + 8\max(L_p,G)
$$

permits card reuse, ignores character uniqueness, and chooses the best possible
leader independently. It is therefore an admissible upper bound.

## 21. Unconstrained Power: exact Top-K dynamic programming

Implementation: src/search/solver/power.rs.

The unconstrained maximizing Power case enumerates 49 deck-wide scenarios:
unit condition in {none, six units} crossed with attribute condition in
{none, six attributes}.

Every legal five-card deck induces at least one of these scenarios matching its
deck-wide unit/attribute conditions. In that matching scenario,
resolve_card_power_scenario gives exactly the per-card power contribution used
by the full evaluator. If more than one unit scenario is applicable, enumerating
all of them only duplicates coverage; it cannot omit the deck.

Within one scenario, each card has an exact additive power contribution and
characters are processed independently. Consequently a globally Top-K Power
deck must occur in the Top-K of at least one scenario that represents it:
otherwise that scenario alone would already contain K not-worse public sets.

### Lemma 3 — per-character choice truncation

For one character, choices are ordered by additive contribution, then canonical
public identity. Keeping the first K distinct public card ids is sufficient for
global Top-K.

**Proof.** A discarded choice has K same-character choices that are not worse.
For any fixed selection of the other four characters, replacing the discarded
choice by each of those K alternatives produces K distinct public sets with
objective no worse. Hence a final deck requiring the discarded choice cannot
rank above all K replacements. ∎

Cultivation variants sharing one public game_id are reduced to the
scenario-best representative before this K limit because they are one public
card identity, not K distinct results.

### Lemma 4 — partial-state Top-K truncation

After processing some character prefix, states are partitioned by selected-card
count. Future characters and their additive contributions are independent of
which cards formed a state in the processed prefix.

Suppose state $x$ is outside the best K states of one cardinality. For every
future extension $E$, the same extension is legal for each of the K states
that precede $x$, and

$$
value(y_i\cup E) \ge value(x\cup E).
$$

For equal additive values, inserting the same future public ids into two sorted
partial public-id sequences preserves their lexicographic order. Therefore the
canonical public-set tie order is extension-monotone as well.

Thus $x\cup E$ can never enter global Top-K. Keeping K states per cardinality
is exact.

The final concrete decks are still exact-evaluated and merged through the
common TopKTracker.

## 22. Challenge bound frontier

Implementation: src/search/solver/challenge.rs.

Challenge search fixes one character and chooses five distinct public card ids.

ChallengeBounds builds, for every suffix and remaining cardinality, a frontier
of triples

$$
(power,\ skill,\ leader).
$$

A frontier state $A$ dominates state $B$ iff every component of $A$ is at
least the corresponding component of $B$, with at least one strict
inequality.

The Challenge bound is used only for maximizing non-event Power/Skill/Score
cases. SuffixBound::ceiling is monotone in all three components there.
Therefore, for every prefix,

$$
ceiling(prefix+A) \ge ceiling(prefix+B).
$$

A dominated **bound state** can be deleted without deleting any actual card
set; it can never produce a larger ceiling than its dominator.

The branch ceiling is the maximum SuffixBound::ceiling over the surviving
frontier, so it upper-bounds every exact suffix choice. The branch is pruned
only when this maximum is strictly below threshold.

Minimizing Power, event Score, Bonus, and MySekai disable this frontier rather
than reuse a bound whose monotonic assumptions do not apply.

### Challenge-all merge

Challenge-all solves each character independently and merges each character's
Top-K list. If a global Top-K deck $D$ of character $c$ were outside
character $c$'s own Top-K, then character $c$ alone would already contain K
decks better than $D$, making $D$ impossible in the global Top-K. Thus the
per-character truncation before merge is exact.

## 23. Exact feasibility pruning

The following early exits are direct feasibility checks, not numeric
approximations:

- duplicate public game_id in one deck;
- repeated character where uniqueness is required;
- fixed-card mismatch;
- fixed-character mismatch;
- Challenge card from the wrong character;
- insufficient candidates remaining to fill all slots;
- placement that violates an explicit skill-order or role constraint.

Each condition is a clause of the requested feasible-set definition.
A completion below such a node does not exist.

## 24. Mechanisms that do not need a pruning proof

These mechanisms may strongly affect speed but do not remove a search branch:

- greedy warm start;
- one-swap local improvement;
- Final member beam used to seed incumbents;
- leader-key sorting;
- candidate sorting;
- correlated-plane auto selection;
- Final ranked candidate buffer;
- search of a tighter admissible bound before a looser one.

Their correctness requirement is simply that any seed inserted in the tracker
is a legal, exact-evaluated deck. Search completeness does not depend on seed
success.

## 25. Timeout

SearchBudget is shared across all phases of one operation. A deadline check may
stop traversal, but this is **not** claimed to preserve exactness.

Once expiry is observed, SearchStats.deadline_hit is sticky and the public
completion becomes TimedOut. Returned incumbents remain legal and exactly
evaluated; the solver makes no Top-K-completeness claim for that run.

Thus deadline handling is deliberately outside Theorem 1.

## 26. Proof-to-code inventory

| Mechanism | Main implementation | Safety result |
| --- | --- | --- |
| Hard request filters | handler/filter.rs, handler/build.rs | removes infeasible cards only |
| Exact-tier over-target filter | handler/build.rs | non-negative unavoidable bonus exceeds every requested tier |
| Capacity handling | handler/capacity.rs | explicit error; never truncates |
| Same-character dominance | search/dominance.rs | Theorems 2–3 |
| Dominance Top-K recovery | search/alternatives.rs | root mapping + exhaustive inverse substitution |
| Alternative-tree threshold | search/alternatives.rs | inverse substitutions are score non-increasing |
| Character suffix bound | search/suffix.rs | top-r per-character relaxation |
| Exclusion delta | search/suffix.rs | exact removal/replacement inside relaxed top-r set |
| Dense suffix break | search/dfs.rs, search/suffix.rs | nested suffix sets imply non-increasing ceiling |
| Sorted Power / Skill break | search/dfs.rs | descending candidate component + fixed relaxed tail |
| No-event numerator | search/dfs.rs, search/suffix.rs | exact floor/division equivalence |
| Correlated Score bound | search/correlated.rs | linear relaxation + concave quadratic envelope |
| Event independent bound | search/suffix.rs | monotonic formula over componentwise maxima |
| Joint Multi event bound | search/suffix.rs | P+wB relaxation + exact concave maximum candidates |
| SIMD threshold mask | simd.rs | vectorized scalar upper >= threshold |
| WL attribute matching | search/suffix.rs | every legal novel-attribute set induces a matching |
| WL support upper bound | search/suffix.rs, Final helpers | support can only stay or decrease as main deck grows |
| Bonus tier interval / BonusReach | search/bonus_reach.rs, search/dfs.rs | selected lower bound + suffix upper bound + relaxed subset-sum superset |
| Final member dominance | search/dominance.rs, search/alternatives.rs | member-role substitution + legal leader rotations |
| Final leader/job bound | solver/final_chapter.rs | admissible character ceiling |
| Final attribute DP | solver/final_chapter.rs | exact isolated OR-union DP |
| Final character-loop break | solver/final_chapter.rs | nested group suffixes imply non-increasing character ceiling |
| Final card-group bound | solver/final_chapter.rs | independent per-group maxima + limited top-cap + support UB |
| Final ranked-buffer break | solver/final_chapter.rs | candidates sorted by admissible UB; overflow candidates still visited |
| Numeric Power max/min | solver/numeric.rs | global max UB / global min LB |
| Numeric Skill | solver/numeric.rs | per-card skill-max relaxation |
| Power scenario DP | solver/power.rs | exhaustive scenarios + Lemmas 3–4 |
| Challenge bound frontier | solver/challenge.rs | exact skip/take relaxation + componentwise-dominated bound states |
| Challenge-all Top-K merge | solver/challenge.rs | a global Top-K deck must lie in its character's own Top-K |
| Feasibility pruning | DFS / numeric / Challenge / placement | exact request constraints |

This table is intended to be exhaustive for mechanisms in the current search
code that can remove legal-looking search work before a leaf is evaluated. If a
new early-return, candidate truncation, or bound-driven break is introduced, it
should either map to one of the rows above or add a new proof here.

## 27. Independent verification

The proofs above are the correctness argument. The repository also contains
independent checks designed to expose a violated premise.

| Proof area | Verification |
| --- | --- |
| Generic bounds / mode dispatch | property_matrix::long_exact_all_scene_property_matrix — 256 cases × 15 comparisons = 3840 checks |
| Correlated bound | search/correlated_audit.rs, property_bounds.rs, A/B with bound disabled |
| No-event numerator | exact_score.rs numerator threshold identities |
| Dominance | exact_dominance.rs, dominance_contract.rs, exact_world_bloom.rs |
| Top-K / ties | canonical_topk.rs, same-game-id cultivation regressions |
| BonusReach / exact tiers | exact_bonus.rs, fractional_bonus.rs |
| Final Chapter | exact_final_chapter.rs, role_constraints.rs, historical auto-leader counterexample |
| WL / Final cross-product | validation_oracle.rs, complete ordered Top-K with support profiles, constraints, variants and nonmonotone attributes |
| Power | exact_power.rs and all-scene oracle matrix |
| Challenge | exact_challenge.rs and challenge-all timeout regression |
| SIMD equality | simd::tests::dispatched_mask_keeps_bounds_equal_to_threshold |
| Historical incomplete oracle | case7_audit.rs |

The permanent case7 fixture is important evidence for the methodology:
agreement with another implementation, or even with an incomplete “oracle”, is
not a proof. The pruning argument must stand independently, and exhaustive
enumeration is used only where the state space is small enough to provide an
independent counterexample search.

## 28. End-to-end theorem

Assume:

1. pool construction succeeds without a capacity/representation error;
2. the request is within a solver mode covered above;
3. the shared search budget is not observed expired;
4. exact leaf evaluation and the canonical tracker implement the ordering
   described in [exactness-proof.md](exactness-proof.md).

Then every omitted feasible subtree is justified by one of Sections 3–23 and
cannot contain a result that should precede the final K-th result. Every
dominance-compressed public set that could belong to Top-K is reconstructed by
Section 6. Every specialized DP truncation is exact by Sections 21–22.

Therefore a search that ends with SearchCompletion::Complete returns exactly
the canonical Top-K of the full supported feasible set. ∎
