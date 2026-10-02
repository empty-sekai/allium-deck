import Allium.RootForest

/-!
# Cultivation groups before inverse substitutions

Every dense state of a public card is enumerated before traversing its inverse
substitutions. Empty stored groups mean a singleton, not an empty candidate
set. The coverage result is combinatorial; objective monotonicity and legal
root-seed coverage are separate obligations.
-/
namespace Allium.VariantRecovery

variable {count : Nat}

def family (publicId : Fin count → Nat) (card : Fin count) : Finset (Fin count) :=
  Finset.univ.filter (fun candidate => publicId candidate = publicId card)

@[simp] theorem family_mem (publicId : Fin count → Nat) (reference card : Fin count) :
    card ∈ family publicId reference ↔ publicId card = publicId reference := by simp [family]

/-- Source storage leaves one-state groups empty to avoid a special traversal. -/
def storedGroup (publicId : Fin count → Nat) (card : Fin count) : Finset (Fin count) :=
  if 1 < (family publicId card).card then family publicId card else ∅

def choices (publicId : Fin count → Nat) (card : Fin count) : Finset (Fin count) :=
  if (storedGroup publicId card).Nonempty then storedGroup publicId card else {card}

theorem choices_exact (publicId : Fin count → Nat) (card : Fin count) :
    choices publicId card = family publicId card := by
  have hm : card ∈ family publicId card := by simp
  by_cases h : 1 < (family publicId card).card
  · have hn : (family publicId card).Nonempty := ⟨card, hm⟩
    simp [choices, storedGroup, h, hn]
  · have small : (family publicId card).card ≤ 1 := by omega
    have he : family publicId card = {card} := by
      apply Finset.Subset.antisymm
      · intro other ho
        exact Finset.mem_singleton.mpr ((Finset.card_le_one.mp small) other ho card hm)
      · exact Finset.singleton_subset_iff.mpr hm
    simp [choices, storedGroup, he]

def cultivationDecks (publicId : Fin count → Nat) (roots : List (Fin count)) :
    Finset (List (Fin count)) := DP.assignments (roots.map (choices publicId))

/-- Every and only pointwise public-ID-equivalent cultivation combination is
visited, independently of the score of the stored representative. -/
theorem cultivation_exact (publicId : Fin count → Nat) (roots deck : List (Fin count)) :
    deck ∈ cultivationDecks publicId roots ↔ deck.map publicId = roots.map publicId := by
  unfold cultivationDecks
  rw [DP.mem_assignments]
  induction roots generalizing deck with
  | nil => simp
  | cons first rest ih =>
      cases deck with
      | nil => simp
      | cons card deck =>
          simp only [List.map_cons, List.forall₂_cons, List.cons.injEq]
          rw [choices_exact, family_mem, ih]

/-- Per-root cultivation groups are not globally deduplicated by public set
before inverse substitution: each complete combination gets its own tree. -/
def recover (forest : RootForest.Forest count) (storedRoots : List (Fin count)) :
    Finset (List (Fin count)) :=
  (cultivationDecks forest.publicId storedRoots).biUnion
    (fun roots => DP.assignments (roots.map (RootForest.options forest)))

/-- The needed parent-root cultivation combination is among those enumerated,
then the compressed inverse fibers reconstruct the original deck in all slots. -/
theorem recover_covers (forest : RootForest.Forest count) (storedRoots deck : List (Fin count))
    (publicRoots : (deck.map (RootForest.root forest)).map forest.publicId = storedRoots.map forest.publicId) :
    deck ∈ recover forest storedRoots := by
  apply Finset.mem_biUnion.mpr
  refine ⟨deck.map (RootForest.root forest), ?_, ?_⟩
  · exact (cultivation_exact forest.publicId storedRoots _).mpr publicRoots
  · exact RootForest.inverse_fibers_cover forest deck

/-- Every recovered deck carries an explicit cultivation-root witness; this
is stronger than retaining only one representative of the root public set. -/
theorem recovered_witness (forest : RootForest.Forest count) (storedRoots deck : List (Fin count))
    (member : deck ∈ recover forest storedRoots) :
    ∃ roots : List (Fin count), roots.map forest.publicId = storedRoots.map forest.publicId ∧
      deck ∈ DP.assignments (roots.map (RootForest.options forest)) := by
  obtain ⟨roots, hc, hd⟩ := Finset.mem_biUnion.mp member
  exact ⟨roots, (cultivation_exact forest.publicId storedRoots roots).mp hc, hd⟩

end Allium.VariantRecovery
