import Allium.Arithmetic
import Allium.DynamicProgramming

/-!
# Dominance roots and compressed inverse fibers

The deletion guard never replaces a (public ID, dense ID) pair by a larger
pair. Distinct cards therefore decrease a concrete natural rank. Following
parents terminates at a retained card, and the compressed inverse table is
exactly the fiber of that root function.
-/
namespace Allium.RootForest

variable {count : Nat}

/-- Public-ID first, then dense-ID ordering, encoded without field overlap. -/
def rank (publicId : Fin count → Nat) (card : Fin count) : Nat :=
  Arithmetic.pack (count + 1) (publicId card) card.val

def ParentGuard (publicId : Fin count → Nat) (winner loser : Fin count) : Prop :=
  winner ≠ loser ∧
    (publicId winner < publicId loser ∨
      (publicId winner = publicId loser ∧ winner.val ≤ loser.val))

theorem parent_rank_lt (publicId : Fin count → Nat) (winner loser : Fin count)
    (guard : ParentGuard publicId winner loser) : rank publicId winner < rank publicId loser := by
  apply (Arithmetic.pack_lt_iff (count + 1) _ _ _ _ (by omega) (by omega)).mpr
  rcases guard.2 with hp | ⟨hp, hd⟩
  · exact Or.inl hp
  · refine Or.inr ⟨hp, ?_⟩
    have hn : winner.val ≠ loser.val := fun he => guard.1 (Fin.ext he)
    omega

structure Forest (count : Nat) where
  publicId : Fin count → Nat
  keep : Fin count → Bool
  parent : Fin count → Fin count
  guarded : ∀ card, keep card = false → ParentGuard publicId (parent card) card

/-- Mathematical counterpart of the while-not-keep parent traversal. -/
def root (forest : Forest count) (card : Fin count) : Fin count :=
  if _kept : forest.keep card = true then card else root forest (forest.parent card)
termination_by rank forest.publicId card
decreasing_by
  exact parent_rank_lt _ _ _ (forest.guarded card (Bool.eq_false_iff.mpr _kept))

theorem root_kept (forest : Forest count) (card : Fin count) : forest.keep (root forest card) = true := by
  induction card using (measure (rank forest.publicId)).wf.induction with
  | h card ih =>
      rw [root]
      split
      next kept => exact kept
      next removed =>
        exact ih (forest.parent card)
          (parent_rank_lt _ _ _ (forest.guarded card (Bool.eq_false_iff.mpr removed)))

@[simp] theorem root_of_kept (forest : Forest count) (card : Fin count)
    (kept : forest.keep card = true) : root forest card = card := by
  rw [root, dif_pos kept]

@[simp] theorem root_idempotent (forest : Forest count) (card : Fin count) :
    root forest (root forest card) = root forest card := root_of_kept forest _ (root_kept forest card)

/-- Every removed card is stored under its retained root, not merely its
immediate dominator, which may itself have been removed later. -/
def alternatives (forest : Forest count) (winner : Fin count) : Finset (Fin count) :=
  Finset.univ.filter (fun card => forest.keep card = false ∧ root forest card = winner)

def options (forest : Forest count) (winner : Fin count) : Finset (Fin count) :=
  insert winner (alternatives forest winner)

theorem removed_in_alternatives (forest : Forest count) (card : Fin count)
    (removed : forest.keep card = false) : card ∈ alternatives forest (root forest card) := by
  simp [alternatives, removed]

theorem original_in_options (forest : Forest count) (card : Fin count) :
    card ∈ options forest (root forest card) := by
  cases hk : forest.keep card with
  | false => exact Finset.mem_insert_of_mem (removed_in_alternatives forest card hk)
  | true => simp [options, root_of_kept forest card hk]

theorem options_iff_root (forest : Forest count) (winner card : Fin count)
    (kept : forest.keep winner = true) :
    card ∈ options forest winner ↔ root forest card = winner := by
  simp only [options, Finset.mem_insert, alternatives, Finset.mem_filter, Finset.mem_univ, true_and]
  constructor
  · rintro (he | ⟨_, hr⟩)
    · subst card
      exact root_of_kept forest winner kept
    · exact hr
  · intro hr
    cases hk : forest.keep card with
    | false => exact Or.inr ⟨rfl, hr⟩
    | true => exact Or.inl ((root_of_kept forest card hk).symm.trans hr)

/-- Enumerating one independent inverse fiber per slot covers simultaneous
substitutions in any number of slots. -/
theorem inverse_fibers_cover (forest : Forest count) (deck : List (Fin count)) :
    deck ∈ DP.assignments ((deck.map (root forest)).map (options forest)) := by
  rw [DP.mem_assignments]
  induction deck with
  | nil => exact List.Forall₂.nil
  | cons card rest ih =>
      exact List.Forall₂.cons (original_in_options forest card) ih

/-- Every recovered slot maps back to the root in that same slot. -/
theorem inverse_fibers_exact (forest : Forest count) (roots deck : List (Fin count))
    (kept : ∀ card ∈ roots, forest.keep card = true) :
    deck ∈ DP.assignments (roots.map (options forest)) ↔ deck.map (root forest) = roots := by
  rw [DP.mem_assignments]
  induction roots generalizing deck with
  | nil => simp
  | cons first rest ih =>
      cases deck with
      | nil => simp
      | cons card deck =>
          have hk := kept first List.mem_cons_self
          have hr : ∀ other ∈ rest, forest.keep other = true :=
            fun other ho => kept other (List.mem_cons_of_mem first ho)
          simp only [List.map_cons, List.forall₂_cons, List.cons.injEq]
          rw [options_iff_root forest first card hk, ih deck hr]

end Allium.RootForest
