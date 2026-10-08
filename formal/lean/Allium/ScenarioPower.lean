import Allium.Composition

/-!
# The multi-unit Power scenario envelope

This is solver/power.rs::scenario_power, not composition::power_over_keys.
For nonempty common units it maximizes the evaluator over singleton common-unit
hypotheses. The result can be strictly larger than the real power with several
shared units; this is why visited leaves must be evaluated in their true context.
No monotonicity of the eight decoded values is used.
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

end Allium.ScenarioPower
