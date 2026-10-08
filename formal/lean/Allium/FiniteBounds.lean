import Mathlib.Data.Finset.Powerset
import Mathlib.Data.Finset.Lattice.Fold
import Mathlib.Algebra.Order.BigOperators.Group.Finset
import Mathlib.Tactic

/-!
# Finite relaxations

The semantic value of a top-r sum is the maximum over subsets of size at most
r. Non-negative values permit zero padding. This module proves the actual
per-character relaxation, including character injectivity, rather than taking
`score <= bound` as an input assumption.

Source: suffix.rs, solver/{numeric,power,challenge,final_chapter}.rs.
-/
namespace Allium.FiniteBounds

variable {Card Character : Type*} [DecidableEq Character]

def selections (r : ℕ) (pool : Finset Card) : Finset (Finset Card) :=
  pool.powerset.filter (fun picks => picks.card ≤ r)

@[simp] theorem mem_selections (r : ℕ) (pool picks : Finset Card) :
    picks ∈ selections r pool ↔ picks ⊆ pool ∧ picks.card ≤ r := by
  simp [selections]

/-- A finite maximum, with no heuristically chosen prefix. -/
def maxSum (r : ℕ) (pool : Finset Card) (weight : Card → ℕ) : ℕ :=
  (selections r pool).sup (fun picks => ∑ card ∈ picks, weight card)

theorem sum_le_maxSum (r : ℕ) (pool picks : Finset Card) (weight : Card → ℕ)
    (hsub : picks ⊆ pool) (hcard : picks.card ≤ r) :
    (∑ card ∈ picks, weight card) ≤ maxSum r pool weight := by
  exact Finset.le_sup (f := fun s : Finset Card => ∑ card ∈ s, weight card)
    ((mem_selections r pool picks).mpr ⟨hsub, hcard⟩)

theorem maxSum_mono_pool (r : ℕ) (weight : Card → ℕ)
    {small large : Finset Card} (h : small ⊆ large) :
    maxSum r small weight ≤ maxSum r large weight := by
  apply Finset.sup_le
  intro picks hp
  rcases (mem_selections _ _ _).mp hp with ⟨hsub, hcard⟩
  exact sum_le_maxSum r large picks weight (hsub.trans h) hcard

theorem maxSum_mono_count (pool : Finset Card) (weight : Card → ℕ)
    {r s : ℕ} (h : r ≤ s) : maxSum r pool weight ≤ maxSum s pool weight := by
  apply Finset.sup_le
  intro picks hp
  rcases (mem_selections _ _ _).mp hp with ⟨hsub, hcard⟩
  exact sum_le_maxSum s pool picks weight hsub (hcard.trans h)

theorem maxSum_mono_weight (r : ℕ) (pool : Finset Card) (a b : Card → ℕ)
    (h : ∀ card ∈ pool, a card ≤ b card) : maxSum r pool a ≤ maxSum r pool b := by
  apply Finset.sup_le
  intro picks hp
  rcases (mem_selections _ _ _).mp hp with ⟨hsub, hcard⟩
  calc
    (∑ card ∈ picks, a card) ≤ ∑ card ∈ picks, b card :=
      Finset.sum_le_sum (fun card hc => h card (hsub hc))
    _ ≤ maxSum r pool b := sum_le_maxSum r pool picks b hsub hcard

theorem maxSum_global (r : ℕ) (pool : Finset Card) (weight : Card → ℕ) :
    maxSum r pool weight ≤ r * pool.sup weight := by
  apply Finset.sup_le
  intro picks hp
  rcases (mem_selections _ _ _).mp hp with ⟨hsub, hcard⟩
  calc
    (∑ card ∈ picks, weight card) ≤ ∑ _card ∈ picks, pool.sup weight :=
      Finset.sum_le_sum (fun _ hc => Finset.le_sup (hsub hc))
    _ = picks.card * pool.sup weight := by simp
    _ ≤ r * pool.sup weight := Nat.mul_le_mul_right _ hcard

/-- The maximum of each component may come from a different card. -/
theorem independent_components {n : ℕ} (r : ℕ) (pool picks : Finset Card)
    (feature : Card → Fin n → ℕ) (objective : (Fin n → ℕ) → ℕ)
    (hmono : Monotone objective) (hsub : picks ⊆ pool) (hcard : picks.card ≤ r) :
    objective (fun i => ∑ card ∈ picks, feature card i) ≤
      objective (fun i => maxSum r pool (fun card => feature card i)) := by
  apply hmono
  intro i
  exact sum_le_maxSum r pool picks (fun card => feature card i) hsub hcard

/-- Exact maximum among the pool's variants of one character. -/
def characterMax (pool : Finset Card) (character : Card → Character)
    (weight : Card → ℕ) (c : Character) : ℕ :=
  (pool.filter (fun card => character card = c)).sup weight

def characterBound (r : ℕ) (pool : Finset Card) (character : Card → Character)
    (weight : Card → ℕ) : ℕ :=
  maxSum r (pool.image character) (characterMax pool character weight)

theorem le_characterMax (pool : Finset Card) (character : Card → Character)
    (weight : Card → ℕ) (card : Card) (h : card ∈ pool) :
    weight card ≤ characterMax pool character weight (character card) := by
  exact Finset.le_sup (by simp [h])

/-- A legal unique-character completion contributes at most the top-r sum of
per-character maxima. Reuse of a character is NOT silently permitted here. -/
theorem character_bound_sound (r : ℕ) (pool picks : Finset Card)
    (character : Card → Character) (weight : Card → ℕ)
    (hsub : picks ⊆ pool) (hcard : picks.card ≤ r)
    (hunique : Set.InjOn character picks) :
    (∑ card ∈ picks, weight card) ≤ characterBound r pool character weight := by
  unfold characterBound
  calc
    (∑ card ∈ picks, weight card) ≤
        ∑ card ∈ picks, characterMax pool character weight (character card) :=
      Finset.sum_le_sum (fun card hc => le_characterMax pool character weight card (hsub hc))
    _ = ∑ c ∈ picks.image character, characterMax pool character weight c := by
      rw [Finset.sum_image]
      exact hunique
    _ ≤ maxSum r (pool.image character) (characterMax pool character weight) :=
      sum_le_maxSum r _ _ _ (Finset.image_subset_image hsub)
        ((Finset.card_image_le).trans hcard)

/-- Too few unused characters is a genuine feasibility contradiction. -/
theorem insufficient_characters (pool picks : Finset Card)
    (character : Card → Character) (hsub : picks ⊆ pool)
    (hunique : Set.InjOn character picks) : picks.card ≤ (pool.image character).card := by
  calc
    picks.card = (picks.image character).card :=
      (Finset.card_image_of_injOn hunique).symm
    _ ≤ (pool.image character).card :=
      Finset.card_le_card (Finset.image_subset_image hsub)

/-- Removing possible cards cannot increase a character-aware suffix bound. -/
theorem character_bound_mono (r : ℕ) (character : Card → Character)
    (weight : Card → ℕ) {small large : Finset Card} (h : small ⊆ large) :
    characterBound r small character weight ≤ characterBound r large character weight := by
  unfold characterBound
  apply le_trans (maxSum_mono_weight r _ _ _ ?_)
    (maxSum_mono_pool r _ (Finset.image_subset_image h))
  intro c _
  apply Finset.sup_le
  intro card hc
  exact Finset.le_sup (by
    rcases Finset.mem_filter.mp hc with ⟨hm, heq⟩
    exact Finset.mem_filter.mpr ⟨h hm, heq⟩)

/-- A sorted candidate and its entire tail can be bounded by reusing the
current candidate's maximum; this deliberately relaxes uniqueness. -/
theorem repeated_max_bound (picks : Finset Card) (weight : Card → ℕ)
    (maximum r : ℕ) (hcard : picks.card ≤ r)
    (hmax : ∀ card ∈ picks, weight card ≤ maximum) :
    (∑ card ∈ picks, weight card) ≤ r * maximum := by
  calc
    (∑ card ∈ picks, weight card) ≤ ∑ _card ∈ picks, maximum := Finset.sum_le_sum hmax
    _ = picks.card * maximum := by simp
    _ ≤ r * maximum := Nat.mul_le_mul_right _ hcard

/-- The dual lower bound used by minimizing Power; exactly r picks remain. -/
theorem repeated_min_bound (picks : Finset Card) (weight : Card → ℕ)
    (minimum r : ℕ) (hcard : picks.card = r)
    (hmin : ∀ card ∈ picks, minimum ≤ weight card) :
    r * minimum ≤ ∑ card ∈ picks, weight card := by
  calc
    r * minimum = ∑ _card ∈ picks, minimum := by simp [hcard]
    _ ≤ ∑ card ∈ picks, weight card := Finset.sum_le_sum hmin

/-- A bound over a nested tail justifies break, not merely continue. -/
theorem monotone_break (bound : ℕ → ℕ) (hmono : Antitone bound)
    (score i j threshold : ℕ) (hij : i ≤ j)
    (hsound : score ≤ bound j) (hprune : bound i < threshold) : score < threshold :=
  lt_of_le_of_lt (hsound.trans (hmono hij)) hprune

/-- Dominance of a BOUND state is safe for computing a ceiling. This theorem
makes no claim that an actual deck represented by that state can be deleted. -/
theorem frontier_bound {State : Type*} [DecidableEq State] [Preorder State]
    (old frontier : Finset State) (value : State → ℕ) (hmono : Monotone value)
    (hcover : ∀ state ∈ old, ∃ better ∈ frontier, state ≤ better) :
    old.sup value ≤ frontier.sup value := by
  apply Finset.sup_le
  intro state hs
  rcases hcover state hs with ⟨better, hb, hle⟩
  exact (hmono hle).trans (Finset.le_sup hb)

end Allium.FiniteBounds
