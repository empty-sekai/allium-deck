import Allium.PowerModel
import Allium.Collection

/-!
# All 49 area-item composition regimes

The option pair represents Mixed, SharedAttr, SharedUnit, SharedUnitAttr.
`Matches` is the semantic deck class; `Admits` is the weaker pool filter.
The latter alone does NOT justify a regime bound. Tables remain arbitrary.

Sources: composition.rs::{Regime,RegimePlan::new,power_over_keys,search_regimes}.
-/
namespace Allium.Composition
open PowerModel

abbrev Regime := Option UnitId × Option Attribute

theorem regime_count : Fintype.card Regime = 49 := by decide

def Admits (data : CardData) (r : Regime) : Prop :=
  (match r.1 with | none => True | some unit => unit ∈ data.units) ∧
  (match r.2 with | none => True | some attr => data.attr = attr)

variable {Card : Type*}

def Matches (data : Card → CardData) (deck : List Card) (r : Regime) : Prop :=
  (match r.1 with
    | none => commonUnits data deck = ∅
    | some unit => unit ∈ commonUnits data deck) ∧
  (match r.2 with
    | none => ¬ ∃ attr, UniformAt data deck attr
    | some attr => UniformAt data deck attr)

/-- Every deck has a regime, also when several units are shared at once. -/
theorem regimes_cover (data : Card → CardData) (deck : List Card) :
    ∃ r : Regime, Matches data deck r := by
  classical
  by_cases hu : commonUnits data deck = ∅
  · by_cases ha : ∃ attr, UniformAt data deck attr
    · rcases ha with ⟨attr, ha⟩
      exact ⟨(none, some attr), hu, ha⟩
    · exact ⟨(none, none), hu, ha⟩
  · obtain ⟨unit, hu⟩ := Finset.nonempty_iff_ne_empty.mpr hu
    by_cases ha : ∃ attr, UniformAt data deck attr
    · rcases ha with ⟨attr, ha⟩
      exact ⟨(some unit, some attr), hu, ha⟩
    · exact ⟨(some unit, none), hu, ha⟩

/-- Every real member of a regime survives that regime's pool filter. -/
theorem matches_admits (data : Card → CardData) (deck : List Card) (r : Regime)
    (h : Matches data deck r) (card : Card) (hc : card ∈ deck) :
    Admits (data card) r := by
  rcases r with ⟨unit, attr⟩
  rcases h with ⟨hu, ha⟩
  constructor
  · cases unit with
    | none => trivial
    | some unit => exact (mem_commonUnits data deck unit).mp hu card hc
  · cases attr with
    | none => trivial
    | some attr => exact ha card hc

def unitFlags (r : Regime) : Finset Bool :=
  if r.1.isSome then {false, true} else {false}

def attrFlag (r : Regime) : Bool := r.2.isSome

/-- Mixed/SharedAttr use only false; shared-unit regimes must include BOTH
false and true because a card may carry units outside the common intersection. -/
theorem actual_unit_flag_mem (data : Card → CardData) (deck : List Card)
    (r : Regime) (h : Matches data deck r) (unit : UnitId) :
    decide (unit ∈ commonUnits data deck) ∈ unitFlags r := by
  rcases r with ⟨u, a⟩
  cases u with
  | none =>
      have he : commonUnits data deck = ∅ := h.1
      change decide (unit ∈ commonUnits data deck) ∈ ({false} : Finset Bool)
      rw [he]
      simp
  | some u =>
      change decide (unit ∈ commonUnits data deck) ∈ ({false, true} : Finset Bool)
      cases decide (unit ∈ commonUnits data deck) <;> decide

theorem actual_attribute_flag (data : Card → CardData) (deck : List Card)
    (r : Regime) (h : Matches data deck r) :
    sharesAttribute data deck = attrFlag r := by
  classical
  rcases r with ⟨u, a⟩
  cases a with
  | none =>
      have ha : ¬ ∃ attr, UniformAt data deck attr := h.2
      simp [sharesAttribute, attrFlag, ha]
  | some a =>
      have ha : ∃ attr, UniformAt data deck attr := ⟨a, h.2⟩
      simp [sharesAttribute, attrFlag, ha]

/-- Maximum over carried units and exactly the selected member keys. -/
def powerBound (card : CardData) (r : Regime) : ℕ :=
  card.units.sup (fun unit => (unitFlags r).sup (fun shared =>
    card.values (tableIndex (card.profile unit) shared (attrFlag r))))

/-- A concrete bound, derived from the evaluator's key selection. No premise
of the form `actual power <= bound` or table monotonicity is assumed. -/
theorem power_bound_sound (data : Card → CardData) (deck : List Card)
    (r : Regime) (h : Matches data deck r) (card : Card) :
    cardPower data deck card ≤ powerBound (data card) r := by
  unfold cardPower resolved
  apply Finset.sup_le
  intro unit hu
  have inner : unitValue (data card) (commonUnits data deck) (sharesAttribute data deck) unit ≤
      (unitFlags r).sup (fun shared =>
        (data card).values (tableIndex ((data card).profile unit) shared (attrFlag r))) := by
    unfold unitValue
    rw [actual_attribute_flag data deck r h]
    exact Finset.le_sup (f := fun shared =>
      (data card).values (tableIndex ((data card).profile unit) shared (attrFlag r)))
      (actual_unit_flag_mem data deck r h unit)
  exact inner.trans (Finset.le_sup (f := fun unit => (unitFlags r).sup (fun shared =>
    (data card).values (tableIndex ((data card).profile unit) shared (attrFlag r)))) hu)

/-- Independent sum relaxation; legality constraints are not needed here. -/
theorem deck_sum_bound (data : Card → CardData) (deck : List Card)
    (r : Regime) (h : Matches data deck r) :
    total data deck ≤ (deck.map (fun card => powerBound (data card) r)).sum :=
  List.sum_le_sum (fun card _ => power_bound_sound data deck r h card)

/-- A regime partition may overlap. Keeping all variants and canonicalizing
only at the end avoids assuming disjoint public card sets across regimes. -/
noncomputable def deckClass (data : Card → CardData) (pool : Finset (List Card))
    (r : Regime) : Finset (List Card) := by
  classical
  exact pool.filter (fun deck => Matches data deck r)

theorem deck_classes_cover [DecidableEq Card] (data : Card → CardData) (pool : Finset (List Card)) :
    Finset.univ.biUnion (deckClass data pool) = pool := by
  classical
  ext deck
  simp only [Finset.mem_biUnion, Finset.mem_univ, true_and, deckClass, Finset.mem_filter]
  constructor
  · rintro ⟨_, h, _⟩
    exact h
  · intro h
    obtain ⟨r, hr⟩ := regimes_cover data deck
    exact ⟨r, h, hr⟩

end Allium.Composition
