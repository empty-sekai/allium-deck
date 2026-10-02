import Allium.ProofContracts

namespace Allium.ProofCertificates

/-- These proofs inhabit the separately stated contract propositions. -/
theorem p09 : ProofContracts.P09 :=
  ⟨CompactDelta.production_delta_exact, CompactDelta.production_address, CompactDelta.production_budget_iff⟩

theorem p13 : ProofContracts.P13 := Arithmetic.numerator_prune

theorem p19 : ProofContracts.P19 :=
  ⟨SimdMask.unsigned_halves_eq_scalar, SimdMask.surviving_iff, SimdMask.threshold_refresh⟩

theorem p40 : ProofContracts.P40 :=
  fun k groups pool => group_topK_merge Canonical.identity k groups pool

theorem s01 : ProofContracts.S01 :=
  ⟨Canonical.result_collection_exact, Canonical.result_count, Canonical.key_lt_iff⟩

theorem s02 : ProofContracts.S02 :=
  fun k tree sound seeds hseeds =>
    search_exact Canonical.identity Canonical.score Canonical.score_order k tree sound seeds hseeds

theorem s03 : ProofContracts.S03 :=
  ⟨fun k expired tree seeds hseeds =>
      timed_out_results_legal Canonical.identity Canonical.score k expired tree seeds hseeds,
   fun k expired tree sound seeds hseeds complete =>
      complete_exact Canonical.identity Canonical.score Canonical.score_order k
        expired tree sound seeds hseeds complete⟩

theorem s04 : ProofContracts.S04 :=
  ⟨Enumeration.exhaustive_complete, Enumeration.exhaustive_keys_complete⟩

theorem mixedScenes : ProofContracts.MixedScenes :=
  ⟨MixedPower.regimes_cover, MixedPower.regime_bound_sound, MixedPower.multi_bound_eq_filtered,
    SidecarLayout.source_plan_covers, MixedPower.suffix_bound_sound, ScenarioPower.source_full_deck_bound⟩

theorem mixedExtrema : ProofContracts.MixedExtrema :=
  fun mode mixed common attr card =>
    ⟨MixedPower.effective_lower mode mixed common attr card,
     MixedPower.effective_upper mode mixed common attr card⟩

theorem gateSelection : ProofContracts.GateSelection :=
  ⟨fun rows best selected =>
      ⟨Gate.highest_mem rows best selected, Gate.highest_ge rows best selected⟩,
    Gate.no_fallback_from_missing_highest⟩

theorem cardCapacity : ProofContracts.CardCapacity :=
  ⟨Admission.CardTable.validate_iff, (fun _ _ => Admission.Intern.from_empty_invariants),
    (fun _ _ => Admission.Intern.intern_invariants), (fun _ _ => Admission.Intern.run_lookup_stable),
    Admission.CardTable.dense_address, Admission.LimitedCode.accepted_packing,
    Admission.PowerEncoding.row_decode, Admission.SkillGather.source_equal_iff,
    Admission.SkillGather.concrete_from_empty, Admission.GatherOrder.sort_preserves_validation,
    Admission.GatherPipeline.gather_success, Admission.NumericFlow.source_order⟩

theorem constructionCapacity : ProofContracts.ConstructionCapacity :=
  ⟨cardCapacity, Admission.Construction.success, Admission.SourceNumeric.build_success_iff,
    Admission.SourceNumeric.build_failure_iff, Admission.SourceNumeric.nonnegative_failure,
    Admission.SourceNumeric.maximum_failure, Admission.SkillGather.fresh_failure⟩

theorem p03 : ProofContracts.P03 :=
  ⟨constructionCapacity, Admission.PreparedSkills.build_success, Admission.PreparedSkills.build_failure_iff⟩

theorem rawMembershipPower : ProofContracts.RawMembershipPower :=
  ⟨MixedPower.from_raw_units, MixedPower.effective_lower_for,
    MixedPower.empty_minimum_zero, MixedPower.empty_legacy_zero⟩

theorem finaleInheritance : ProofContracts.FinaleInheritance :=
  ⟨SourceRows.finale_first_eligible, SourceRows.finale_output_domain,
    fun sources characters destination first next nonpositive positive =>
      SourceRows.nonpositive_then_positive sources characters destination first next nonpositive
        ((SourceRows.eligible_iff sources characters next).mpr positive)⟩

theorem suffixScans : ProofContracts.SuffixScans :=
  ⟨SuffixScan.production_prefix_exact, SuffixScan.effective_scan_bound, SuffixScan.scan_bound_antitone⟩

theorem numericGlobalPower : ProofContracts.NumericGlobalPower :=
  ⟨NumericPower.maximizing_prune, NumericPower.minimizing_prune, NumericSlots.complete_pool_domain⟩

theorem checkedRootPower : ProofContracts.CheckedRootPower :=
  fun _ => DominanceLoop.checked_scan_power

end Allium.ProofCertificates
