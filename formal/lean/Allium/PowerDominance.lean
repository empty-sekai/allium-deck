import Allium.MixedPower

/-!
# Statewise power dominance

The checked comparison preserves card facts and the unit-profile selector,
and compares every legacy and optional mixed table entry. This implies total
power monotonicity after a slot substitution, including simultaneous all-match
and mixed-unit activation. Optional-sidecar presence is pool-wide.
-/
namespace Allium.PowerDominance
open PowerModel MixedPower

def MultiLE : Option (Fin 8 → Nat) → Option (Fin 8 → Nat) → Prop
  | none, none => True
  | some left, some right => ∀ state, left state ≤ right state
  | _, _ => False

structure Dominates (weaker stronger : Card) : Prop where
  facts : weaker.facts = stronger.facts
  emptyMask : weaker.emptyMask = stronger.emptyMask
  profiles : weaker.profile = stronger.profile
  values : ∀ state, weaker.legacyValues state ≤ stronger.legacyValues state
  mixed : MultiLE weaker.multiValues stronger.multiValues

theorem dominates_refl (card : Card) : Dominates card card := by
  refine ⟨rfl, rfl, rfl, fun _ => Nat.le_refl _, ?_⟩
  cases card.multiValues <;> simp [MultiLE]

theorem dominates_trans (a b c : Card) (first : Dominates a b) (second : Dominates b c) :
    Dominates a c := by
  refine ⟨first.facts.trans second.facts, first.emptyMask.trans second.emptyMask,
    first.profiles.trans second.profiles,
    fun state => (first.values state).trans (second.values state), ?_⟩
  have hab := first.mixed
  have hbc := second.mixed
  cases ha : a.multiValues <;> cases hb : b.multiValues <;> cases hc : c.multiValues <;>
    simp only [MultiLE, ha, hb, hc] at hab hbc ⊢
  exact fun state => (hab state).trans (hbc state)

theorem legacy_mono (weaker stronger : Card) (guard : Dominates weaker stronger)
    (common : Finset UnitId) (attr : Bool) :
    resolved (legacy weaker) common attr ≤ resolved (legacy stronger) common attr := by
  apply Finset.sup_le
  intro unit hu
  have hm : unit ∈ (legacy stronger).units := by simpa only [legacy, guard.facts, guard.emptyMask] using hu
  have hv : unitValue (legacy weaker) common attr unit ≤ unitValue (legacy stronger) common attr unit := by
    unfold unitValue legacy
    simp only
    rw [guard.profiles]
    exact guard.values _
  exact hv.trans (Finset.le_sup hm)

theorem effective_mono (mode : Mode) (mixed : Bool) (common : Finset UnitId) (attr : Bool)
    (weaker stronger : Card) (guard : Dominates weaker stronger) :
    effective mode mixed common attr weaker ≤ effective mode mixed common attr stronger := by
  have hm := guard.mixed
  cases hw : weaker.multiValues with
  | none =>
      cases hs : stronger.multiValues with
      | none => simpa only [effective, hw, hs] using legacy_mono weaker stronger guard common attr
      | some values => simp [MultiLE, hw, hs] at hm
  | some oldValues =>
      cases hs : stronger.multiValues with
      | none => simp [MultiLE, hw, hs] at hm
      | some newValues =>
          simp only [MultiLE, hw, hs] at hm
          simp only [effective, hw, hs]
          by_cases enabledNow : enabled mode mixed = true
          · simp only [if_pos enabledNow, originalAll, supportAll, guard.facts, guard.emptyMask]
            exact hm _
          · simp only [if_neg enabledNow]
            exact legacy_mono weaker stronger guard common attr

theorem facts_map_eq (left right : List Card) (related : List.Forall₂ Dominates left right) :
    left.map Card.facts = right.map Card.facts := by
  induction related with
  | nil => rfl
  | cons guard rest ih => simp only [List.map_cons, guard.facts, ih]

theorem empty_masks_map_eq (left right : List Card) (related : List.Forall₂ Dominates left right) :
    left.map Card.emptyMask = right.map Card.emptyMask := by
  induction related with
  | nil => rfl
  | cons guard rest ih => simp only [List.map_cons, guard.emptyMask, ih]

theorem active_deck_facts (left right : List Card) (facts : left.map Card.facts = right.map Card.facts)
    (empties : left.map Card.emptyMask = right.map Card.emptyMask) :
    deckFacts id left = deckFacts id right := by
  unfold deckFacts
  simp only [id_eq]
  induction left generalizing right with
  | nil => cases right <;> simp_all
  | cons first rest ih =>
      cases right with
      | nil => simp at facts
      | cons next tail =>
          simp only [List.map_cons, List.cons.injEq] at facts empties
          simp only [List.filterMap_cons]
          rw [ih tail facts.2 empties.2]
          simp only [activeFact, facts.1, empties.1]

theorem common_of_facts (left right : List Card) (facts : left.map Card.facts = right.map Card.facts)
    (empties : left.map Card.emptyMask = right.map Card.emptyMask) :
    commonUnits legacy left = commonUnits legacy right := by
  induction left generalizing right with
  | nil => cases right <;> simp_all [commonUnits]
  | cons first rest ih =>
      cases right with
      | nil => simp at facts
      | cons next tail =>
          simp only [List.map_cons, List.cons.injEq] at facts empties
          simp only [commonUnits, legacy]
          rw [facts.1, empties.1, ih tail facts.2 empties.2]

theorem uniform_of_facts (left right : List Card) (facts : left.map Card.facts = right.map Card.facts)
    (attr : Attribute) : UniformAt legacy left attr ↔ UniformAt legacy right attr := by
  constructor
  · intro h card member
    have hm : card.facts ∈ left.map Card.facts := by
      rw [facts]
      exact List.mem_map.mpr ⟨card, member, rfl⟩
    obtain ⟨original, ho, he⟩ := List.mem_map.mp hm
    have ha := h original ho
    simpa only [legacy, he] using ha
  · intro h card member
    have hm : card.facts ∈ right.map Card.facts := by
      rw [← facts]
      exact List.mem_map.mpr ⟨card, member, rfl⟩
    obtain ⟨original, ho, he⟩ := List.mem_map.mp hm
    have ha := h original ho
    simpa only [legacy, he] using ha

theorem attribute_of_facts (left right : List Card) (facts : left.map Card.facts = right.map Card.facts) :
    sharesAttribute legacy left = sharesAttribute legacy right := by
  classical
  have he : (∃ attr, UniformAt legacy left attr) = (∃ attr, UniformAt legacy right attr) :=
    propext (exists_congr (uniform_of_facts left right facts))
  simp only [sharesAttribute, he]

theorem card_power_mono (mode : Mode) (left right : List Card)
    (facts : left.map Card.facts = right.map Card.facts)
    (empties : left.map Card.emptyMask = right.map Card.emptyMask) (weaker stronger : Card)
    (guard : Dominates weaker stronger) :
    MixedPower.cardPower mode id left weaker ≤ MixedPower.cardPower mode id right stronger := by
  have hl : left.length = right.length := by simpa using congrArg List.length facts
  unfold MixedPower.cardPower
  simp only [id_eq]
  rw [hl, common_of_facts left right facts empties, attribute_of_facts left right facts,
    active_deck_facts left right facts empties]
  exact effective_mono mode _ _ _ weaker stronger guard

theorem sum_related {A B : Type*} (left : List A) (right : List B) (f : A → Nat) (g : B → Nat)
    (related : List.Forall₂ (fun a b => f a ≤ g b) left right) :
    (left.map f).sum ≤ (right.map g).sum := by
  induction related with
  | nil => exact Nat.le_refl _
  | cons h rest ih => simpa only [List.map_cons, List.sum_cons] using Nat.add_le_add h ih

theorem total_mono (mode : Mode) (left right : List Card) (related : List.Forall₂ Dominates left right) :
    MixedPower.total mode id left ≤ MixedPower.total mode id right := by
  have facts := facts_map_eq left right related
  have empties := empty_masks_map_eq left right related
  apply sum_related
  apply related.imp
  intro weaker stronger guard
  exact card_power_mono mode left right facts empties weaker stronger guard

theorem capped_objective_mono (mode : Mode) (honor : Nat) (cap : Option Nat)
    (left right : List Card) (related : List.Forall₂ Dominates left right) :
    clamp cap (MixedPower.total mode id left + honor) ≤
      clamp cap (MixedPower.total mode id right + honor) :=
  clamp_monotone cap (Nat.add_le_add_right (total_mono mode left right related) honor)

/-- ForceOff skips the sidecar comparison, while retaining every legacy
comparison. The other policies compare the complete optional table. -/
structure PolicyDominates (mode : Mode) (weaker stronger : Card) : Prop where
  baseline : Dominates { weaker with multiValues := none } { stronger with multiValues := none }
  mixed : mode ≠ .forceOff → MultiLE weaker.multiValues stronger.multiValues

theorem policy_effective_mono (mode : Mode) (mixed : Bool) (common : Finset UnitId) (attr : Bool)
    (weaker stronger : Card) (guard : PolicyDominates mode weaker stronger) :
    effective mode mixed common attr weaker ≤ effective mode mixed common attr stronger := by
  by_cases off : mode = .forceOff
  · subst mode
    simp only [force_off_legacy]
    exact legacy_mono { weaker with multiValues := none } { stronger with multiValues := none }
      guard.baseline common attr
  · exact effective_mono mode mixed common attr weaker stronger
      ⟨guard.baseline.facts, guard.baseline.emptyMask, guard.baseline.profiles,
        guard.baseline.values, guard.mixed off⟩

theorem policy_total_mono (mode : Mode) (left right : List Card)
    (related : List.Forall₂ (PolicyDominates mode) left right) :
    MixedPower.total mode id left ≤ MixedPower.total mode id right := by
  have both : left.map Card.facts = right.map Card.facts ∧
      left.map Card.emptyMask = right.map Card.emptyMask := by
    induction related with
    | nil => exact ⟨rfl, rfl⟩
    | cons guard rest ih =>
        exact ⟨congrArg₂ List.cons guard.baseline.facts ih.1,
          congrArg₂ List.cons guard.baseline.emptyMask ih.2⟩
  have facts := both.1
  have empties := both.2
  have lengths : left.length = right.length := by simpa using congrArg List.length facts
  apply sum_related
  apply related.imp
  intro weaker stronger guard
  unfold MixedPower.cardPower
  simp only [id_eq]
  rw [lengths, common_of_facts left right facts empties, attribute_of_facts left right facts,
    active_deck_facts left right facts empties]
  exact policy_effective_mono mode _ _ _ weaker stronger guard

end Allium.PowerDominance
