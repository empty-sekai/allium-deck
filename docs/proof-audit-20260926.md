# Proof audit: 2026-09-26

Base audited: `165ff63525be2c13d4ae9b6417924e0b789e5ff6`.

This review compares the implementation with `exactness-proof.md` and
`pruning-proof.md`. It does not change the leaf evaluator to make search
counterexamples disappear. The two reproduced wrong-result cases are distinct
from arithmetic hazards and incomplete arguments found during the repair.

## Reproduced wrong results

### Floating-point support compensation

Six cards give two possible public sets. Cards 100 and 200 have the same
character, attribute, unit mask, power (90,000 in every context) and skill (100).
Their base bonuses are 64.1% and 62.5%; four other characters have no bonus.
The support profile, count 4, is:

```text
(100, 7.05), (200, 5.45), (300, 2.05), (301, 0.8), (302, 0.5)
```

Over the reals both choices total 72.9%. The unchanged evaluator produces
72.89999999999999147 and 72.90000000000000568, and MYSEKAI values 1728 and
1729. In Final Chapter with character 2 fixed as leader, the old dominance
predicate deleted card 200 and returned `Complete` with 1728. The exhaustive
ordered oracle and dominance-disabled search returned 1729.

Repair: support order-statistic monotonicity handles the no-loss case; a
positive loss requires a strict outward-certified surplus exceeding the
proved two-deck floating error. Malformed profiles cannot certify deletion.
See pruning-proof Theorems 3a and 3b. The inverse substitution theorem now
inherits preservation of the implemented objective, not just a real formula.

Permanent tests: `proof_audit::support_compensation_preserves_actual_floating_point_top_one`,
`strict_support_surplus_still_allows_certified_dominance`, and malformed-profile
regressions. The first also exercises K > 1, ordinary WL and other targets.

### Public ID 65535 is not an empty slot

The eight `(public ID, power)` rows are:

```text
(101,107), (106,107), (104,105), (102,109),
(107,108), (105,101), (103,110), (65535,105)
```

They have distinct characters and identical composition contexts. Power K=20
previously returned `Complete`, with rank 20 equal to
`534 / [101,103,104,106,65535]` instead of the canonical
`534 / [101,102,104,107,65535]`.

Repair: both Power and generic numeric ID frontiers use `SmallestIds` with an
explicit occupied length. Every `u16` remains representable. No new candidate
filter or reduced public-ID range is introduced.

Permanent tests include the exact failing box, deterministic tied boxes,
Power/Skill/MYSEKAI, constrained Power, minimization, and the builder's acceptance
of public ID 65535. See pruning-proof Sections 3.3, 20.5 and 21.2.

## Additional repair obligations

| Area | Defect or missing argument | Repair |
| --- | --- | --- |
| Log-linear bound | A fixed epsilon and a few-ulp claim do not certify a ratio of nearly equal logarithms | Outward interval preparation, an explicit atanh-series logarithm remainder, certified endpoint widening, and fallback for an uncertifiable chord |
| Log-bound consumers | Ordinary sums and last-group subtraction also need an error argument | Products/sums upward; threshold logarithm and threshold-minus-fixed terms downward at every consumer |
| Event cutoff inversion | Probe values can exceed legal-leaf maxima; narrowing event stages could wrap | Shared `i128` stages and a final monotone clip above every supported leaf output |
| Optional joint bound | Large coefficient/rational intermediates are not covered merely by bounded final scores | Checked coefficient/product/denominator operations; decline or return an infinite ceiling on overflow |
| Support-only identity | A card excluded from the main pool could still have its support ID clamped before deduplication | Checked conversion before constructing support seeds, including the standalone support operation |
| Raw character identity | A malformed character could be skipped before the later representation check | Validate before the unit-mask preparation filter |
| Root recovery proof | Numeric threshold alone does not establish membership in canonical Top-K | Full-order counting argument and explicit cultivation enumeration before inverse-score pruning |
| Floating operation count | Reference shares are not generally quarter-integral; Multi Average adds another dependency chain | Count the whole dependency path conservatively and distinguish loose bounds from legal-leaf maxima |
| Evidence reporting | Missing external data returned from tests as if successful | Explicit ignored tests; an explicit invocation without prerequisites fails |

The log-linear analytic derivation remains in Section 18.8. Its machine
arithmetic now follows that derivation using interval enclosures. No platform
`ln` accuracy assumption or heuristic epsilon authorizes pruning. No claim of a
third end-to-end wrong-result counterexample is made for the old log bound.

## Verification

The core patch was checked on native x86-64, Rust 1.98.1, one pinned logical
CPU with an 80% quota and a 1536 MiB memory limit, without swap:

```sh
cargo fmt --all --check
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo test --locked --release --all-features
cargo test --locked --release --all-features --lib \
  long_exact_all_scene_property_matrix -- --ignored --nocapture --test-threads=1
ALLIUM_VALIDATION_ORACLE_SEEDS=16 cargo test --locked --release --all-features \
  --lib validation_oracle_extended -- --ignored --nocapture --test-threads=1
```

The native unit suite passes 308 tests with 8 explicitly ignored. The default
integration suite runs its corpus-classification test and explicitly ignores
three external-masterdata proofs; one doctest passes. The all-scene matrix
passes 256 cases / 3840 comparisons. Extended-matrix and independent runtime
results are recorded in the PR verification section as those checks finish.

The ordinary PR/push CI now executes both exhaustive synthetic matrices.
`ExactOracle` independently enumerates placements and canonical public sets,
but shares the leaf evaluator. It proves search equivalence only on the tested
finite boxes, not independent correctness of the game's scoring rules.

## Scope

The low-level search theorem assumes a pool and context preserving the handler's
numeric, representation, support-order and alignment invariants. It is not a
promise for arbitrary invalid mutations of `SearchContext`.

This review repairs the counterexamples and the identified proof obligations;
it is not a machine-checked whole-program proof or a claim that no other bug
exists. No real-account corpus, production-tail latency, or real-browser
release acceptance is inferred from synthetic/native tests. In particular,
this correctness change does not certify a 20 ms performance target.
