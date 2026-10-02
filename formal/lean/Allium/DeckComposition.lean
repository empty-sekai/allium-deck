import Allium.Composition

/-!
# Deck membership, mixed-unit activation, and shuffle composition

Original units and optional support units have separate roles. All-match uses
membership in their union; mixed-unit activation processes support units in
member order; shuffle counts only original units.
-/
namespace Allium.DeckComposition
open PowerModel

abbrev piapro : UnitId := 5

structure CardFacts where
  original : UnitId
  support : Option UnitId
  attr : Attribute
  deriving DecidableEq

def membership (card : CardFacts) : Finset UnitId :=
  {card.original} ∪ card.support.toFinset

@[simp] theorem mem_membership (card : CardFacts) (unit : UnitId) :
    unit ∈ membership card ↔ unit = card.original ∨ card.support = some unit := by
  simp [membership]

def sharedUnits : List CardFacts → Finset UnitId
  | [] => Finset.univ
  | card :: rest => membership card ∩ sharedUnits rest

@[simp] theorem mem_sharedUnits (deck : List CardFacts) (unit : UnitId) :
    unit ∈ sharedUnits deck ↔ ∀ card ∈ deck, unit ∈ membership card := by
  induction deck with
  | nil => simp [sharedUnits]
  | cons card rest ih => simp [sharedUnits, ih]

def allMatch (deck : List CardFacts) (unit : UnitId) : Bool :=
  decide (unit ∈ sharedUnits deck)

theorem allMatch_iff (deck : List CardFacts) (unit : UnitId) :
    allMatch deck unit = true ↔
      ∀ card ∈ deck, unit = card.original ∨ card.support = some unit := by
  simp [allMatch]

def nonVirtualUnits (deck : List CardFacts) : Finset UnitId :=
  ((deck.filter (fun card => card.original != piapro)).map CardFacts.original).toFinset

structure MixedState where
  units : Finset UnitId
  needsVirtual : Bool
  deriving DecidableEq

def supportStep (state : MixedState) (card : CardFacts) : MixedState :=
  if card.original != piapro then state else
    match card.support with
    | none => { state with needsVirtual := true }
    | some unit =>
        if unit ∈ state.units then { state with needsVirtual := true }
        else { state with units := insert unit state.units }

def mixedState (deck : List CardFacts) : MixedState :=
  deck.foldl supportStep ⟨nonVirtualUnits deck, false⟩

def mixedUnits (deck : List CardFacts) : Finset UnitId :=
  let state := mixedState deck
  if state.needsVirtual then insert piapro state.units else state.units

def isMultiUnit (deck : List CardFacts) : Bool :=
  decide (1 < (mixedUnits deck).card)

def originalUnits (deck : List CardFacts) : Finset UnitId :=
  (deck.map CardFacts.original).toFinset

def shuffleBonus (isFinale3 : Bool) (deck : List CardFacts) : Nat :=
  if isFinale3 then
    match (originalUnits deck).card with
    | 3 => 10
    | 4 => 30
    | 5 => 50
    | _ => 0
  else 0

@[simp] theorem shuffle_disabled (deck : List CardFacts) :
    shuffleBonus false deck = 0 := by simp [shuffleBonus]

theorem shuffle_le_fifty (isFinale3 : Bool) (deck : List CardFacts) :
    shuffleBonus isFinale3 deck ≤ 50 := by
  unfold shuffleBonus
  split
  · split <;> decide
  · decide

private def bareVirtual : CardFacts := ⟨piapro, none, 0⟩
private def supportedVirtual : CardFacts := ⟨piapro, some 0, 0⟩
private def human : CardFacts := ⟨0, none, 0⟩

/-- Repeated bare virtual singers do not activate mixed-unit area effects. -/
theorem bare_virtual_not_multi :
    isMultiUnit (List.replicate 5 bareVirtual) = false := by decide

/-- Repeated support units retain a virtual-singer contribution. -/
theorem same_support_virtual_multi :
    isMultiUnit (List.replicate 5 supportedVirtual) = true := by decide

/-- All-match and mixed-unit activation can hold simultaneously. -/
theorem all_match_and_multi :
    let deck := [human, human, human, human, supportedVirtual]
    allMatch deck 0 = true ∧ isMultiUnit deck = true := by decide

/-- Support units do not contribute to the shuffle count. -/
theorem all_virtual_shuffle_zero :
    shuffleBonus true (List.replicate 5 supportedVirtual) = 0 := by decide

end Allium.DeckComposition
