import Allium.Search

/-!
# Interrupted search

The deadline oracle may return any Boolean. A stopped left subtree is never
silently turned into Complete by the right subtree. Partial results contain
only evaluated seeds/leaves; Complete additionally agrees with normal search.
This models the semantic boundary, not a wall-clock or OS timer.
-/
namespace Allium

variable {Candidate Identity : Type*} [LinearOrder Candidate] [DecidableEq Identity]

structure SearchOutcome (Candidate : Type*) where
  retained : Finset Candidate
  deadlineHit : Bool

/-- Observe a deadline before each nonempty tree; propagate the first expiry. -/
def searchInterruptible (identity : Candidate → Identity) (score : Candidate → ℕ)
    (k : ℕ) (expired : SearchTree Candidate → Bool) (retained : Finset Candidate) :
    SearchTree Candidate → SearchOutcome Candidate
  | .empty => ⟨retained, false⟩
  | .leaf candidate =>
      if expired (.leaf candidate) then ⟨retained, true⟩
      else ⟨insert candidate retained, false⟩
  | .branch upper left right =>
      if expired (.branch upper left right) then ⟨retained, true⟩
      else if k ≤ (strictWitnesses identity score retained upper).card then
        ⟨retained, false⟩
      else
        let first := searchInterruptible identity score k expired retained left
        if first.deadlineHit then first
        else searchInterruptible identity score k expired first.retained right

/-- Legality of partial incumbents needs no optimality assumption. -/
theorem interruptible_valid (identity : Candidate → Identity) (score : Candidate → ℕ)
    (k : ℕ) (expired : SearchTree Candidate → Bool) (tree : SearchTree Candidate)
    (retained : Finset Candidate) :
    retained ⊆ (searchInterruptible identity score k expired retained tree).retained ∧
    (searchInterruptible identity score k expired retained tree).retained ⊆
      retained ∪ tree.leaves := by
  classical
  induction tree generalizing retained with
  | empty => simp [searchInterruptible, SearchTree.leaves]
  | leaf candidate =>
      by_cases hx : expired (.leaf candidate) = true
      · simp [searchInterruptible, hx, SearchTree.leaves]
      · simp only [searchInterruptible, if_neg hx, SearchTree.leaves]
        constructor
        · exact Finset.subset_insert _ _
        · intro x hx
          simpa only [Finset.mem_insert, Finset.mem_union, Finset.mem_singleton,
            or_comm] using hx
  | branch upper left right ihLeft ihRight =>
      by_cases hx : expired (.branch upper left right) = true
      · simp [searchInterruptible, hx, SearchTree.leaves]
      · by_cases hp : k ≤ (strictWitnesses identity score retained upper).card
        · simp [searchInterruptible, hx, hp, SearchTree.leaves]
        · let first := searchInterruptible identity score k expired retained left
          rcases ihLeft retained with ⟨hlgrow, hlvalid⟩
          rcases ihRight first.retained with ⟨hrgrow, hrvalid⟩
          simp only [searchInterruptible, if_neg hx, if_neg hp, SearchTree.leaves]
          change retained ⊆ (if first.deadlineHit then first else
            searchInterruptible identity score k expired first.retained right).retained ∧
            (if first.deadlineHit then first else
              searchInterruptible identity score k expired first.retained right).retained ⊆
              retained ∪ (left.leaves ∪ right.leaves)
          by_cases hf : first.deadlineHit = true
          · simp only [if_pos hf]
            refine ⟨hlgrow, ?_⟩
            intro candidate hc
            rcases Finset.mem_union.mp (hlvalid hc) with hs | hl
            · exact Finset.mem_union_left _ hs
            · exact Finset.mem_union_right _ (Finset.mem_union_left _ hl)
          · simp only [if_neg hf]
            refine ⟨hlgrow.trans hrgrow, ?_⟩
            intro candidate hc
            rcases Finset.mem_union.mp (hrvalid hc) with hm | hr
            · rcases Finset.mem_union.mp (hlvalid hm) with hs | hl
              · exact Finset.mem_union_left _ hs
              · exact Finset.mem_union_right _ (Finset.mem_union_left _ hl)
            · exact Finset.mem_union_right _ (Finset.mem_union_right _ hr)

/-- Completion is operational: it implies equality with the uninterrupted
algorithm, not merely that the returned vector happens to be nonempty. -/
theorem complete_agrees_search (identity : Candidate → Identity) (score : Candidate → ℕ)
    (k : ℕ) (expired : SearchTree Candidate → Bool) (tree : SearchTree Candidate)
    (retained : Finset Candidate)
    (hcomplete : (searchInterruptible identity score k expired retained tree).deadlineHit = false) :
    (searchInterruptible identity score k expired retained tree).retained =
      search identity score k retained tree := by
  induction tree generalizing retained with
  | empty => rfl
  | leaf candidate =>
      by_cases hx : expired (.leaf candidate) = true
      · simp [searchInterruptible, hx] at hcomplete
      · simp [searchInterruptible, hx, search]
  | branch upper left right ihLeft ihRight =>
      by_cases hx : expired (.branch upper left right) = true
      · simp [searchInterruptible, hx] at hcomplete
      · by_cases hp : k ≤ (strictWitnesses identity score retained upper).card
        · simp [searchInterruptible, hx, hp, search]
        · let first := searchInterruptible identity score k expired retained left
          simp only [searchInterruptible, if_neg hx, if_neg hp] at hcomplete ⊢
          change (if first.deadlineHit then first else
            searchInterruptible identity score k expired first.retained right).deadlineHit = false
            at hcomplete
          change (if first.deadlineHit then first else
            searchInterruptible identity score k expired first.retained right).retained = _
          by_cases hf : first.deadlineHit = true
          · simp [hf] at hcomplete
          · simp only [if_neg hf] at hcomplete ⊢
            have hleft : first.deadlineHit = false := Bool.eq_false_iff.mpr hf
            rw [ihRight first.retained hcomplete, ihLeft retained hleft]
            simp only [search, if_neg hp]

/-- The public mathematical completion theorem. There is deliberately no
exactness assertion for TimedOut. -/
theorem complete_exact (identity : Candidate → Identity) (score : Candidate → ℕ)
    (horder : ∀ a b, score b < score a → a < b) (k : ℕ)
    (expired : SearchTree Candidate → Bool) (tree : SearchTree Candidate)
    (hsound : tree.Sound score) (seeds : Finset Candidate)
    (hseeds : seeds ⊆ tree.leaves)
    (hcomplete : (searchInterruptible identity score k expired seeds tree).deadlineHit = false) :
    topK identity k (searchInterruptible identity score k expired seeds tree).retained =
      topK identity k tree.leaves := by
  rw [complete_agrees_search identity score k expired tree seeds hcomplete]
  exact search_exact identity score horder k tree hsound seeds hseeds

/-- With legal seeds, even a timeout cannot fabricate an invalid deck. -/
theorem timed_out_results_legal (identity : Candidate → Identity) (score : Candidate → ℕ)
    (k : ℕ) (expired : SearchTree Candidate → Bool) (tree : SearchTree Candidate)
    (seeds : Finset Candidate) (hseeds : seeds ⊆ tree.leaves) :
    (searchInterruptible identity score k expired seeds tree).retained ⊆ tree.leaves := by
  intro candidate hc
  rcases Finset.mem_union.mp ((interruptible_valid identity score k expired tree seeds).2 hc)
    with hs | hl
  · exact hseeds hs
  · exact hl

end Allium
