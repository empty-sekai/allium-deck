import Allium.FiniteBounds

/-!
# Concrete eight-entry power semantics

The two unit profiles each have four member keys. The table is arbitrary:
shared-unit/attribute bonuses are NOT assumed to increase a table entry.
The six-unit scan, count-equals-five guards, and optional total-power cap are
modeled explicitly. The packed u18 representation is not identified with Lean
machine integers; this is the mathematical model of the decoded entries.

Sources: evaluate.rs::{member_key,resolve_card_power,resolve_power_target},
context.rs::clamp_power_total, solver/numeric.rs::bound_can_prune.
-/
namespace Allium.PowerModel

abbrev UnitId := Fin 6
abbrev Attribute := Fin 6

/-- Exactly the index `profile * 4 + sharedUnit * 2 + sharedAttr`. -/
def tableIndex (profile sharedUnit sharedAttr : Bool) : Fin 8 :=
  ⟨profile.toNat * 4 + sharedUnit.toNat * 2 + sharedAttr.toNat, by
    cases profile <;> cases sharedUnit <;> cases sharedAttr <;> decide⟩

theorem tableIndex_surjective :
    ∀ i : Fin 8, ∃ p u a : Bool, tableIndex p u a = i := by decide

structure CardData where
  attr : Attribute
  units : Finset UnitId
  profile : UnitId → Bool
  values : Fin 8 → ℕ

variable {Card : Type*}

def commonUnits (data : Card → CardData) : List Card → Finset UnitId
  | [] => Finset.univ
  | card :: rest => (data card).units ∩ commonUnits data rest

@[simp] theorem mem_commonUnits (data : Card → CardData) (deck : List Card)
    (unit : UnitId) :
    unit ∈ commonUnits data deck ↔ ∀ card ∈ deck, unit ∈ (data card).units := by
  induction deck with
  | nil => simp [commonUnits]
  | cons card rest ih => simp [commonUnits, ih]

def UniformAt (data : Card → CardData) (deck : List Card) (a : Attribute) : Prop :=
  ∀ card ∈ deck, (data card).attr = a

noncomputable def sharesAttribute (data : Card → CardData) (deck : List Card) : Bool := by
  classical
  exact decide (∃ a, UniformAt data deck a)

@[simp] theorem sharesAttribute_eq_true (data : Card → CardData) (deck : List Card) :
    sharesAttribute data deck = true ↔ ∃ a, UniformAt data deck a := by
  classical
  simp [sharesAttribute]

/-- The evaluator's unit count test is exactly membership in the intersection. -/
theorem count_five_iff_common (data : Card → CardData) (deck : List Card)
    (hfive : deck.length = 5) (unit : UnitId) :
    deck.countP (fun card => decide (unit ∈ (data card).units)) = 5 ↔
      unit ∈ commonUnits data deck := by
  rw [← hfive, List.countP_eq_length, mem_commonUnits]
  simp

/-- At every actual member, count-equals-five has one deck-wide attribute flag. -/
theorem attribute_count_five (data : Card → CardData) (deck : List Card)
    (hfive : deck.length = 5) (card : Card) (hcard : card ∈ deck) :
    deck.countP (fun other => decide ((data other).attr = (data card).attr)) = 5 ↔
      sharesAttribute data deck = true := by
  classical
  rw [← hfive, List.countP_eq_length, sharesAttribute_eq_true]
  simp only [decide_eq_true_eq]
  constructor
  · exact fun h => ⟨(data card).attr, h⟩
  · rintro ⟨a, ha⟩ other ho
    exact (ha other ho).trans (ha card hcard).symm

/-- The profile is selected per carried unit, not chosen independently. -/
def unitValue (card : CardData) (common : Finset UnitId) (attr : Bool)
    (unit : UnitId) : ℕ :=
  card.values (tableIndex (card.profile unit) (decide (unit ∈ common)) attr)

def resolved (card : CardData) (common : Finset UnitId) (attr : Bool) : ℕ :=
  card.units.sup (unitValue card common attr)

def tableMax (card : CardData) : ℕ := Finset.univ.sup card.values

def tableMin (card : CardData) : ℕ :=
  Finset.univ.inf' Finset.univ_nonempty card.values

theorem tableMin_le (card : CardData) (i : Fin 8) : tableMin card ≤ card.values i :=
  Finset.inf'_le card.values (Finset.mem_univ i)

theorem le_tableMax (card : CardData) (i : Fin 8) : card.values i ≤ tableMax card :=
  Finset.le_sup (Finset.mem_univ i)

theorem resolved_upper (card : CardData) (common : Finset UnitId) (attr : Bool) :
    resolved card common attr ≤ tableMax card := by
  apply Finset.sup_le
  intro unit _
  exact le_tableMax card _

/-- Nonempty carried units are essential for the lower bound: an empty scan
returns zero, even when all eight decoded entries are positive. -/
theorem resolved_lower (card : CardData) (hunits : card.units.Nonempty)
    (common : Finset UnitId) (attr : Bool) :
    tableMin card ≤ resolved card common attr := by
  rcases hunits with ⟨unit, hu⟩
  calc
    tableMin card ≤ unitValue card common attr unit := tableMin_le card _
    _ ≤ resolved card common attr := Finset.le_sup (f := unitValue card common attr) hu

noncomputable def cardPower (data : Card → CardData) (deck : List Card) (card : Card) : ℕ :=
  resolved (data card) (commonUnits data deck) (sharesAttribute data deck)

noncomputable def total (data : Card → CardData) (deck : List Card) : ℕ :=
  (deck.map (cardPower data deck)).sum

/-- The cap is applied AFTER adding the honor bonus. -/
def clamp (cap : Option ℕ) (value : ℕ) : ℕ :=
  match cap with
  | none => value
  | some limit => min value limit

theorem clamp_monotone (cap : Option ℕ) : Monotone (clamp cap) := by
  intro a b h
  cases cap with
  | none => exact h
  | some limit => exact min_le_min_right limit h

noncomputable def objective (data : Card → CardData) (honor : ℕ) (cap : Option ℕ)
    (deck : List Card) : ℕ := clamp cap (total data deck + honor)

/-- No uniqueness assumption: fixed slots and repeated-character requests
remain valid for this independent numeric relaxation. -/
theorem prefix_free_upper (data : Card → CardData) (selected free : List Card)
    (maximum : ℕ) (hmax : ∀ card ∈ free, tableMax (data card) ≤ maximum) :
    total data (selected ++ free) ≤
      (selected.map (fun card => tableMax (data card))).sum + free.length * maximum := by
  let value := cardPower data (selected ++ free)
  have hs : (selected.map value).sum ≤
      (selected.map (fun card => tableMax (data card))).sum :=
    List.sum_le_sum (fun card _ => resolved_upper (data card) _ _)
  have hf : (free.map value).sum ≤ free.length * maximum := by
    calc
      (free.map value).sum ≤ (free.map (fun _ => maximum)).sum :=
        List.sum_le_sum (fun card hc => (resolved_upper (data card) _ _).trans (hmax card hc))
      _ = free.length * maximum := by simp
  simpa only [total, List.map_append, List.sum_append] using Nat.add_le_add hs hf

theorem prefix_free_lower (data : Card → CardData) (selected free : List Card)
    (minimum : ℕ)
    (hunits : ∀ card ∈ selected ++ free, (data card).units.Nonempty)
    (hmin : ∀ card ∈ free, minimum ≤ tableMin (data card)) :
    (selected.map (fun card => tableMin (data card))).sum + free.length * minimum ≤
      total data (selected ++ free) := by
  let value := cardPower data (selected ++ free)
  have hs : (selected.map (fun card => tableMin (data card))).sum ≤ (selected.map value).sum :=
    List.sum_le_sum (fun card hc => resolved_lower (data card) (hunits card (List.mem_append_left _ hc)) _ _)
  have hf : free.length * minimum ≤ (free.map value).sum := by
    calc
      free.length * minimum = (free.map (fun _ => minimum)).sum := by simp
      _ ≤ (free.map value).sum := List.sum_le_sum (fun card hc =>
        (hmin card hc).trans (resolved_lower (data card) (hunits card (List.mem_append_right _ hc)) _ _))
  simpa only [total, List.map_append, List.sum_append] using Nat.add_le_add hs hf

/-- Actual maximizing numeric prune, including the optional cap and honor. -/
theorem maximizing_prune (data : Card → CardData) (selected free : List Card)
    (maximum honor threshold : ℕ) (cap : Option ℕ)
    (hmax : ∀ card ∈ free, tableMax (data card) ≤ maximum)
    (hcut : clamp cap ((selected.map (fun card => tableMax (data card))).sum +
      free.length * maximum + honor) < threshold) :
    objective data honor cap (selected ++ free) < threshold := by
  apply lt_of_le_of_lt _ hcut
  exact clamp_monotone cap (Nat.add_le_add_right (prefix_free_upper data selected free maximum hmax) honor)

/-- The dual branch must compare a LOWER bound using strict `>` instead. -/
theorem minimizing_prune (data : Card → CardData) (selected free : List Card)
    (minimum honor threshold : ℕ) (cap : Option ℕ)
    (hunits : ∀ card ∈ selected ++ free, (data card).units.Nonempty)
    (hmin : ∀ card ∈ free, minimum ≤ tableMin (data card))
    (hcut : threshold < clamp cap ((selected.map (fun card => tableMin (data card))).sum +
      free.length * minimum + honor)) :
    threshold < objective data honor cap (selected ++ free) := by
  apply lt_of_lt_of_le hcut
  exact clamp_monotone cap (Nat.add_le_add_right (prefix_free_lower data selected free minimum hunits hmin) honor)

end Allium.PowerModel
