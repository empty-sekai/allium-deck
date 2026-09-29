import Allium.TopK

/-!
# A branch-and-bound kernel

This is a mathematical search implementation, not an extraction of Rust.
The tree contains concrete candidates. A branch's bound is required to be
proved by `SearchTree.Sound`, which is not an axiom. The kernel obtains
K-distinct-identity witnesses from already evaluated candidates before pruning.
-/

namespace Allium

variable {Candidate Identity : Type*} [LinearOrder Candidate] [DecidableEq Identity]

inductive SearchTree (Candidate : Type*) where
  | empty
  | leaf (candidate : Candidate)
  | branch (upper : ℕ) (left right : SearchTree Candidate)

def SearchTree.leaves : SearchTree Candidate → Finset Candidate
  | .empty => ∅
  | .leaf candidate => {candidate}
  | .branch _ left right => left.leaves ∪ right.leaves

/-- An upper-bound obligation at every internal node. -/
def SearchTree.Sound (score : Candidate → ℕ) : SearchTree Candidate → Prop
  | .empty => True
  | .leaf _ => True
  | .branch upper left right =>
      (∀ candidate ∈ left.leaves ∪ right.leaves, score candidate ≤ upper) ∧
      left.Sound score ∧ right.Sound score

/-- Exact branch-and-bound, with a mathematical set of evaluated candidates. -/
def search (identity : Candidate → Identity) (score : Candidate → ℕ) (k : ℕ)
    (retained : Finset Candidate) : SearchTree Candidate → Finset Candidate
  | .empty => retained
  | .leaf candidate => insert candidate retained
  | .branch upper left right =>
      if k ≤ (strictWitnesses identity score retained upper).card then retained
      else search identity score k (search identity score k retained left) right

/-- Search neither fabricates a leaf nor removes a seed. Every unvisited
leaf has a coverage certificate in the final retained pool. -/
theorem search_spec (identity : Candidate → Identity) (score : Candidate → ℕ)
    (horder : ∀ a b, score b < score a → a < b) (k : ℕ)
    (tree : SearchTree Candidate) (hsound : tree.Sound score)
    (retained : Finset Candidate) :
    retained ⊆ search identity score k retained tree ∧
    search identity score k retained tree ⊆ retained ∪ tree.leaves ∧
    ∀ candidate ∈ tree.leaves,
      Covered identity k (search identity score k retained tree) candidate := by
  classical
  induction tree generalizing retained with
  | empty =>
      simp only [search, SearchTree.leaves, Finset.union_empty]
      exact ⟨(fun _ h => h), (fun _ h => h), by simp⟩
  | leaf candidate =>
      simp only [search, SearchTree.leaves]
      refine ⟨Finset.subset_insert _ _, ?_, ?_⟩
      · intro x hx
        simpa only [Finset.mem_insert, Finset.mem_union, Finset.mem_singleton,
          or_comm] using hx
      · intro x hx
        have heq : x = candidate := Finset.mem_singleton.mp hx
        subst x
        exact covered_of_mem identity k (Finset.mem_insert_self _ _)
  | branch upper left right ihLeft ihRight =>
      rcases hsound with ⟨hub, hl, hr⟩
      by_cases hprune : k ≤ (strictWitnesses identity score retained upper).card
      · simp only [search, if_pos hprune, SearchTree.leaves]
        refine ⟨(fun _ h => h), Finset.subset_union_left, ?_⟩
        intro candidate hc
        exact strict_upper_prune identity score horder k retained candidate upper
          (hub candidate hc) hprune
      · simp only [search, if_neg hprune, SearchTree.leaves]
        rcases ihLeft hl retained with ⟨hlgrow, hlvalid, hlcover⟩
        rcases ihRight hr (search identity score k retained left) with
          ⟨hrgrow, hrvalid, hrcover⟩
        refine ⟨hlgrow.trans hrgrow, ?_, ?_⟩
        · intro candidate hc
          rcases Finset.mem_union.mp (hrvalid hc) with hmid | hright
          · rcases Finset.mem_union.mp (hlvalid hmid) with hseed | hleft
            · exact Finset.mem_union_left _ hseed
            · exact Finset.mem_union_right _ (Finset.mem_union_left _ hleft)
          · exact Finset.mem_union_right _ (Finset.mem_union_right _ hright)
        · intro candidate hc
          rcases Finset.mem_union.mp hc with hleft | hright
          · exact covered_mono identity k hrgrow (hlcover candidate hleft)
          · exact hrcover candidate hright

/-- Exactness of the mathematical search kernel for every K, including zero.
An Allium mode still has to discharge leaf enumeration and concrete bounds. -/
theorem search_exact (identity : Candidate → Identity) (score : Candidate → ℕ)
    (horder : ∀ a b, score b < score a → a < b) (k : ℕ)
    (tree : SearchTree Candidate) (hsound : tree.Sound score)
    (seeds : Finset Candidate) (hseeds : seeds ⊆ tree.leaves) :
    topK identity k (search identity score k seeds tree) =
      topK identity k tree.leaves := by
  rcases search_spec identity score horder k tree hsound seeds with
    ⟨_, hvalid, hcover⟩
  apply coverage_exactness identity k _ hcover
  intro candidate hc
  rcases Finset.mem_union.mp (hvalid hc) with hseed | hleaf
  · exact hseeds hseed
  · exact hleaf

end Allium
