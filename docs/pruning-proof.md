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
- the MySekai rank key: MySekai score times $2^{32}$ plus resolved power
  (clamped, honor bonus included), the first two fields of the MySekai result
  order;
- total Power;
- encoded Skill value;
- the per-tier live-score key for exact Bonus tiers.

The public result order contains additional deterministic tie-break fields.
Therefore equal values of $v$ are **not interchangeable**.

For a maximizing Top-K search whose tracker already contains K distinct public
card sets, let $\tau$ be the K-th primary objective value. For minimizing
Power, $\tau$ is the K-th value in the reversed numeric order. An external
floor $f$ (Section 13) is a MySekai score for MySekai and enters as the rank
key $f\cdot2^{32}$: a deck below it has a score below $f$.

Every MySekai ceiling bounds the rank key: it packs the MySekai value of its
power and bonus bounds with the power bound itself, and since both bound the
deck's score and resolved power, the packed pair bounds the deck's pair in
lexicographic order. On the numeric domain (Section 29, D2 and D3) resolved
power is at most $2^{24}$ and the MySekai value below $2^{28}$, so the numeric
order of the packed keys is the lexicographic one.

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

### Corollary 3 — candidate runs

A candidate run is a maximal block of consecutive dense cards whose candidate
bonus terms are equal: the rounded total bonus, or under Final Chapter the base
bonus plus any counted limited amount together with whether the card takes a
limited slot. Let candidate $i$ lie in a run ending at $e$. Every candidate $j$
with $i<j<e$ has the same bonus terms, power and skill no larger than the run
maxima from $i$, and dense tails from $j+1$ no larger than those from $i+1$,
because the suffixes are nested. The candidate ceiling is non-decreasing in
card power, card skill and every tail, so the ceiling evaluated with the run
maxima and the tails after $i$ bounds every candidate in $[i,e)$. When it is
below $\tau$, the scan resumes at $e$.

The dense-suffix break of Corollary 2 is checked by scan position rather than
stride alignment, so a skip never lowers how often that check runs.

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
Section 29 (Lemma N7) shows that the prepared coefficients alone already
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

For Solo, Auto and Challenge lives under the Best, Worst and Specific skill
orders, the rate adds six slot score-ups, the five members in placement order
and the leader again, each multiplied by one of six slot rates. Let $S$ bound
the members' score-up sum and $L$ each member's score-up, and let
$M=\min(L,S)$, which bounds every single slot. Every order pairs the slots
with the rates by some permutation, and for rates $R_1\ge\dots\ge R_6$ any
such sum is at most

$$
M R_1+\sum_{j=2}^{6} v_j R_j,\qquad v_j=\min\Bigl(M,\;S-\sum_{i<j,\,i\ge2}v_i\Bigr),
$$

the leader slot taken on its own at most $M$ on the largest rate and the
member sum spread over the others, largest rates first, at most $M$ each (a
fractional knapsack with unit weights). The value is non-decreasing in $S$
and $L$, and it is at most the uniform relaxation $L\sum_j R_j$.

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
for every deck of $R$. A same-character search (Section 22) instead takes five
distinct public cards, so its plan sums the five largest values over the
admitted cards themselves, and each such sum again bounds the deck sum. A plan
is dropped when the regime admits fewer than five characters (five public
cards for a same-character search) or cannot satisfy a fixed card, fixed
character or forced leader; it then has no feasible deck.

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

Each event-score layer computes the extra-bonus bound of any completion of its
prefix once, from the matching bound of Section 15 and the support sum above,
capped by the pool-wide fallback; the minimum of two admissible bounds is
admissible by Lemma 1. The candidate, run and exclusion-aware ceilings of that
layer use it in place of the fallback.

## 17. Exact bonus tiers

Implementation: src/search/solver/bonus_tiers.rs.

A request lists integer tiers $T$ (percent). For each tier the solver returns
the canonical Top-K (§2 of [exactness-proof.md](exactness-proof.md)) of legal
ordered decks whose evaluated total bonus is exactly $T$. All decks of one
tier share the bonus half of the ranking key, so inside a tier the primary
objective is the live score, and $\tau_T$ below is the tier's K-th live
score. Each tier has its own tracker; pruning for tier $T$ reads only
$\tau_T$.

### 17.1 Groups and leaves

A deck takes one card from each of five distinct **groups**:

- the *fixed roles* in slot order — slot $s$ of a fixed card or fixed
  character, and in the Final Chapter the leader slot 0 — each holding every
  card that satisfies that slot's public constraint;
- the *free groups* — one per character under character uniqueness, one per
  public card id inside a Challenge character (Challenge-all solves every
  character as its own scope and merges through the shared trackers, which is
  exact by §22).

Fixed roles and, outside the Final Chapter, a forced leader's character are
mandatory. A character (public id) that every card of a fixed role shares is
removed from the free groups; any remaining overlap (for example the leader
role of an automatic Final leader) is rejected per branch by the character /
public-id checks. Hence the sets of cards reachable as leaves are exactly the
legal card sets with a legal fixed-slot assignment, each fixed role in its
slot and the free cards in the remaining slots.

A leaf is evaluated by the shared placement routine
(`visit_bonus_candidates`). When first-N limited counting can distinguish
orders of the free slots, it offers every permutation of the free slots;
otherwise every legal placement has the same total and it offers the
canonically best one. Each offered deck is inserted into the tier whose value
equals its evaluated total (`resolve_total_bonus == T`, exact comparison), so
for every visited card set and fixed-role assignment the tier trackers
receive exactly the placements the ordered oracle enumerates for that set.
The trackers deduplicate by public set with the canonical order. It remains
to show that no pruned branch contains a deck that belongs to some tier's
Top-K.

### 17.2 Keys, slack and support excess

Every card $c$ in its group has an integer **key** $\kappa_c$, a **slack**
$\sigma_c \ge 0$ (tenths of a percent) and a **displaced count**
$q_c\ge 0$. There is a non-decreasing integer **excess** function $\xi$
with $\xi(0)=\xi(1)=0$ such that for every legal deck $D$ with total
$\operatorname{total}(D)$ and $q(D)=\sum_{c\in D}q_c$ there are deck-level
terms $E(D)$ with

$$
\sum_{c\in D}\kappa_c + E(D) - \xi\bigl(q(D)\bigr)
\;\le\; 10\cdot\operatorname{total}(D) \;\le\;
\sum_{c\in D}(\kappa_c+\sigma_c) + E(D),
$$

and a known outward range $E(D)\in[E_{lo},E_{hi}]$.

*Without World Bloom*, $\sigma_c = q_c = 0$, $\xi\equiv 0$ and $E(D)=0$:
$\kappa_c$ is the card's exact counted bonus in tenths — the whole card
bonus when every limited bonus counts, otherwise its base bonus plus its
limited bonus when (and only when) that card is one of the counted limited
cards (§17.4). On the Final leader role $\kappa_c$ also contains the exact
leader-only honor and limit bonus of that card. The inequality is then the equality computed by the evaluator,
which sums these integer tenths.

*With World Bloom* the evaluator adds the diversity bonus $d(k)$ of the
deck's number $k$ of distinct attributes and the support bonus. Let a support
profile list entries $s_1\ge s_2\ge\cdots\ge 0$ with $W$ counted entries,
let $M$ be the largest number of entries five main cards can hold (a main
card removes every entry of its public id), and put $s_i=0$ past the end.
With base $B=\sum_{i\le W}s_i$ the support bonus is $B-\operatorname{loss}(D)$.
Let $R$ be the positions of the removed entries and $a=|R\cap[1,W]|$. The
counted entries are the surviving positions $\le W$ and the first $a$
surviving positions $w_1<\cdots<w_a$ after $W$, so

$$
\operatorname{loss}(D)
= \sum_{\substack{i\in R\\ i\le W}} s_i - \sum_{j=1}^{a} s_{w_j}
= \sum_{c\in D}\ell_c + X(D),\qquad
\ell_c=\sum_{\substack{i\le W\\ i\in c}}(s_i-s_{W+1}),\quad
X(D)=\sum_{j=1}^{a}\bigl(s_{W+1}-s_{w_j}\bigr),
$$

with $\ell_c$ summed over the card's own entries. Every term of $X(D)$ is
non-negative because $w_j>W$. Let $q=|R\cap[1,W+M]|\le M$. The positions
$W+1,\dots,W+q$ hold at most $q-a$ removed entries, hence at least $a$
surviving ones, so every $w_j\le W+q$ and

$$
0\le X(D)\le X_q=\sum_{k=1}^{q}\bigl(s_{W+1}-s_{W+k}\bigr),
$$

a sum of non-negative terms that contains every term of $X(D)$. $X_q$ is
non-decreasing in $q$ and $X_0=X_1=0$: a deck holding at most one entry of
the first $W+M$ positions loses exactly $\sum\ell_c$. With $q_c$ the number
of the card's entries at positions $\le W+M$, $q=\sum_{c\in D}q_c$.

Write $[\ell_c^{\min},\ell_c^{\max}]$ for the range of $\ell_c$ over the
support profiles in use: a single profile outside the Final Chapter, where
both ends are $\ell_c$; in the Final Chapter the profile depends on the
leader and every profile is included. $q_c$ is the maximum over those
profiles, and $\xi(q)$ the maximum over profiles of
$\lceil 10X_{\min(q,M)}\rceil$ with that profile's $M$; since a deck holds
at most $M$ entries of each profile and each $X$ is non-decreasing,
$10X(D)\le\xi(q(D))$ under every leader. The key folds the
card's own loss in: with the card's exact counted bonus $b_c$ (as above) and
unit $u$ (§17.3),

$$
\kappa_c = b_c + u\left\lfloor\frac{\lfloor -10\ell_c^{\max}\rfloor}{u}\right\rfloor,
\qquad
\kappa_c+\sigma_c = b_c + \lceil -10\ell_c^{\min}\rceil ,
$$

so $-10\operatorname{loss}(D)$ lies between the key sum minus $\xi(q(D))$
and the key-plus-slack sum. $E(D)=10\,d(k)+10B$ is bounded by $10\,d(k)$
plus the outward integer range of $10B$ — of the leader's profile once the
leader is chosen, of all profiles before. A card with no entry in the first
$W+M$ positions keeps $\kappa_c=b_c$ and $\sigma_c=q_c=0$.

*Rounding.* Each real-valued term $x$ (a card loss, a profile base, an excess
bound) enters as $\lfloor 10x+\varepsilon\rfloor$ where it bounds from
below and $\lceil 10x-\varepsilon\rceil$ where it bounds from above, with
$\varepsilon=10^{-6}$. A deck's bound sums at most eight such terms, so a
rounded lower bound exceeds the exact one by less than $8\varepsilon$ plus
the binary rounding of the support sums (below $10^{-9}$), and symmetrically
for upper bounds. The bounds are compared only with the integer $10T$: an
integer that exceeds a quantity that is at most $10T$ by less than one is
itself at most $10T$. Values within $\varepsilon$ of a whole tenth are thus
taken exactly, and the integer inequalities hold for every deck whose
evaluated total is $T$.

If some profile is unsorted, negative or non-finite, the fold is not used:
$\kappa_c=b_c$, $\sigma_c=q_c=0$ and $E(D)$ is treated as unbounded, which
leaves only count and counting-state feasibility (§17.5) to prune.

*Diversity classes.* The search runs once per class $v$ of attribute counts
$k$ with equal $d(k)$, fixing $d(k)=v$, and rejects a branch as soon as no
count of the class is reachable: with $a$ distinct attributes selected and
$r$ cards left, the final count lies in $[\max(a,1),\min(5,a+r)]$. Every deck
lies in exactly one class, so the classes partition the feasible set.

### 17.3 Regimes and the suffix table

The decks are covered by the area-item composition regimes documented in
src/search/composition.rs: `Mixed`, `SharedAttr(a)`, `SharedUnit(u)` and
`SharedUnitAttr(u,a)`. A deck of a regime has every card admitted by it, and
each admitted card's resolved power is at most its regime power bound, the
maximum of its power over the regime's member keys. Every deck belongs to
at least one regime. The regime searches below are complete for the decks of
their regime; decks of other regimes they visit are evaluated exactly and
never required. A regime that shares an attribute needs only decks with one
attribute, the others only decks with at least two, which fixes the
diversity classes it searches.

Let $u$ be the greatest common divisor of every card's counted bonus parts
(base, limited, and on the leader role base plus the leader-only bonus). Keys
are multiples of $u$, and a common offset $O$ (a multiple of $u$) makes every
$\kappa_c+O\ge 0$. For a regime, order the groups: fixed roles first, then
the free groups by decreasing best power bound. Define for position $p$,
remaining count $r$, counting state and shifted key sum $x$ the table entry

$$
G_p(r,\text{state},x) = \Bigl(\max\sum P,\ \max\sum S,\ \max\max L\Bigr)
$$

over all selections of exactly $r$ admitted cards from distinct groups at
positions $\ge p$, containing one card of every mandatory group there, with
that counting state and $\sum(\kappa+O)=x\,u$. $P$ is the regime power bound,
$S$ and $L$ the per-card skill maximum; each component is maximized
independently and an empty set is marked unreachable. The recurrence over
$p$ from the end — skip the group (unless mandatory) or take one card of it,
in either counting state for a limited card (§17.4) — is the standard
exact selection DP, so by induction every entry is exact for its definition.
Cards with equal key, slack, displaced count and limited bonus share one
table item with their componentwise maxima, which can only raise entries.
Sums above the largest value any hitting deck can need are dropped: with
bounded deck terms ($E_{lo}\ge -1$ tenth) a hitting deck has
$\sum\kappa\le 10T_{\max}+1+\max_q\xi(q)$.

The fixed roles are treated in the table as if their limited bonus could be
counted or not; this only enlarges the selection set.

### 17.4 First-N limited counting

When the event counts limited bonuses only for the first $N<5$ positive
limited cards in slot order, the fixed roles precede every free slot, so
their limited bonuses count deterministically in slot order while capacity
remains. Among the free cards the offered placements realize exactly the
counted sets $S$ of the free positive limited cards with
$|S|=\min(|\Lambda|,N')$, $N'$ the remaining capacity. Writing $j$ for the
counted number and $u\in\{0,1\}$ for "some positive limited card is not
counted", these are exactly the choices with $j\le N'$ and $u=1\Rightarrow
j=N'$. The search branches on counted/uncounted for each free positive
limited card and carries $(j,u)$; the table indexes the suffix by its own
$(j,u)$ and the query admits exactly the suffix states compatible with the
prefix (mode 0: prefix $u=0$; mode 1: prefix $u=1$, the suffix must fill the
capacity). Hence the path of every deck under the placement that hits the
tier survives these feasibility tests.

### 17.5 Pruning

At a state after deciding the groups before position $p$, with $r$ cards
left, prefix key sum $K$, prefix slack $\Sigma$, prefix displaced count
$q_{pre}$ and deck-term range $[E_{lo},E_{hi}]$, let $\sigma^{\max}_p(r)$
and $q^{\max}_p(r)$ be the sums of the $r$ largest per-group maximum slacks
and displaced counts among positions $\ge p$. Every completion has
$q(D)\le q_{pre}+q^{\max}_p(r)$, and $\xi$ is non-decreasing, so by §17.2
the suffix $Q$ of every completion that hits $T$ satisfies

$$
10T - E_{hi} - \Sigma - \sigma^{\max}_p(r) - K
\;\le\; \sum_{c\in Q}\kappa_c \;\le\;
10T - E_{lo} + \xi\bigl(q_{pre}+q^{\max}_p(r)\bigr) - K .
$$

The branch is discarded when this interval contains no shifted multiple of
$u$ below the table cap, or when every such entry $G_p(r,\cdot,\cdot)$
compatible with the counting state is unreachable: then no completion hits
$T$ (a feasibility proof). Otherwise let $(P^*,S^*,L^*)$ be the componentwise
maximum of those entries. Every hitting completion's suffix is one of the
selections counted there, so

$$
\operatorname{live}(D)\le
\operatorname{ceiling}\bigl(P_{pre}+P^*,\ S_{pre}+S^*,\ \max(L_{pre},L^*)\bigr),
$$

the `ObjectiveBound` live-score ceiling, which is monotone in non-negative
power, skill-sum and leader-skill upper bounds (§12) and admissible because
the per-card skill maximum bounds every resolved skill value (§20). The branch
is discarded only when this ceiling is strictly below $\tau_T$ (Theorem 1,
Corollary 1). A class of cards with a common key, slack, displaced count and
counting choice is first tested with its componentwise maxima, which dominate each card of
the class. Children are explored in non-increasing ceiling order and the
loop stops at the first child below the current $\tau_T$; $\tau_T$ only
increases, so every later child is also below it. A regime is skipped for a
tier when its unconditioned regime ceiling — roles' best power, skill and
the top remaining groups — is below $\tau_T$.

When the live type has no admissible live-score relaxation (MySekai), the
trackers disable numeric cutoffs and only the feasibility proofs above
prune.

### 17.6 Result

Every deck in some tier's canonical Top-K lies in a regime and a diversity
class whose search reaches its card set with its fixed-role assignment and
its counting choices, because every test on its path is either a
feasibility test it passes or a strict bound test its live score passes. Its
leaf offers its hitting placements (§17.1). Hence a completed search returns,
per tier, exactly the canonical Top-K of the full ordered feasible set.

## 18. Final Chapter bounds

Implementation: src/search/solver/final_chapter.rs.

### 18.1 Leader jobs

Every surviving leader card becomes a search job. There is no heuristic
per-character leader cap.

A leader job is skipped only if character_ceiling(...) < threshold. Auto-leader
jobs are ordered by a ceiling read from one table for every leader character:
its top lists skip the leader's character exactly, and its attribute rows
maximize over the groups of every character, a superset of the leader's
groups. Section 18.3 is non-decreasing in every table entry, so this ceiling
bounds the one of the leader character's own table. A job is skipped when
either ceiling is below the threshold; the character's own table is built when
its first job is reached.

### 18.2 Character-attribute groups and the attribute-union DP

A member group holds one character's cards of one attribute. Every member card
lies in exactly one group, and a deck takes at most one group of each
character, so enumerating strictly increasing group indices that skip
characters already taken lists every member set of distinct characters once.
Because a group fixes its attribute, the attribute union of the selected
prefix is exact, and the group's power, skill and bonus maxima are those of
cards with that attribute.

attr_bonus[k][s] stores the maximum `diff_attr_bonus` obtainable by selecting
exactly $k$ groups from a suffix, starting from attribute union `s`. Its
transition keeps the skip-current-group value and takes the value for
`s | attribute` in the `k-1` row, where `attribute` is the current group's.
This is the OR-product DP with the final bonus lookup memoized, and it holds
for nonmonotone `diff_attr_bonus` tables. The DP may select two groups of one
character; the legal selections are a subset of the ones it maximizes over,
so the table bounds the attribute bonus of every legal completion.

Combining the selected-prefix union and leader attribute into `s`, then looking
up `attr_bonus[remaining][s]`, therefore bounds that dimension.

### 18.3 Character-level numeric ceiling

For the remaining slots, the suffix independently takes the largest values
of power, skill, base bonus, and limited bonus over distinct characters: each
character contributes its best value over its groups in the suffix, since a
deck takes at most one of them.

For limited bonus, the selected values are merged with the first `remaining`
suffix values, and only the largest values up to the remaining
card_bonus_count_limit are admitted. The $j$-th largest value of a legal
completion is at most the $j$-th suffix value, so it can contribute no more
than this top-cap sum.

These maxima may come from mutually incompatible card choices inside a group.
That is a relaxation, so their combination can only overestimate.

Adding the attribute-union maximum and the support upper bound from Section 16
therefore yields an admissible character_ceiling.

### 18.4 Character-loop break

Groups are sorted by a descending group key, but the correctness of the break
does not depend on that heuristic key. character_ceiling at position i reads
GroupCeilingTail[i], which was built from the exact suffix groups[i..]. Moving
to i+1 removes a group from that suffix. Every per-character maximum, every
per-component top list and every reachable attribute-union set can therefore
only stay equal or shrink.

Thus character_ceiling is non-increasing with the start index. Once it is below
the threshold, the rest of the group loop can safely break. Groups of a
character already taken are skipped without a ceiling. Within one loop the
prefix is fixed, so the ceiling is a function of the suffix entries it reads:
the first `remaining` values of each top list and the attribute row for
`remaining`. Each table carries, for every count of open slots, a version that
changes exactly when those entries change; they only grow with the suffix, so
a version never returns to an earlier table, and an unchanged version reuses
the previous ceiling.

### 18.5 Card-level plan

After four member groups are fixed, CardGroupPlan stores suffix sums of
each selected group's own best power, skill, base bonus, and sorted limited
bonuses. The leader and the four groups fix the deck's attribute union, so
the plan stores its exact diversity bonus.

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

Every scan position of a group stores the maxima of power, skill, base
bonus and rounded limited bonus from that card to the end of the group, whose
cards share one attribute. The candidate ceiling reads only these four terms
and the attribute, and it is non-decreasing in each term: the power, skill and
base sums grow, the merged top limited values cannot shrink when one value
grows, and the attribute table is read at the same union. The ceiling of the
rest maxima therefore bounds every later card of the group. When it is below
$\tau$, the scan of the group ends (Theorem 1), including the cards after the
ranked buffer, since the threshold never decreases.

### 18.6 Ranked card buffer and its monotone break

The fixed-size RANKED_CAP buffer only reorders the first candidates by their
already-proved upper bound. Insertion keeps this buffer in non-increasing upper
bound order.

During exploration the tracker threshold can only stay equal or increase. If
the next ranked entry has upper bound $U<\tau$, every later ranked entry has
upper bound at most $U$ and is also below the current (or any future) threshold.
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

### 18.8 Log-linear event-point bound

Implementation: src/search/log_linear.rs, used by solver/final_chapter.rs.

Sections 18.3 and 18.5 relax power, skill and bonus independently, so a
subtree whose largest power, skill and bonus come from different cards or
groups receives the ceiling of a deck that does not exist. For the Score
target of an event on a Solo, Auto or Multi live, a second bound couples the
three. For a deck let $P$ be the sum of its card power maxima, $S$ the sum of
its card skill maxima (the leader included), $L$ the leader's skill and $B$
the bonus input of the card-level ceiling of Section 18.5, with the limited
bonuses summed without the count cap, which can only raise it. Let $E$ be the
event point of `ObjectiveBound::ceiling` at these features, which bounds the
event point of the deck by Section 18.5 and Section 29.

**Lemma LL1 (product form).** Let $h$ be the honor bonus, $m$ and $\beta$ the
music and boost rates in percent, $\kappa=m\beta/10^6$, and
$(c,D)=(100,20000)$ for Solo and Auto, $(110+o,17000)$ for Multi, where $o=13$
when the opponent score is zero and $\min(\lfloor\text{other}/340000\rfloor,13)$
otherwise. Then

$$
E\le\kappa\,\bigl(c'+u\bigr)(100+B),\qquad u=\frac{(P+h)\,q(S,L)}{D},
$$

where $q(S,L)=(4\bar r(S,L)+a)/10^6$, $a$ is the active-score coefficient per
unit power (five times $0.075\cdot10^6$ without a teammate power, once with
one), $c'=c+4\cdot0.075\,t/D$ for teammate power $t$ ($c'=c$ otherwise), and
$\bar r$ bounds the fixed-point live rate of Section 12:

- Multi: $\max(4L+S,T)\rho\le(4L+S)\rho+\max(0,T-4L_{\min})\rho$ over leaders
  with skill at least $L_{\min}$, where $T$ is five times the teammate
  score-up and $\rho$ the rate per skill;
- Solo and Auto under the Average order: each outward-rounded division adds
  less than one, so $\bar r=r_0+2+S A_5/500+L A_L/100$;
- Solo and Auto otherwise: the rate is non-decreasing in its peak slot, and
  the peak is at most the largest card skill $M$, so
  $\bar r=r_0+MR_1+\sum_{j=2}^{6}R_j\,\mathrm{clamp}(S-(j-2)M,0,M)$, which is
  concave and non-decreasing in $S$ because $R_2\ge\dots\ge R_6\ge0$.

*Proof.* The live numerator is $4rP'+a'$ with $P'\le P+h$ after the optional
power cap, and the live score is its floor over $10^6$. The event base score
is at most $c$ plus the live score over $D$, since the floors only lower it
and the opponent term is at most $o$. The event point takes floors of
non-negative products, each at most the product itself. $\square$

**Lemma LL2 (threshold interval).** Let every deck in question satisfy
$P\le\hat P$, $S\le\hat S$, $L_{\min}\le L\le\hat L$ and $B\le\hat B$. Every
such deck with $E\ge\tau_0$ has $u\in[u_{\mathrm{lo}},u_{\mathrm{hi}}]$ with
$u_{\mathrm{hi}}=(\hat P+h)q(\hat S,\hat L)/D$ and
$u_{\mathrm{lo}}=\tau_0/(\kappa(100+\hat B))-c'$.

*Proof.* $q$ is non-decreasing in both arguments, and by Lemma LL1 a deck with
$u<u_{\mathrm{lo}}$ has $E<\kappa(c'+u_{\mathrm{lo}})(100+\hat B)=\tau_0$.
$\square$

**Lemma LL3 (affine bound).** Suppose $0<u_{\mathrm{lo}}<u_{\mathrm{hi}}$. Let
$\ell(S,L)=\ell_0+\ell_SS+\ell_LL$ be affine with $\ell\ge q$ for $S\ge0$ and
$L\in[L_{\min},\hat L]$, and let $P_0,B_0>0$ and $Q_0=\ell(S_0,\hat L)>0$ for
some $S_0$. Every deck of Lemma LL2 with $E\ge\tau_0$ satisfies

$$
\ln E\le K+\frac{\sigma}{P_0}P+\frac{\sigma\ell_S}{Q_0}S+\frac{\sigma\ell_L}{Q_0}L+\frac{B}{100+B_0},
$$

$$
K=\ln\kappa+\varphi(t_{\mathrm{lo}})-\sigma t_{\mathrm{lo}}
+\sigma\Bigl(\ln P_0-1+\frac{h}{P_0}+\ln Q_0-1+\frac{\ell_0}{Q_0}-\ln D\Bigr)
+\ln(100+B_0)-1+\frac{100}{100+B_0},
$$

where $\varphi(t)=\ln(c'+e^t)$, $t_{\mathrm{lo}}=\ln u_{\mathrm{lo}}$,
$t_{\mathrm{hi}}=\ln u_{\mathrm{hi}}$ and
$\sigma=(\varphi(t_{\mathrm{hi}})-\varphi(t_{\mathrm{lo}}))/(t_{\mathrm{hi}}-t_{\mathrm{lo}})$.

*Proof.* By Lemma LL1, $\ln E\le\ln\kappa+\varphi(\ln u)+\ln(100+B)$. The
function $\varphi$ is convex, since its derivative $e^t/(c'+e^t)$ increases,
so on $[t_{\mathrm{lo}},t_{\mathrm{hi}}]$, which contains $\ln u$ by Lemma
LL2, it lies below its chord: $\varphi(t)\le\varphi(t_{\mathrm{lo}})+\sigma(t-t_{\mathrm{lo}})$
with $\sigma\in(0,1)$. The logarithm is concave, so $\ln x\le\ln x_0-1+x/x_0$
for every $x_0>0$; applied to $P+h$, to $q\le\ell$ and to $100+B$ it gives
$\ln u\le\ln P_0-1+(P+h)/P_0+\ln Q_0-1+\ell(S,L)/Q_0-\ln D$ and the bonus
term. Since $\sigma>0$ the first bound may replace $\ln u$ in the chord.
$\square$

For the affine rates $\ell=\bar r$ scaled as $q$. For the concave rate, $\ell$
is the tangent at $S_0$ whose slope is the right slope of the piecewise-linear
fill at $S_0$; that slope is a supergradient of a concave function, so the
tangent lies above $\bar r$ for every $S\ge0$. The lemma holds for every
choice of $P_0$, $S_0$ and $B_0$; the search takes the point
$\lambda(\hat P+h,\hat S,\hat B)$ where the ray from the origin to the box
maximum meets the surface $\kappa(c'+u)(100+B)=\tau_0$, found by bisection.
That choice affects only how tight the bound is; the search skips the test
when $\lambda<2^{-10}$ (see Numerics).

**Theorem LL (pruning).** Each group of a leader character gets the weight
$w_g=\max_{\text{card}}(a_Pp+a_Ss+a_B(b_{\mathrm{base}}+b_{\mathrm{lim}}))$ over its scanned cards, the
coefficients of Lemma LL3. The box of a group set takes the largest leader
power, skill and bonus of the character, the four largest group maxima of
distinct characters from Section 18.3, the largest leader-only, attribute and
support bonus, and the smallest leader skill. A character-level node sums
the leader's terms, the weights of the selected groups, the $r$ largest group
weights of distinct characters from the suffix start, and $a_B$ times the
attribute and support bound of Section 18.3. A card-level node sums the
terms of the chosen cards, the weights of the remaining planned groups and
$a_B$ times the plan's diversity bonus and the current support ceiling. If
the sum is below $\ln\tau-10^{-9}$, where $\tau\ge\tau_0$ is the event point
of the current threshold, no deck of the subtree has $E\ge\tau$, so every
leaf of the subtree is below the threshold and the subtree is pruned by
Theorem 1.

*Proof.* The coefficients are non-negative, so each card's terms are at most
its group's weight, the attribute and support terms are at most their
bounds (Sections 16, 18.2 and 18.5), and a deck takes at most one group of
each character, whose best weights of distinct characters the suffix list
ranks. The sum therefore bounds the right-hand side of Lemma LL3 for every
deck of the subtree, and decks with $E\ge\tau$ also have $E\ge\tau_0$. A leaf
whose event point is below that of the threshold is below the threshold.
$\square$

The group loop breaks on this test for the same reason as in Section 18.4:
the suffix lists and attribute rows only shrink as the start index grows. The
scan of a group stops when the rest maxima of Section 18.5 fail it, since the
weight is non-decreasing in every term. A bound built for $\tau_0$ holds at
every higher threshold, so the search rebuilds the weights only when the
event-point threshold has risen by $1/128$ since the last build, which
narrows the chord and moves the tangent point; every sum is recomputed from
the current weights, never mixed across builds. When $u_{\mathrm{lo}}\le0$ or
$u_{\mathrm{lo}}\ge u_{\mathrm{hi}}$ no log-linear test is made.

*Numerics.* The parameters $P_0$, $Q_0$, $B_0$ and $\sigma$ are `f64`
numbers, and Lemma LL3 holds for their exact values, except that $\sigma$,
$\varphi(t_{\mathrm{lo}})$ and the logarithms in $K$ carry an error of a few
units in the last place. The search uses a bound only when $\lambda\ge2^{-10}$,
so each weighted term $a_PP$, $a_SS$, $a_LL$ and $a_BB$ is at most $2^{10}$
($P_0$, $Q_0$ and $100+B_0$ are at least $\lambda$ times the box values),
and only when $|K|<2^8$. A sum of fewer than twenty such terms in
`f64` therefore has an absolute error below $10^{-11}$, which together with
the parameter errors stays far below the margin of $10^{-9}$.
`log_linear::tests` checks the bound against `ObjectiveBound::ceiling` at
random feature points for every supported live type and skill order.

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

Implementation: src/search/solver/numeric.rs (`SkillCeiling`,
`selected_value`, `skill_frontier`, `global_skill_upper`).

### 20.1 Objective

Let $s_i$ be the resolved score-up of member $i$ and $s_L$ that of the
leader. The encoded Skill objective is

$$
10s_L + 2\sum_{\text{other 4}} s_i
= 2\sum_{i=1}^5 s_i + 8s_L.
$$

Whatever rule fixes the leader (best skill, a forced leader character, the
first fixed slot, or the slot order of a fully fixed lineup), the leader is a
member, so $s_L\le\max_i s_i$. Hence integers $c_i\ge s_i$ give

$$
10v \le 2\sum_i c_i + 8\max_i c_i ,
$$

and by Lemma N6 the key obeys the same inequality.

### 20.2 Static ceiling

For every card, skill_max is an upper bound on its resolved value in every
deck:

- ordinary score-up: exact maximum;
- unit-count: maximum table entry;
- different-unit: maximum base plus increments, clamped to skill_max;
- reference skill: skill_min plus the reference maximum, which pool
  construction keeps within skill_max.

Let $G=\max_c skill\_max(c)$.

### 20.3 Composition-aware ceilings

Fix a node with selected cards $P$ and $r=5-|P|$ remaining slots, and let
$D=P\cup C$ be any legal completion, $|C|=r$. For a unit bit $u$ let
$c_u(P)$ be the number of cards of $P$ whose unit mask contains $u$, and
$n_u(D)$ the same count over $D$. A member counts once for every bit it
carries: a Virtual Singer card with a support unit carries the piapro bit and
its support-unit bit and counts toward both units, and no member counts twice
toward one unit. Therefore
$n_u(D)=c_u(P)+|\{y\in C: u\in units(y)\}|$, so

$$
\text{(U1)}\quad n_u(D)\le c_u(P)+r,
\qquad
\text{(U2)}\quad n_u(D)\le c_u(P)+[u\in units(y)]+(r-1)\ \text{ for } y\in C.
$$

**Unit-count skill.** A card whose skill counts unit $u$ with table
$T[1..5]$ resolves to $T[\operatorname{clamp}(n_u(D),1,5)]$. For every
integer $N\ge n_u(D)$, $\operatorname{clamp}(n_u(D),1,5)\le
\operatorname{clamp}(N,1,5)$, hence

$$
s\le M_T(\operatorname{clamp}(N,1,5)),
\qquad M_T(k)=\max_{1\le j\le k}T[j].
$$

Tables need not be non-decreasing (a table may peak below five members), so
the prefix maximum $M_T$ is required; $T[\operatorname{clamp}(N,1,5)]$ alone
would not be a bound. The card itself need not carry $u$; (U2) adds its own
bit only when it does. Taking $N$ from (U1) for a selected card and from (U2)
for an unselected one, and intersecting with skill_max (Lemma 1), gives the
ceiling.

**Different-unit skill.** Every member contributes at most one unit bit,
its member unit $m(y)$ (`member_unit`: the support unit of a Virtual Singer
card that has one, otherwise the card's unit). A card $x$ counts
$d(x,D)=|\{m(y): y\in D\setminus\{x\},\ m(y)\ne m(x)\}|$ units. With
$M_P=\bigcup_{y\in P}m(y)$, the members of $P\setminus\{x\}$ contribute a
subset of $M_P\setminus m(x)$ and every other member at most one more unit, so

$$
d(x,D)\le|M_P\setminus m(x)|+r\ \ (x\in P),
\qquad
d(x,D)\le|M_P\setminus m(x)|+(r-1)\ \ (x\in C).
$$

The resolved value $\min(skill\_max,\ b+i\cdot\min(2,d))$ is non-decreasing
in $d$, so substituting these bounds yields a ceiling.

**Other skills.** Ordinary and reference skills, and entries the evaluator
resolves to zero, keep skill_max.

Write $\kappa_P(x)$ for the ceiling of a selected card $x\in P$ and
$\kappa'_P(y)$ for that of an unselected card $y$ taken as one of the $r$
remaining picks. By construction $\kappa_P,\kappa'_P\le skill\_max$, and for
every legal completion $D$: $s_x\le\kappa_P(x)$ for $x\in P$ and
$s_y\le\kappa'_P(y)$ for $y\in C$.

### 20.4 Node bounds

Let $S_P=\sum_{x\in P}\kappa_P(x)$ and $L_P=\max_{x\in P}\kappa_P(x)$.
Integers throughout are at most $5\cdot255$.

**Global bound.** Every remaining pick resolves to at most $G$, so

$$
U_g = 2(S_P+rG) + 8\max(L_P,G)
$$

is admissible at every node, including nodes with fixed roles left. It permits
card reuse and ignores character uniqueness.

**Frontier bound.** When every remaining slot is free, the completion is
drawn from `cards[pos..]` and uses $r$ distinct unused characters, one card
each. For each unused character $h$ let
$m_h=\max\{\kappa'_P(y): y\in cards[pos..],\ char(y)=h\}$. Then
$\sum_{y\in C}s_y\le\operatorname{Top}_r\{m_h\}$ and
$\max_{y\in C}s_y\le\max_h m_h$, so

$$
U_f = 2\bigl(S_P+\operatorname{Top}_r\{m_h\}\bigr) + 8\max\bigl(L_P,\max_h m_h\bigr)
$$

is admissible. If fewer than $r$ unused characters remain, the node has no
completion.

The scan visits `cards[pos..]` in descending skill_max and keeps the best
$r$ characters seen, best-first; every other character seen has a maximum no
larger than the last kept value $t$. Once $r$ characters are kept and the next
card has skill_max at most $t$, every later card $y$ has
$\kappa'_P(y)\le skill\_max(y)\le t$. If $char(y)$ is kept, its maximum is
already at least $t$; otherwise its maximum stays at most $t$. Neither changes
$\operatorname{Top}_r\{m_h\}$ or $\max_h m_h$, so stopping the scan returns
the exact relaxation.

**Candidate break.** At a free layer the child that takes $x=cards[p]$
continues only with cards after $p$. Every deck of that child contains $P$
and $r$ further members, so (U1) and the different-unit bound for $x\in P$
still apply to the selected cards; $x$ and every later pick resolve to at most
their skill_max, which is at most $skill\_max(x)$ by the scan order. Hence

$$
B(p) = 2\bigl(S_P + r\cdot skill\_max(x)\bigr) + 8\max\bigl(L_P, skill\_max(x)\bigr)
$$

bounds the child and is non-increasing in $p$. The first child with
$B(p)<\tau$ ends the loop by Theorem 1.

### 20.5 Equality with the K-th result

For the Power and Skill targets the canonical key orders equal objectives by
the sorted public card set, then by the ordered ids and card variants. Let a
node's frontier or candidate bound equal $\tau$, and let every public set that
a completion could have be lexicographically larger than the K-th result's
public set $\pi_K$. `smallest_public_set` gives a lower bound on those sets:
the selected ids together with the smallest distinct unused ids of the
suffix minimize every rank of the sorted set at once. A completion $D$ is
no better than $\tau$; if it is strictly worse it cannot enter (Theorem 1).
If $v(D)=\tau$, its key exceeds the K-th key because its public set exceeds
$\pi_K$. It cannot replace a better representative of a retained public set
$E$ either: that needs $v(E)=\tau$ and $\pi(E)=\pi(D)$, but a retained
result with the K-th objective has $\pi(E)\le\pi_K<\pi(D)$. Such a node is
pruned; any node that might reach a public set at most $\pi_K$ is kept.

## 21. Unconstrained Power: scenario branch and bound

Implementation: src/search/solver/power.rs.

### 21.1 Scenarios

For a deck $D$ let $U(D)$ be the set of unit bits that every member's unit
mask contains (over the six bits the evaluator reads) and $A(D)$ the
attribute all five members share, if any. The evaluator resolves member $c$ to

$$
p(c,D)=\max_{w\in units(c)} V_c\bigl[prof_c(w)\bigr]\bigl[2\,[w\in U(D)]+[A(D)\text{ exists}]\bigr],
$$

where $V_c[\cdot][k]$ are the card's stored powers by profile and member key,
and $v(D)=\operatorname{clamp}(\sum_{c\in D}p(c,D)+H)$ with honor power $H$.

A scenario $S=(U,a)$ pairs a unit set $U$ with $a$, either "no shared
attribute" or one attribute. It admits the cards whose mask contains $U$ and,
when $a$ is an attribute, whose attribute is $a$. Every deck $D$ is admitted by
its own scenario $S(D)=(U(D),A(D))$: each member contains $U(D)$ and carries
$A(D)$.

$U(D)$ is the intersection of the members' masks. The search takes the empty
set and the distinct card masks, closed under pairwise intersection; every
intersection of card masks lies in that closure, so $S(D)$ is always among the
scenarios. A unit set need not be any card's own mask. A scenario that admits
fewer than five characters holds no deck and is dropped.

### Lemma 3 — scenario power ceiling

For a card $c$ admitted by $S=(U,a)$ let
$single(c;u,a)$ = `resolve_card_power_scenario` with all-member unit $u$ (or
none) and the attribute flag of $a$, and

$$
g_S(c)=\begin{cases}
single(c;\varnothing,a) & U=\varnothing,\
\max_{u\in U} single(c;u,a) & \text{otherwise.}
\end{cases}
$$

Then $p(c,D)\le g_S(c)$ for every deck $D$ with $S(D)=S$ and every member
$c$, with equality when $|U|\le1$.

**Proof.** $single(c;u,a)$ is the maximum over $w\in units(c)$ of
$V_c[prof_c(w)][2[w=u]+[a]]$. Take any term of the maximum defining
$p(c,D)$. If $w\in U$, it is the $w$-term of $single(c;w,a)$. If $w\notin U$,
it is the $w$-term of $single(c;u,a)$ for every $u\in U$, or of
$single(c;\varnothing,a)$ when $U=\varnothing$. Every term is therefore
bounded by $g_S(c)$. When $|U|\le1$ the single call uses exactly the member
keys of $D$. ∎

No order between member keys or between profiles is assumed. Two all-member
units occur when every member carries both, for example five Virtual Singer
cards with one support unit. Since clamp is non-decreasing,
$v(D)\le\operatorname{clamp}(\sum_{c\in D}g_{S(D)}(c)+H)$.

### Lemma 4 — best completion in a scenario

Inside a scenario the entries are sorted by $g_S$ descending, then by pool
index. For a prefix, the largest $g_S$-sum of $r$ further entries after a
position, from distinct characters outside the prefix, is obtained by taking
the first entry of each such character (its largest) and the $r$ largest of
those; the scan stops after $r$ characters.

**Proof.** A completion uses one entry per character. Replacing each entry by
the first entry of its character does not lower the sum, and among one value
per character the $r$ largest maximize the sum. ∎

### 21.2 Search and bounds

Each admitted scenario runs a depth-first search that picks entries in
increasing position with distinct characters and distinct public ids. Every
complete deck is evaluated exactly by the placement evaluator and inserted
into the one canonical TopKTracker; leaves are the only source of results.
Let $\tau$ be the tracker's K-th objective once it holds K public sets. Every
bound below is compared as $\operatorname{clamp}(\text{sum}+H)<\tau$:

- **Scenario ceiling.** The Lemma 4 value of the empty prefix; below $\tau$
  the scenario is skipped.
- **Node bound.** The selected $g_S$-sum plus the Lemma 4 completion of the
  remaining $r$ slots from the entries after the last pick. With no such
  completion the node has no deck.
- **Candidate break.** A child that takes the entry at position $p$
  continues only with later entries, none above $g_S$ of that entry, so every
  completion through it has sum at most the selected sum plus
  $r\cdot g_S(p)$. This quantity is non-increasing in $p$, and the first
  candidate below $\tau$ ends the loop.

Scenarios run in order of descending ceiling; any order is exact.

### Theorem 4 — scenario search exactness

Let $R$ be any member of the canonical Top-K, i.e. the best legal
representative of a Top-K public set. In $S(R)$ the cards of $R$ are admitted
and visited, and every bound on the path to $R$ is at least
$\sum_{c\in R}g_{S(R)}(c)$, which by Lemma 3 gives at least $v(R)$: the
ceiling and the node bounds by Lemma 4, and each candidate break because the
remaining members of $R$ come after the current entry. The cutoff never
decreases and ends at the K-th objective, which is at most $v(R)$, so no
comparison $\text{bound}<\tau$ removes $R$. Its leaf is evaluated exactly,
inserted, and kept by the canonical tracker. Other decks can be pruned or
visited in scenarios that are not their own; pruning there removes nothing
their own scenario needs, and a visit only inserts an exactly evaluated deck.
∎

**Equality.** Let a node's bound equal $\tau$ without the power cap, i.e.
$\text{sum}+H=\tau$ with no clamp. A deck $D$ of the scenario through the node
with $v(D)=\tau$ then has $\sum_{c\in D} g_S(c)\ge\tau-H$, so its completion
reaches the node's best completion. Each remaining character contributes at
most its largest entry after the node, and a set of character maxima reaches
the best sum only if every one is at least $\ell$, the smallest entry the
best completion takes. So every entry $D$ adds has $g_S\ge\ell$, a character
outside the prefix and an unselected public id, and its sorted public set is
at least the selected ids joined with the smallest such ids. If that set is
larger than the K-th public set $\pi_K$, or fewer such ids exist, $D$ cannot
enter the Top-K by the argument of Section 20.5, and the node is pruned. For
$R$ in $S(R)$ this happens on its path only if $v(R)=\tau$ and
$\pi(R)>\pi_K$, which the K-th key already excludes. A capped bound or one
above $\tau$ is never pruned by equality, so every other deck that ties the
K-th objective is still reached and ordered by the tracker's canonical key.
Entries of equal power are visited in public id order, which only decides how
soon small public sets are found.

**Cultivation variants.** Variants of one public card are separate entries
of one character. A deck holds at most one of them, and each is searched, so
the canonical representative of every public set is among the leaves.
Nothing is truncated per character or per partial state: the only removals
are the three bounds above and the feasibility checks.

## 22. Challenge bound frontier

Implementation: src/search/solver/challenge.rs.

Challenge search fixes one character and chooses five distinct public card ids.
Each character's cards are searched per composition regime of Section 13, so
every bound below reads the regime's per-card power bound in place of
`power_max`.

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

For the Score target of a Solo, Auto or Challenge live, a second ceiling reads
the chosen members' score-up maxima and, for the remaining $r$ slots, the $r$
largest powers and the $r$ largest score-up maxima among the candidates from
the current position. The remaining members' score-ups, largest first, are at
most those largest values rank by rank, and merging with the chosen values
keeps this, so the five members' score-ups sorted as $m_1\ge\dots\ge m_5$ are
at most the merged values $v_1\ge\dots\ge v_5$. The leader repeats one member,
at most $m_1$, so the six slot values sorted are at most
$(v_1,v_1,v_2,v_3,v_4,v_5)$ rank by rank. Every skill order pairs the slots
with the rates by some permutation, which is at most the sorted pairing, and
the sorted pairing is non-decreasing in each rank; together with the power
bound, Section 12 makes the result admissible. By Lemma 1 the branch is pruned
when either ceiling is strictly below the threshold.

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
| Candidate runs | search/dfs.rs, search/suffix.rs | equal bonus terms, run maxima and nested tails bound the rest of a run |
| Sorted Power / Skill break | search/dfs.rs | descending candidate component + fixed relaxed tail |
| No-event numerator | search/dfs.rs, search/suffix.rs | exact floor/division equivalence |
| Correlated Score bound | search/correlated.rs | linear relaxation + concave quadratic envelope |
| Event independent bound | search/suffix.rs | monotonic formula over componentwise maxima |
| Composition regimes | search/composition.rs, pool/card_pool.rs | per-regime member-key power bound, regime ceiling, shared tracker floor |
| SIMD threshold mask | simd.rs | vectorized scalar upper >= threshold |
| WL attribute matching | search/suffix.rs | every legal novel-attribute set induces a matching |
| WL support upper bound | search/suffix.rs, Final helpers | support can only stay or decrease as main deck grows |
| Exact bonus tiers | search/solver/bonus_tiers.rs | per-card key and slack, exact reachable-sum suffix table per regime, per-tier live ceiling |
| Final member dominance | search/dominance.rs, search/alternatives.rs | member-role substitution + legal leader rotations |
| Final leader/job bound | solver/final_chapter.rs | admissible character ceiling |
| Final attribute DP | solver/final_chapter.rs | exact isolated OR-union DP |
| Final character-loop break | solver/final_chapter.rs | nested group suffixes imply non-increasing character ceiling |
| Final card-group bound | solver/final_chapter.rs | independent per-group maxima + limited top-cap + support UB |
| Final group rest maxima | solver/final_chapter.rs | same attribute, rest maxima and a non-decreasing candidate ceiling bound the rest of a group |
| Final ranked-buffer break | solver/final_chapter.rs | candidates sorted by admissible UB; overflow candidates still visited |
| Final log-linear bound | search/log_linear.rs, solver/final_chapter.rs | Section 18.8: product form, chord and tangents of the logarithm, per-group weights |
| Numeric Power max/min | solver/numeric.rs | global max UB / global min LB |
| Numeric Skill | solver/numeric.rs | Section 20: composition-aware per-card ceilings, per-character frontier, candidate break, public-set equality rule |
| Power scenarios | solver/power.rs | Section 21: unit-set scenarios, Lemma 3 scenario ceiling, Lemma 4 completion, Theorem 4 |
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
| Exact bonus tiers | bonus_tiers.rs, exact_bonus.rs, fractional_bonus.rs |
| Final Chapter | exact_final_chapter.rs, role_constraints.rs, historical auto-leader counterexample |
| WL / Final cross-product | validation_oracle.rs, complete ordered Top-K with support profiles, constraints, variants and nonmonotone attributes |
| Power | exact_power.rs, power_scenarios.rs (Top-K against the exhaustive oracle with unit, attribute and two-unit sharing, cultivation variants, honor power and uniform-bonus MySekai; a deck that shares two units; a shared unit set that is no card mask) and the all-scene oracle matrix |
| Challenge | exact_challenge.rs and challenge-all timeout regression |
| SIMD equality | simd::tests::dispatched_mask_keeps_bounds_equal_to_threshold |
| Historical incomplete oracle | case7_audit.rs |
| Numeric admissibility | numeric_soundness.rs, handler/capacity.rs unit tests |
| Log-linear event-point bound | search/log_linear.rs unit tests against `ObjectiveBound::ceiling` for every supported live type and skill order |
| Skill ceilings | skill_composition.rs — every bound on the search path of every deck dominates its key and member values; Top-K against the exhaustive oracle with unit-count, different-unit, reference and two-unit cards, and an equal-objective variant at the public-set equality rule |

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
  $\hat E=\hat I\lambda\beta/100\le2^{30}$.

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

and likewise for the Average five-slot sum and leader rate and for each
sorted slot rate $\lceil\mathrm{fl}(\mathrm{fl}(r/100)\cdot10^6)\rceil$. The integer steps
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
  division at most 5 more. These sums, and the Average reference-skill share,
  add their terms in ascending order, so the value is the same for every
  order of the deck's free members;
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
and $T$ for the integer bonus handed to the ceiling, with $t\le T$ (Lemma N5).

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
   float value is at most $V_B(1+5.1u)$. $V_B$ is a multiple of $10^{-4}$, so
   $V_B\le\lfloor V_B\rfloor+1-10^{-4}$ unless it is an integer. Therefore
   $I_e\le I_B$ whenever $5.1u\,V_B<10^{-4}$, that is for
   $V_B<1.7\cdot10^{11}$; (D4) gives $V_B\le2^{27}$.
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
diversity bound, and $S_B$ a left-to-right `f64` sum over a support list. Every
support sum is computed directly, never updated by subtraction, so $t\le T$:

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

This covers the suffix support bounds, the Final Chapter leader helpers, the
Final Chapter card-level plan and the build-time extra bonus bound. The plan
keeps the sum of its selected prefix; a new card whose game id is not among the
entries summed so far leaves that sum's operation sequence, hence its value,
unchanged, and any other card makes the plan sum the list again. Consequently the Bonus key satisfies
$\mathrm{round}(2t)\le2T$, and the MySekai value, a monotone `f64` chain in its
power and bonus inputs, is dominated by F2.

### 29.8 Lemma N6 — Skill key

The Skill key is $\lfloor\mathrm{fl}(\mathrm{fl}(10v)+10^{-6})\rfloor$, where
$v$ is the leader's score-up plus $0.2$ times each other score-up, added in
ascending order. With integral or quarter-integral score-ups, the exact $10v^*$
is a multiple of $1/2$ and at most the ceiling $2S+8L$. For $10v^*<10^5$ the
float error is below $10^{-9}$, so the float value lies in
$(\lfloor10v^*\rfloor,\lfloor10v^*\rfloor+1)$ and the key is at most
$\lfloor10v^*\rfloor\le2S+8L$.

### 29.9 Lemma N7 — correlated bound

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

### 29.10 Scope

Everything else in the search is integer arithmetic on exact discrete state.
Floating-point values enter a ceiling only through the aggregate objective
coefficients (N1–N4, N6), the correlated coefficients (N7), support sums
(N5) and the MySekai value (N5). A MySekai live has no live score: the
evaluator and every live-score ceiling use 0 for it.

Verification: src/search/tests/numeric_soundness.rs enumerates every deck of
generated pools under decimal constants and asserts packed and component-wise
dominance for every live type, skill order and target; targets exact-integer
and near-integer grid points of the live numerator and of the event stages;
checks the correlated bound and the support ceilings at every prefix; and
compares complete searches, including a Final Chapter support profile whose
remaining sum rounds above an integer,
with the exhaustive oracle.
