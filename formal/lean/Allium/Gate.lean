import Allium.DeckComposition

/-!
# User-level gate selection

The preferred unit is the support unit, otherwise the original unit. A bare
virtual singer chooses the highest user level with stable input-order ties.
The selected row is resolved against the level table only after selection.
-/
namespace Allium.Gate
open PowerModel DeckComposition

structure UserGate where
  unit : UnitId
  level : Nat
  deriving DecidableEq

/-- First row at the maximum user level. -/
def highest : List UserGate → Option UserGate
  | [] => none
  | row :: rest =>
      match highest rest with
      | none => some row
      | some best => if row.level < best.level then some best else some row

theorem highest_mem (rows : List UserGate) (best : UserGate)
    (h : highest rows = some best) : best ∈ rows := by
  induction rows with
  | nil => simp [highest] at h
  | cons row rest ih =>
      simp only [highest] at h
      cases hr : highest rest with
      | none =>
          simp only [hr] at h
          have he : row = best := Option.some.inj h
          subst row
          exact List.mem_cons_self
      | some chosen =>
          simp only [hr] at h
          split at h
          · have he : chosen = best := Option.some.inj h
            subst chosen
            exact List.mem_cons_of_mem row (ih hr)
          · have he : row = best := Option.some.inj h
            subst row
            exact List.mem_cons_self

theorem highest_none (rows : List UserGate) : highest rows = none ↔ rows = [] := by
  cases rows with
  | nil => simp [highest]
  | cons row rest =>
      cases hr : highest rest with
      | none => simp [highest, hr]
      | some best =>
          by_cases hl : row.level < best.level <;> simp [highest, hr, hl]

theorem highest_ge (rows : List UserGate) (best : UserGate)
    (h : highest rows = some best) : ∀ row ∈ rows, row.level ≤ best.level := by
  induction rows generalizing best with
  | nil => simp
  | cons first rest ih =>
      cases hr : highest rest with
      | none =>
          have empty := (highest_none rest).mp hr
          subst rest
          simp only [highest] at h
          have he : first = best := Option.some.inj h
          subst first
          simp
      | some chosen =>
          simp only [highest, hr] at h
          by_cases hc : first.level < chosen.level
          · simp only [if_pos hc] at h
            have he : chosen = best := Option.some.inj h
            subst chosen
            intro row hrow
            rcases List.mem_cons.mp hrow with he | hm
            · subst row
              exact Nat.le_of_lt hc
            · exact ih _ hr row hm
          · simp only [if_neg hc] at h
            have he : first = best := Option.some.inj h
            subst first
            intro row hrow
            rcases List.mem_cons.mp hrow with he | hm
            · subst row
              exact Nat.le_refl _
            · exact (ih _ hr row hm).trans (Nat.le_of_not_gt hc)

/-- Prepending a tied maximum preserves the earlier user's row. -/
theorem highest_tie (first best : UserGate) (rest : List UserGate)
    (h : highest rest = some best) (he : first.level = best.level) :
    highest (first :: rest) = some first := by
  simp [highest, h, he]

def preferredUnit (card : CardFacts) : Option UnitId :=
  match card.support with
  | some unit => some unit
  | none => if card.original = piapro then none else some card.original

def select (card : CardFacts) (rows : List UserGate) : Option UserGate :=
  match preferredUnit card with
  | some unit => rows.find? (fun row => row.unit == unit)
  | none => highest rows

abbrev RateTable := UnitId → Nat → Option Nat

def rate (table : RateTable) (card : CardFacts) (rows : List UserGate) : Nat :=
  match select card rows with
  | none => 0
  | some row => (table row.unit row.level).getD 0

@[simp] theorem no_selected_gate (table : RateTable) (card : CardFacts)
    (rows : List UserGate) (h : select card rows = none) :
    rate table card rows = 0 := by simp [rate, h]

theorem selected_missing_level (table : RateTable) (card : CardFacts)
    (rows : List UserGate) (row : UserGate)
    (h : select card rows = some row) (missing : table row.unit row.level = none) :
    rate table card rows = 0 := by simp [rate, h, missing]

@[simp] theorem support_preferred (original unit : UnitId) (attr : Attribute) :
    preferredUnit ⟨original, some unit, attr⟩ = some unit := rfl

theorem bare_virtual_highest (attr : Attribute) (rows : List UserGate) :
    select ⟨piapro, none, attr⟩ rows = highest rows := by
  simp [select, preferredUnit]

/-- Level-table availability does not affect which row is chosen. -/
theorem no_fallback_from_missing_highest (table : RateTable)
    (attr : Attribute) (rows : List UserGate) (row : UserGate)
    (h : highest rows = some row) (missing : table row.unit row.level = none) :
    rate table ⟨piapro, none, attr⟩ rows = 0 := by
  apply selected_missing_level table _ rows row
  · simpa [bare_virtual_highest] using h
  · exact missing

end Allium.Gate
