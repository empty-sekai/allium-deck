import Allium.Composition
import Allium.MixedPower

/-!
# Shared-unit Power scenario envelopes

The production solver fixes the complete shared-unit set and takes the maximum
of the two physical mixed-unit states under the pool's activation policy.
The singleton envelope below is an auxiliary legacy-table inequality, not the
production scenario constructor. No monotonicity of stored values is required.
-/
namespace Allium.ScenarioPower
open PowerModel

def bound (card : CardData) (common : Finset UnitId) (attr : Bool) : ℕ :=
  if common = ∅ then resolved card ∅ attr
  else common.sup (fun unit => resolved card {unit} attr)

theorem bound_sound (card : CardData) (common : Finset UnitId) (attr : Bool) :
    resolved card common attr ≤ bound card common attr := by
  classical
  by_cases hempty : common = ∅
  · simp [bound, hempty]
  · rw [bound, if_neg hempty]
    apply Finset.sup_le
    intro unit hunit
    by_cases hshared : unit ∈ common
    · have hsame : unitValue card common attr unit = unitValue card {unit} attr unit := by
        simp [unitValue, hshared]
      calc
        unitValue card common attr unit = unitValue card {unit} attr unit := hsame
        _ ≤ resolved card {unit} attr := Finset.le_sup (f := unitValue card {unit} attr) hunit
        _ ≤ common.sup (fun shared => resolved card {shared} attr) := Finset.le_sup (f := fun shared => resolved card {shared} attr) hshared
    · obtain ⟨shared, hchosen⟩ := Finset.nonempty_iff_ne_empty.mpr hempty
      have hne : unit ≠ shared := by
        intro heq
        exact hshared (by simpa only [heq] using hchosen)
      have hsame : unitValue card common attr unit = unitValue card {shared} attr unit := by
        simp [unitValue, hshared, hne]
      calc
        unitValue card common attr unit = unitValue card {shared} attr unit := hsame
        _ ≤ resolved card {shared} attr := Finset.le_sup (f := unitValue card {shared} attr) hunit
        _ ≤ common.sup (fun shared => resolved card {shared} attr) := Finset.le_sup (f := fun shared => resolved card {shared} attr) hchosen

@[simp] theorem empty_exact (card : CardData) (attr : Bool) :
    bound card ∅ attr = resolved card ∅ attr := by simp [bound]

@[simp] theorem singleton_exact (card : CardData) (unit : UnitId) (attr : Bool) :
    bound card {unit} attr = resolved card {unit} attr := by simp [bound]

theorem bound_le_tableMax (card : CardData) (common : Finset UnitId) (attr : Bool) :
    bound card common attr ≤ tableMax card := by
  unfold bound
  split_ifs
  · exact resolved_upper card ∅ attr
  · exact Finset.sup_le (fun _ _ => resolved_upper card _ attr)

/-- The exact required common-unit set always passes the scenario's mask
containment filter at every actual member of a deck. -/
theorem actual_common_admitted {Card : Type*} (data : Card → CardData)
    (deck : List Card) (card : Card) (hc : card ∈ deck) :
    commonUnits data deck ⊆ (data card).units := by
  intro unit hu
  exact (mem_commonUnits data deck unit).mp hu card hc

/-- Regression: several common units can make the singleton envelope loose. -/
def nonmonotoneCard : CardData :=
  { attr := 0
    units := {0, 1}
    profile := fun unit => decide (unit = 1)
    values := fun i => if i = 4 then 100 else if i = 6 then 20 else 10 }

theorem multi_unit_is_only_an_upper_bound :
    resolved nonmonotoneCard {0, 1} false = 20 ∧
      bound nonmonotoneCard {0, 1} false = 100 := by decide

/-- Regression: admission does not make an arbitrary regime bound valid. -/
def sharedBonusCard : CardData :=
  { attr := 0
    units := {0}
    profile := fun _ => false
    values := fun i => if i = 3 then 100 else 1 }

theorem admission_is_not_bound_soundness :
    Composition.Admits sharedBonusCard (none, none) ∧
      Composition.powerBound sharedBonusCard (none, none) = 1 ∧
      resolved sharedBonusCard {0} true = 100 := by
  constructor
  · exact ⟨trivial, trivial⟩
  · decide

/-- Regression: the lower-bound theorem must retain the nonempty-unit guard. -/
def emptyUnitCard : CardData :=
  { attr := 0, units := ∅, profile := fun _ => false, values := fun _ => 1 }

theorem empty_units_require_a_lower_bound_guard :
    tableMin emptyUnitCard = 1 ∧ resolved emptyUnitCard ∅ false = 0 := by decide

/-- Both physical states are evaluated against the same shared-unit set. The
mode is applied inside each evaluation, so ForceOn/ForceOff do not add states
excluded by the caller's policy. -/
def sourceBound (mode : MixedPower.Mode) (card : MixedPower.Card)
    (common : Finset UnitId) (attr : Bool) : Nat :=
  max (MixedPower.effective mode false common attr card)
    (MixedPower.effective mode true common attr card)

theorem source_bound_sound (mode : MixedPower.Mode) (card : MixedPower.Card)
    (common : Finset UnitId) (attr mixed : Bool) :
    MixedPower.effective mode mixed common attr card ≤ sourceBound mode card common attr := by
  cases mixed
  · exact le_max_left _ _
  · exact le_max_right _ _

theorem source_full_deck_bound {Id : Type*} (mode : MixedPower.Mode)
    (data : Id → MixedPower.Card) (deck : List Id) (full : deck.length = 5) (id : Id) :
    MixedPower.cardPower mode data deck id ≤
      sourceBound mode (data id) (commonUnits (fun card => MixedPower.legacy (data card)) deck)
        (sharesAttribute (fun card => MixedPower.legacy (data card)) deck) := by
  simpa only [MixedPower.cardPower, full, ↓reduceIte] using
    source_bound_sound mode (data id) _ _ (DeckComposition.isMultiUnit (MixedPower.deckFacts data deck))

theorem source_bound_le_maximum (mode : MixedPower.Mode) (card : MixedPower.Card)
    (common : Finset UnitId) (attr : Bool) :
    sourceBound mode card common attr ≤ MixedPower.maximum card :=
  max_le (MixedPower.effective_upper mode false common attr card)
    (MixedPower.effective_upper mode true common attr card)

end Allium.ScenarioPower
