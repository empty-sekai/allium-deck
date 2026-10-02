# Formal Verification Checkpoint

This branch contains a partial formal-verification development snapshot. It is not a complete verification of the engine.

## Coverage

- **Proved:** P03, P09, P13, P19, P40, S01, S02, S03, S04.
- **Partial:** 29 obligations.
- **Open:** 8 obligations.
- **Outside the first-tier scope:** R01, binary/compiler refinement.
- **First-tier complete:** false; 37 obligations remain incomplete.

`coverage.json` is the authoritative obligation inventory and pins the source bytes used by the models. Its `baseline_commit` identifies the original inventory baseline, not a substitute for those per-file hashes.

## Verified snapshot

The complete verifier was executed with Lean 4.24.0 and the locked Mathlib dependencies:

- 41 source pins and 43 imported proof modules checked.
- Build completed successfully.
- 3,906 declarations and 1,914 theorems passed the transitive axiom audit.
- 17 proof-certificate type checks passed.
- All 13 negative tests passed.
- `--self-test --require-complete` exited with code 1 because the remaining 37 obligations are incomplete.

The only permitted axioms are `propext`, `Classical.choice`, and `Quot.sound`. These results do not establish the missing mathematical connections or compiler refinement.

## P03 construction contract

`ProofContracts.P03` combines capacity safety with the concrete construction path:

`SkillProducer.buildSkill` → `PreparedSkills.prepare` → intermediates → `SourceNumeric.build` → gather.

`PreparedSkills.build_success` derives source provenance, produced skill shape, and final slot usability from the concrete outer build-success equation. `build_failure_iff` describes failure propagation. The original internal gather domain still permits raw slots independently of skill Options; producer validity is derived for constructed inputs rather than imposed on all internal inputs.

## P38 continuation point

`PowerScenarios.lean` contains compiled components for:

1. Growing unit-intersection worklists and seven attribute slots, including coverage of each real five-card deck's required scene.
2. Sorted best-completion scans, maximum sums, and the least-power property of optimal completions.
3. Production power summaries, stored-column provenance, Binary64 acceptance thresholds, and u32 bounds.
4. The fixed-capacity smallest-distinct-ID collector and local rejection against the full Power comparison key.
5. Actual-composition leaf evaluation and report-order permutation invariance.
6. Bounded tracker insertion, replacement, truncation, and observed-key invariants.
7. `SourceDFS` control flow, `SelectionValid` root/skip/choose lemmas, and `loop_preserves_observed`, `descend_preserves_observed`, and `descend_observed_origin`.

These components are not a P38 completion certificate. The remaining work is:

1. Instantiate selection and suffix invariants from actual scene roots.
2. Define an independent finite set of legal five-card results and connect every observed leaf to it.
3. Discharge the local pruning lemmas' range, legal-completion, threshold, and tracker-witness premises from reachable states.
4. Prove include/skip and cross-scene required-class coverage against that independent legal set.
5. Connect uninterrupted execution to complete canonical Top-K. Interrupted execution must retain a distinct partial-result statement.
6. Define and prove `ProofContracts.P38` and its typed certificate only after those connections are complete.

In particular, entry-level five-card provenance does not yet prove legal-result coverage. The explicit 49-regime model in `ConcretePower` is not a replacement for this specialized production path.

## Reproduce

```sh
cd formal/lean
lake exe cache get
python verify.py --self-test
python verify.py --self-test --require-complete
```

The first verifier command checks the declared partial scope. The strict command must remain unsuccessful until every first-tier obligation is closed. Do not change coverage statuses, remove obligations, or weaken the certificate checks to obtain a complete result.
