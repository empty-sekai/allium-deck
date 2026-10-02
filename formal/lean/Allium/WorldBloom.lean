import Allium.DeckComposition
import Allium.FiniteBounds

/-!
# Finale shuffle and explicit master-row precedence

Shuffle counts original units, independently of area-item mixed-unit counting.
Its suffix envelope is taken over reachable extra unit sets, so no monotonicity
of an arbitrary reward table is required.
-/
namespace Allium.WorldBloom
open PowerModel DeckComposition FiniteBounds

/-- A total reward table; counts outside three through five yield zero. -/
def shuffleValue (finale3 : Bool) (units : Finset UnitId) : Nat :=
  if finale3 then
    match units.card with
    | 3 => 10
    | 4 => 30
    | 5 => 50
    | _ => 0
  else 0

theorem shuffle_value_eq (finale3 : Bool) (deck : List CardFacts) :
    shuffleValue finale3 (originalUnits deck) = shuffleBonus finale3 deck := rfl

/-- Every future member contributes at most one new original unit. -/
theorem original_units_card_le (cards : List CardFacts) :
    (originalUnits cards).card ≤ cards.length := by
  simpa only [originalUnits, List.length_map] using
    List.toFinset_card_le (cards.map CardFacts.original)

theorem original_units_append (selected rest : List CardFacts) :
    originalUnits (selected ++ rest) = originalUnits selected ∪ originalUnits rest := by
  simp [originalUnits]

def shuffleUpper (finale3 : Bool) (selected : List CardFacts) (slots : Nat) : Nat :=
  (selections slots (Finset.univ : Finset UnitId)).sup
    (fun extra => shuffleValue finale3 (originalUnits selected ∪ extra))

/-- Superset enumeration is confined to extra original units allowed by the
remaining slots; it is not the global constant fifty. -/
theorem shuffle_upper_sound (finale3 : Bool) (selected rest : List CardFacts) (slots : Nat)
    (space : rest.length ≤ slots) :
    shuffleBonus finale3 (selected ++ rest) ≤ shuffleUpper finale3 selected slots := by
  rw [← shuffle_value_eq, original_units_append]
  exact Finset.le_sup (f := fun extra =>
    shuffleValue finale3 (originalUnits selected ∪ extra))
    ((mem_selections _ _ _).mpr ⟨Finset.subset_univ _,
      (original_units_card_le rest).trans space⟩)

/-- Independent bonus components include shuffle explicitly before scoring. -/
theorem total_bonus_upper (finale3 : Bool) (selected rest : List CardFacts) (slots : Nat)
    (base support attributeExtra baseUpper supportUpper attributeUpper : Nat)
    (space : rest.length ≤ slots) (hb : base ≤ baseUpper) (hs : support ≤ supportUpper)
    (ha : attributeExtra ≤ attributeUpper) :
    base + support + attributeExtra + shuffleBonus finale3 (selected ++ rest) ≤
      baseUpper + supportUpper + attributeUpper + shuffleUpper finale3 selected slots :=
  Nat.add_le_add (Nat.add_le_add (Nat.add_le_add hb hs) ha)
    (shuffle_upper_sound finale3 selected rest slots space)

inductive Edition where
  | wlOne
  | wlTwo
  | wlThree
  deriving DecidableEq

def limitedFallback (edition : Edition) (finale : Bool) : Nat :=
  match edition, finale with
  | .wlTwo, true => 4
  | _, _ => 5

def limitedCap (edition : Edition) (finale : Bool) (master : Option Nat) : Nat :=
  master.getD (limitedFallback edition finale)

@[simp] theorem explicit_limited_wins (edition : Edition) (finale : Bool) (limit : Nat) :
    limitedCap edition finale (some limit) = limit := rfl

theorem finale_caps : limitedCap .wlTwo true none = 4 ∧ limitedCap .wlThree true none = 5 := by
  decide

/-- Skill master rows contain the raw cap, with no base-percent subtraction. -/
def skillCap (master fallback : Option Nat) : Option Nat :=
  match master with
  | some limit => some limit
  | none => fallback

@[simp] theorem explicit_skill_wins (limit : Nat) (fallback : Option Nat) :
    skillCap (some limit) fallback = some limit := rfl

/-- The total-power ceiling is independent of furniture/shuffle activation. -/
def thirdPowerCap (edition : Edition) (worldBloom : Bool) : Option Nat :=
  if edition = .wlThree ∧ worldBloom then some 336000 else none

theorem third_power_cap : thirdPowerCap .wlThree true = some 336000 := by decide

/-- The production context uses a policy-dependent global shuffle ceiling. -/
def globalShuffleUpper (finale3 : Bool) : Nat := if finale3 then 50 else 0

theorem global_shuffle_upper (finale3 : Bool) (deck : List CardFacts) :
    shuffleBonus finale3 deck ≤ globalShuffleUpper finale3 := by
  cases finale3
  · simp [globalShuffleUpper]
  · exact shuffle_le_fifty true deck

def shuffleClasses (finale3 : Bool) : Finset Nat :=
  if finale3 then {0, 10, 30, 50} else {0}

theorem actual_shuffle_class (finale3 : Bool) (deck : List CardFacts) :
    shuffleBonus finale3 deck ∈ shuffleClasses finale3 := by
  unfold shuffleBonus shuffleClasses
  split
  · split <;> simp
  · simp

/-- Every tier's attribute-diversity value is crossed with all shuffle classes.
Merging equal sums changes only the stored attribute-count mask. -/
def tierExtraValues (finale3 : Bool) (counts : Finset Nat) (diversity : Nat → Nat) (scale : Nat) : Finset Nat :=
  (counts ×ˢ shuffleClasses finale3).image (fun pair => (diversity pair.1 + pair.2) * 10 * scale)

theorem tier_extra_value_present (finale3 : Bool) (deck : List CardFacts)
    (counts : Finset Nat) (diversity : Nat → Nat) (scale count : Nat) (member : count ∈ counts) :
    (diversity count + shuffleBonus finale3 deck) * 10 * scale ∈
      tierExtraValues finale3 counts diversity scale := by
  exact Finset.mem_image.mpr ⟨(count, shuffleBonus finale3 deck),
    Finset.mem_product.mpr ⟨member, actual_shuffle_class finale3 deck⟩, rfl⟩

/-- Context construction tests both finale identity and the third edition. -/
def thirdFinale (context : Option (Bool × Option Int)) : Bool :=
  match context with
  | none => false
  | some (finale, turn) => finale && decide (turn = some 3)

theorem third_finale_iff (context : Option (Bool × Option Int)) :
    thirdFinale context = true ↔ context = some (true, some 3) := by
  cases context with
  | none => simp [thirdFinale]
  | some state => rcases state with ⟨finale, turn⟩; cases finale <;> simp [thirdFinale]

/-- An explicit signed master limit wins even when it is not a fallback value. -/
def fixtureLimit (finale legacy : Bool) (master : Option Int) : Option Int :=
  match master with
  | some limit => some limit
  | none => if finale then some (if legacy then 20 else 60) else none

theorem fixture_explicit_wins (finale legacy : Bool) (limit : Int) :
    fixtureLimit finale legacy (some limit) = some limit := rfl

theorem fixture_fallbacks : fixtureLimit true true none = some 20 ∧
    fixtureLimit true false none = some 60 ∧ fixtureLimit false false none = none := by decide

end Allium.WorldBloom
