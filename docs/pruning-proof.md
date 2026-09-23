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
correlated, World Bloom, or composition-regime bounds.

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

Section 29 proves that these integer and fixed-point ceilings also dominate the
floating-point leaf evaluator after its truncations, on the numeric domain that
pool construction enforces.

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
3. **Ordering only.** Warm starts, one-swap neighborhoods, group seeds, candidate
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
Section 29 (Lemma N8) shows that the prepared coefficients alone already
dominate the floating-point evaluator.

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

## 13. Area-item composition regimes

Implementation: src/search/composition.rs (`search_regimes`, `RegimePlan`) and
`CardPool::restrict`.

For a deck $D$ let $A(D)$ be the attribute shared by all five cards, if any,
and $U(D)$ the set of units contained in all five cards. The evaluator resolves
card $c$ of $D$ to

$$
p_D(c)=\max_{w\in\mathrm{units}(c)} v_c\bigl(\pi_c(w),\,[w\in U(D)],\,[A(D)=\mathrm{attr}(c)]\bigr),
$$

where $v_c(\pi,u,a)$ is the stored power of profile $\pi$ under member key
$2u+a$. Since $A(D)=\mathrm{attr}(c)$ holds for every card exactly when $A(D)$
exists, the key of each unit depends only on $(w\in U(D),\ A(D)\text{ exists})$.

**Regimes.** The feasible decks are covered by 49 regimes:

| regime | condition on $D$ | admitted cards | member keys $K_R$ |
| --- | --- | --- | --- |
| Mixed | $A(D)$ none, $U(D)=\emptyset$ | all | $\{0\}$ |
| SharedAttr$(a)$ | $A(D)=a$, $U(D)=\emptyset$ | attribute $a$ | $\{1\}$ |
| SharedUnit$(u)$ | $A(D)$ none, $u\in U(D)$ | containing $u$ | $\{0,2\}$ |
| SharedUnitAttr$(u,a)$ | $A(D)=a$, $u\in U(D)$ | attribute $a$, containing $u$ | $\{1,3\}$ |

*Coverage.* Every deck satisfies at least one row: choose Mixed or
SharedAttr$(A(D))$ when $U(D)$ is empty, and otherwise any $u\in U(D)$ with
SharedUnit or SharedUnitAttr. Every card of a deck of a regime is admitted by
that regime.

*Per-regime power bound.* For a deck of regime $R$ every unit key that occurs
in $p_D(c)$ lies in $K_R$, hence

$$
p_D(c)\le b_R(c)=\max_{w\in\mathrm{units}(c)}\ \max_{k\in K_R} v_c(\pi_c(w),k).
$$

`RegimePlan::new` computes $b_R$ and `CardPool::restrict` builds the pool of
admitted cards with $b_R$ as their `power_max`; every other column, including
the eight exact power contexts, is copied unchanged. Every bound of Sections
4–12 and 14–18 uses `power_max` only as a per-card upper bound on resolved
power, so on the restricted pool each of them is admissible for every deck of
$R$. Decks outside $R$ may be visited and are then evaluated exactly; they are
never required to be found in $R$.

*Dominance inside a regime.* Dominance (Sections 4–6) runs on the restricted
pool. It removes a card only in favour of one with the same attribute and the
same unit membership, so a substitution leaves $A(D)$ and $U(D)$ unchanged and
keeps the deck inside $R$. Theorem 2 and the recovery of Section 6 therefore
apply verbatim to the feasible decks of $R$, with $b_R$ in place of
`power_max`.

*Regime ceiling.* For each regime the plan takes, per character, the largest
$b_R$, skill maximum and card-bonus ceiling among admitted cards, and adds an
admissible extra-bonus term: for World Bloom the best diversity bonus over the
attribute counts the admitted cards can reach (one when the regime fixes the
attribute) plus, over every support profile, the rounded-up sum of its first
`count` entries, which is the largest sum any exclusion can leave; otherwise
the context's extra-bonus bound; and for Final Chapter the largest leader
bonus. A deck uses five distinct characters, so each sum of five per-character
maxima bounds the corresponding deck sum, and the objective relaxation is
monotone in every argument (Sections 10 and 12). The resulting ceiling is admissible
for every deck of $R$. A plan is dropped when the regime admits fewer than five
characters or cannot satisfy a fixed card, fixed character or forced leader;
it then has no feasible deck.

**Shared tracker and external floor.** All regimes feed one canonical tracker
in original pool indices. `restrict` preserves the relative order of dense
indices, so remapping a regime result keeps every tie-break of the canonical
order (exactness-proof Section 2), and a deck found in several regimes is
deduplicated by public set with its best representative kept.

Let $f$ be the K-th primary objective held by that tracker. There are K
distinct legal public sets at least as good as $f$, so by Theorem 1 a deck
whose objective is strictly below $f$ cannot enter the global Top-K. Each
regime search therefore starts its own tracker with $f$ as an external floor:
its cutoff is the larger of its own K-th value and $f$, and the floor never
evicts a result. Regimes are visited in non-increasing ceiling order, and a
regime whose ceiling is strictly below the current cutoff is skipped: every
deck of it is below the K-th known value. Equality is never pruned.

**Incumbents.** Before the first regime, warm-start seeds are generated once
on the whole pool, canonicalized to their optimal legal placement and inserted
into the shared tracker. They are legal, exactly evaluated decks, so they only
raise $f$ (Section 24). Seeds whose cards are all admitted by a regime are also
passed to that regime's search in its dense indices; they are ordering hints
and are never needed for completeness.

Consequently every deck of the global canonical Top-K is found: it belongs to
some regime, that regime is either searched with admissible bounds or has a
ceiling below a value already reached by K known sets, and the shared tracker
keeps the canonical best of everything any regime returns.

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

A leader job is skipped only if character_ceiling(...) < threshold. Auto-leader
jobs run in non-increasing ceiling order, so the first job whose ceiling falls
below the threshold also bounds every later one.

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
- Final per-leader group seeds;
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
| Numeric domain | handler/capacity.rs, handler/validate.rs | Section 29: integer ceilings dominate the `f64` evaluator |
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
| Composition regimes | search/composition.rs, pool/card_pool.rs | per-regime member-key power bound, regime ceiling, shared tracker floor |
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
| Numeric admissibility | numeric_soundness.rs, handler/capacity.rs unit tests |

The permanent case7 fixture is important evidence for the methodology:
agreement with another implementation, or even with an incomplete “oracle”, is
not a proof. The pruning argument must stand independently, and exhaustive
enumeration is used only where the state space is small enough to provide an
independent counterexample search.

## 28. End-to-end theorem

Assume:

1. pool construction succeeds without a capacity/representation error,
   including the numeric domain of Section 29;
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

## 29. Numeric admissibility

Sections 3–23 prove that each ceiling dominates the *real-valued* objective of
every legal completion. The leaf evaluator, however, computes live score, event
point and bonus in IEEE-754 `f64` and truncates, while the ceilings are integer
or fixed-point expressions whose coefficients are themselves rounded from `f64`
constants. This section proves that each ceiling also dominates the
*floating-point* leaf value after the evaluator's truncations, on an explicit
numeric domain that pool construction enforces. Together with Sections 3–23
this makes Definition 1 hold for the implemented evaluator, not only for the
real formulas.

### 29.1 Floating-point facts

Let $u=2^{-53}$ and let $\mathrm{fl}$ denote round-to-nearest. Every operand
below is finite and non-negative, and nonzero magnitudes stay far above the
subnormal range.

- **(F1) Relative error.** $\mathrm{fl}(x\circ y)=(x\circ y)(1+\delta)$ with
  $|\delta|\le u$ for $\circ\in\{+,\times,/\}$. A chain of $k$ such operations
  on non-negative operands has relative error at most
  $(1+u)^k-1\le1.01\,ku$ while $ku\le10^{-2}$.
- **(F2) Monotonicity.** Rounding is monotone: if $0\le a\le a'$ and
  $0\le b\le b'$ then $\mathrm{fl}(a+b)\le\mathrm{fl}(a'+b')$,
  $\mathrm{fl}(ab)\le\mathrm{fl}(a'b')$ and $\mathrm{fl}(a/b)\le\mathrm{fl}(a'/b)$.
  A left-to-right sum of non-negative terms is therefore monotone in every term
  and does not decrease when a further non-negative term is appended.
- **(F3) Exact values.** Integers of magnitude at most $2^{53}$, and multiples
  of $1/4$ well inside that range, are exact, and so are their sums while they
  stay in range. Decimal constants such as $0.1$, $1.1$ or $1.15$ are not.
- **(F4) Truncation.** On non-negative values the evaluator's `as i32` /
  `as u32` / `as u64` casts and `floor` are the floor function; on non-negative
  integers the ceilings' integer `/` is the floor function.

### 29.2 Numeric domain

`numeric_domain` in `handler/capacity.rs` runs on every pool built by
`build_card_pool`, and `handler/validate.rs` checks the request parameters.
Together with the existing representation checks (card power at most
$2^{18}-1$, skill values at most 255, per-card bonus at most 409.5%, reference
skill base plus maximum at most the card's skill maximum) they establish:

- **(D1)** base, auto-base and fever constants, every skill-rate constant and
  every support-deck bonus are finite and non-negative; teammate power,
  teammate score-up and opponent score are non-negative; teammate power is at
  most $2^{24}$.
- **(D2)** Let $\hat P$ be the sum of the five largest card power maxima plus
  the honor bonus: $\hat P\le2^{24}$. After the optional power cap, let
  $\hat s$ be the largest card skill value and $\hat\sigma=\hat s$ for
  single-player lives, $\hat\sigma=\max(9\hat s/5,\ \text{teammate score-up})$
  for Multi/Cheerful. For Score and Bonus targets the a priori rate
  $\hat R=\text{base rate}+\hat\sigma\sum_k r_k/100$ is at most $2^{16}$ and
  the a priori live score $\hat X=4\hat P\hat R+0.075\cdot\text{power sum}$ is
  at most $2^{27}$.
- **(D3)** The a priori bonus $\hat B$ (five largest per-card ceilings, the
  largest leader-only bonus, the largest diversity bonus, the top-count sums
  of all support profiles added together, and the fallback extra bound) is at
  most $2^{20}$.
- **(D4)** For event Score let $\hat b$ be the event base score at $\hat X$
  ($100+\hat X/20000$ for Solo/Auto, $123+\hat X/17000$ for Multi/Cheerful),
  $m$ the music rate percent, $\beta$ the boost percent and $\lambda$ the
  Cheerful life factor (1 otherwise). Then
  $\hat I=\hat b\,m(\hat B+100)/10^4\le2^{27}$,
  $\hat E=\hat I\lambda\beta/100\le2^{30}$, and the slack $\sigma$ of Lemma N6
  is at most $10^{-5}$.

Every argument passed to an aggregate ceiling is a sum of at most five
per-card maxima (power, skill, bonus ceilings) plus request constants, and the
skill-peak argument is a single card's skill value. Hence every ceiling value
evaluated for a request is at most the corresponding a priori quantity, and
(D2) also keeps every `i64` numerator product below $2^{63}$ and every `u32`
power sum below $2^{32}$. The limits keep a factor of at least 2 below the
thresholds at which the lemmas stop holding; real master data lies one to three
orders of magnitude inside them. A `SearchContext` constructed directly for the
search API is expected to satisfy the same domain.

### 29.3 Lemma N1 — fixed-point coefficients

For a non-negative `f64` constant $c$ the prepared coefficient
$K=\lceil\mathrm{fl}(c\cdot10^6)\rceil$ satisfies $K\ge10^6c(1-u)$. The
rate-sum coefficients carry at most seven roundings, e.g.

$$
\left\lceil\mathrm{fl}\!\left(\mathrm{fl}\Big(\sum_{k<6}r_k\Big)/500\cdot10^6\right)\right\rceil
\ge10^6\,\frac{\sum_k r_k}{500}\,(1-u)^7,
$$

and likewise for the Average five-slot sum and leader rate. The integer steps
that follow (products and `ceil_div_positive`) are exact or round up, and the
base rate is the same `f64` value on both sides. Hence the live numerator
satisfies

$$
N\ge10^6X_B(1-u)^7,
$$

where $X_B$ is the real-valued ceiling evaluated at the exact `f64` constants.

The ceiling of a rounded product can lie below the real product: the constant
$1.1$ is stored as $1.1+8.9\cdot10^{-17}$ and $\mathrm{fl}(1.1\cdot10^6)$ is
exactly $1\,100\,000$, so $K/10^6<c$. Lemma N1 needs only the relative form.

### 29.4 Lemma N2 — evaluator rounding

Let $X^*(D)$ be the live score of a legal completion $D$ computed in exact
arithmetic from the same `f64` constants, and $X_e(D)$ the evaluator's value
before truncation. Then $X_e\le X^*(1+u)^{20}$:

- integral and quarter-integral slot score-ups are exact (F3); the Multi self
  score-up $L+\sum_i o_i/5$ costs at most 5 roundings, and a further Average
  division at most 5 more;
- the rate $\mathrm{base}+\sum_k\mathrm{fl}(\mathrm{fl}(su_k r_k)/100)$ costs 2
  roundings per term and 6 additions;
- the product with power costs 1 (the factor 4 is exact), the co-op term
  $\mathrm{fl}(\mathrm{fl}(5\cdot0.015)\cdot\text{power sum})$ costs 3 and the
  final addition 1.

All terms are non-negative, so by F1 the counts compose to at most 20. The
monotonicity arguments of Section 12 and of exactness-proof Section 3 (every
slot score-up is at most the slot peak used by the ceiling, rates are
non-negative, power and skill inputs are upper bounds) give $X^*\le X_B$.

### 29.5 Theorem N3 — live-score granularity

Inside the domain, $\lfloor X_e\rfloor\le\lfloor N/10^6\rfloor$ for every legal
completion.

**Proof.** By N1 and N2,
$X_e\le(N/10^6)(1+u)^{20}(1-u)^{-7}\le N/10^6+28u\,N/10^6$. By (D2),
$N/10^6\le2^{27}(1+10^{-12})$, so $28u\,N/10^6<4.2\cdot10^{-7}<10^{-6}$. Since
$N$ is an integer, $\lfloor N/10^6\rfloor+1\ge(N+1)/10^6>X_e$. ∎

The argument needs no extra margin in the code: the $10^{-6}$ grid of the
numerator is the margin. It fails only beyond $N/10^6\approx3.2\cdot10^8$; at
real magnitudes (live scores near $10^7$) the rounding uses less than 4% of one
grid step. Section 10 compares the same integer $N$, and the packed live
component takes $\lfloor N/10^6\rfloor<2^{31}$ without wrapping.

### 29.6 Theorem N4 — event-point stages

Write $b$ for the integer event base score, $t$ for the evaluator's `f64` bonus
and $T$ for the integer bonus handed to the ceiling, with $t\le T+\varepsilon$.

1. *Base score.* $b$ is non-decreasing in the live score, and Theorem N3 makes
   the ceiling's live score at least the evaluator's. For Multi/Cheerful the
   evaluator's `(live as f64 / 17000.0) as i32` equals the integer quotient:
   for $0\le\text{live}<2^{31}$ the real quotient is either an integer or at
   least $1/17000$ away from one. The opponent term is the same integer
   expression on both sides; for live scores below $2^{29}$ neither the
   saturating `i32` nor the `i64` product by 4 saturates.
2. *First stage.* The evaluator computes
   $I_e=\lfloor\mathrm{fl}(\mathrm{fl}(b\cdot\mathrm{fl}(m/100))\cdot\mathrm{fl}(\mathrm{fl}(t/100)+1))\rfloor$
   and the ceiling $I_B=\lfloor V_B\rfloor$ with $V_B=b_Bm(T+100)/10^4$. The
   float value is at most $V_B(1+5.1u)+1.01\,b_Bm\varepsilon/10^4$. $V_B$ is a
   multiple of $10^{-4}$, so $V_B\le\lfloor V_B\rfloor+1-10^{-4}$ unless it is
   an integer. Therefore $I_e\le I_B$ whenever

   $$
   \sigma=5.1u\,V_B+1.01\,\frac{b_Bm}{10^4}\,\varepsilon<10^{-4}.
   $$

   For $\varepsilon=0$ this needs only $V_B<1.7\cdot10^{11}$; (D4) gives
   $V_B\le2^{27}$ and $\sigma\le10^{-5}$.
3. *Cheerful life stage.* The evaluator's life factor is
   $\mathrm{fl}(\mathrm{fl}(1.15)+q)$ where $q=\mathrm{fl}(\ell/5000)$ clamped to
   $[\mathrm{fl}(0.1),\mathrm{fl}(0.2)]$. Because $\mathrm{fl}(0.1)$ and
   $\mathrm{fl}(0.2)$ are exactly $\mathrm{fl}(500/5000)$ and
   $\mathrm{fl}(1000/5000)$, $q=\mathrm{fl}(c/5000)$ with
   $c=\mathrm{clamp}(\ell,500,1000)$, the ceiling's integer clamp, and the
   factor is at most $(1+u)^2(5750+c)/5000$. The ceiling's stage
   $\lfloor I_B(5750+c)/5000\rfloor$ lies on a $1/5000$ grid; with
   $I_B\le2^{27}$ the float error $1.35\cdot3.1u\,I_B<10^{-7}$ stays far below
   the grid step $2\cdot10^{-4}$.
4. *Boost stage.* $\lfloor W\beta/100\rfloor$ lies on a $1/100$ grid and the
   float error is at most $2.1u\,\hat E<10^{-6}$. For the normalized boost
   values, multiples of 100, the stage is exact.
5. *Range.* By (D4) every intermediate is below $2^{30}$, so the ceiling's
   `i64 → i32` casts are lossless and the evaluator's casts do not saturate.

Challenge event points use the same integer expression on both sides, and a
MySekai live has event point 0 on both sides. Component-wise dominance of event
point and live score gives dominance of the packed key
`(event_point << 32) | live`.

### 29.7 Lemma N5 — directly summed bonus

Let the ceiling's bonus be $T=C+D+\lceil S_B\rceil$, where $C$ is a sum of
per-card and leader-only ceilings of tenth-percent values, $D$ an integer
diversity bound, and $S_B$ a left-to-right `f64` sum over a support list. If the
support sum is computed directly, then $t\le T$ exactly ($\varepsilon=0$):

- $\mathrm{fl}(x_{10}/10)\le C$ by F2–F3, because $C\ge x_{10}/10$ is an
  exactly representable integer;
- both sides scan a support list sorted by non-increasing bonus. The ceiling
  excludes the game ids of the selected prefix, the evaluator those of the
  whole deck, a superset. The $k$-th element of a subsequence of a
  non-increasing list occurs no earlier than the $k$-th element of any longer
  subsequence containing it, so the ceiling's $k$-th picked value dominates
  the evaluator's, and the ceiling picks at least as many non-negative terms.
  For the Final Chapter envelope (per-id maximum over every profile, count
  equal to the largest count) the same order-statistic argument applies to the
  $k$-th largest remaining value. By F2 the evaluator's `f64` support sum is at
  most $S_B$;
- the final additions satisfy
  $\mathrm{fl}(\mathrm{fl}(\mathrm{fl}(x_{10}/10)+d)+s)\le C+D+\lceil S_B\rceil$
  by F2–F3.

This covers the suffix support bounds, the Final Chapter leader helpers and the
build-time extra bonus bound. Consequently the Bonus key satisfies
$\mathrm{round}(2t)\le2T$, and the MySekai value, a monotone `f64` chain in its
power and bonus inputs, is dominated by F2.

### 29.8 Lemma N6 — incrementally maintained support sums

The Final Chapter card-level plan maintains the remaining support sum with one
subtraction and one addition per selected card instead of re-summing. In exact
arithmetic it equals the direct sum of Lemma N5; in `f64` it may differ. For
the profile $(2.2,2.2,0.6,0.2,0.2)$ with count 3, removing the second $2.2$ and
the first $0.2$ updates the sum to exactly $3$, while the evaluator sums
$2.2+0.6+0.2$ to $3.0000000000000004$; the ceiling then carries bonus $3$,
below the evaluator's $t$.

Each of the at most $c+8$ operations of the incremental sum, and each of the
$c$ additions of the direct sum, errs by at most $uM$, where $c$ is the profile
count and $M$ the largest sum of $c+5$ support values of a profile. Adding the
evaluator's final rounding of the total gives $t\le T+\varepsilon$ with

$$
\varepsilon=(2c+10)\,uM+u\hat B.
$$

These plans serve the Score target only. (D4) bounds the resulting
$\sigma\le10^{-5}$, so Theorem N4 still gives $I_e\le I_B$, and the later
stages consume integers. The event point, and hence the packed key, remain
dominated.

### 29.9 Lemma N7 — Skill key

The Skill key is $\lfloor\mathrm{fl}(\mathrm{fl}(10v)+10^{-6})\rfloor$, where
$v$ is the left-to-right sum of the leader's score-up and $0.2$ times each
other score-up. With integral or quarter-integral score-ups, the exact $10v^*$
is a multiple of $1/2$ and at most the ceiling $2S+8L$. For $10v^*<10^5$ the
float error is below $10^{-9}$, so the float value lies in
$(\lfloor10v^*\rfloor,\lfloor10v^*\rfloor+1)$ and the key is at most
$\lfloor10v^*\rfloor\le2S+8L$.

### 29.10 Lemma N8 — correlated bound

The coefficients of Section 11 are
$C=\lceil\mathrm{fl}(\mathrm{base}\cdot Q)\rceil+1$ and likewise $B$, $D$, with
$Q=10^{12}$, and the constructor's own domain requires base $\le4$, rates
$\le1$ and card power at most $2^{18}-1$. The rounding of each
$\mathrm{fl}(cQ)$ is below $5\cdot10^{-4}$, so each coefficient exceeds $cQ$ by
at least $1-5\cdot10^{-4}$, and the plane's real live expression exceeds the
exact-constant live score by at least $4P(1+S+L)(1-5\cdot10^{-4})/Q$. The
Solo/Auto Average evaluator exceeds that live score by at most
$((1+u)^{12}-1)\cdot4P(4+(S+L)/100)$. The first is more than 180 times the
second for all $S,L\ge0$, so every plane dominates the floating-point value
before its outward integer steps. The final `+1` in **quadratic** is not
required by this argument.

### 29.11 Scope

Everything else in the search is integer arithmetic on exact discrete state.
Floating-point values enter a ceiling only through the aggregate objective
coefficients (N1–N4, N7), the correlated coefficients (N8), support sums
(N5–N6) and the MySekai value (N5). A MySekai live type has no live-score
formula of its own: the evaluator scores it with the solo constants and the
Bonus and Score keys keep that live score, so the aggregate ceiling uses the
same formula.

Verification: src/search/tests/numeric_soundness.rs enumerates every deck of
generated pools under decimal constants and asserts packed and component-wise
dominance for every live type, skill order and target; targets exact-integer
and near-integer grid points of the live numerator and of the event stages;
checks the correlated bound and the support ceilings at every prefix; and
compares complete searches, including the incremental-support example above,
with the exhaustive oracle.
