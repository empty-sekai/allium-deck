import Allium.MixedPower

/-!
# Dense-index sidecar transformations

The mixed table is optional for the whole pool. Reindexing moves the legacy
card record and its mixed row with the same map and preserves the evaluation
mode. Permutation round trips recover both tables without encoding mixed rows
inside the legacy profile bits.
-/
namespace Allium.SidecarLayout
open PowerModel DeckComposition MixedPower

structure LegacyCard where
  facts : CardFacts
  profile : UnitId → Bool
  values : Fin 8 → Nat
  emptyMask : Bool := false

structure Pool (Id : Type*) where
  cards : Id → LegacyCard
  mixed : Option (Id → Fin 8 → Nat)
  mode : Mode

def Pool.card {Id : Type*} (pool : Pool Id) (id : Id) : Card :=
  ⟨(pool.cards id).facts, (pool.cards id).profile, (pool.cards id).values,
    pool.mixed.map (fun rows => rows id), (pool.cards id).emptyMask⟩

@[simp] theorem pool_presence {Id : Type*} (pool : Pool Id) (id : Id) :
    (pool.card id).multiValues.isSome = pool.mixed.isSome := by
  unfold Pool.card
  cases pool.mixed <;> rfl

/-- Pool-wide optional storage supplies the activation-plan coverage premise
for every row, including empty membership rows. -/
theorem source_plan_covers {Id : Type*} (pool : Pool Id) (deck : List Id) (full : deck.length = 5) :
    ∃ plan ∈ sourcePlans pool.mode pool.mixed.isSome, ∀ id,
      MixedPower.cardPower pool.mode pool.card deck id ≤ planBound pool.mode (pool.card id) plan.1 plan.2 :=
  source_plans_cover pool.mode pool.mixed.isSome pool.card deck (pool_presence pool) full

def reindex {Old New : Type*} (pool : Pool Old) (index : New → Old) : Pool New :=
  ⟨pool.cards ∘ index, pool.mixed.map (fun rows => rows ∘ index), pool.mode⟩

@[simp] theorem reindex_card {Old New : Type*} (pool : Pool Old) (index : New → Old) (id : New) :
    (reindex pool index).card id = pool.card (index id) := by
  unfold reindex Pool.card
  cases pool.mixed <;> rfl

@[simp] theorem reindex_mode {Old New : Type*} (pool : Pool Old) (index : New → Old) :
    (reindex pool index).mode = pool.mode := rfl

theorem reindex_selector {Old New : Type*} (pool : Pool Old) (index : New → Old)
    (id : New) (mixed : Bool) (common : Finset UnitId) (attr : Bool) :
    effective (reindex pool index).mode mixed common attr ((reindex pool index).card id) =
      effective pool.mode mixed common attr (pool.card (index id)) := by simp

/-- No owned mixed effect means no allocation is required in any reindexing. -/
theorem absent_sidecar_preserved {Old New : Type*} (pool : Pool Old) (index : New → Old)
    (absent : pool.mixed = none) : (reindex pool index).mixed = none := by
  simp [reindex, absent]

theorem no_owned_effect_modes_agree {Id : Type*} (pool : Pool Id) (id : Id)
    (absent : pool.mixed = none) (first second : Mode)
    (mixed : Bool) (common : Finset UnitId) (attr : Bool) :
    effective first mixed common attr (pool.card id) = effective second mixed common attr (pool.card id) := by
  apply no_multi_mode_invariant
  simp [Pool.card, absent]

/-- Sort/restore uses inverse dense permutations for the complete card record. -/
theorem permutation_roundtrip {Old New : Type*} (pool : Pool Old) (index : New ≃ Old) :
    reindex (reindex pool index) index.symm = pool := by
  cases pool with
  | mk cards mixed mode =>
      cases mixed with
      | none => simp [reindex, Function.comp_def]
      | some rows => simp [reindex, Function.comp_def]

/-- Restriction/compaction may use any injective dense map; card semantics do
not rely on numeric proximity or on a packed representation of the new rows. -/
theorem composition_reindex {Old New : Type*} (pool : Pool Old) (index : New → Old)
    (deck : List New) :
    deck.map (fun id => ((reindex pool index).card id).facts) =
      (deck.map index).map (fun id => (pool.card id).facts) := by
  simp [List.map_map]

/-- The builder allocates mixed rows on the first retained mixed card. An
empty compaction performs no row writes and therefore leaves the sidecar absent. -/
def compactRows {Old : Type*} (pool : Pool Old) (kept : List Old) : Pool (Fin kept.length) :=
  { cards := fun id => pool.cards kept[id]
    mixed := if kept.length = 0 then none else pool.mixed.map (fun rows id => rows kept[id])
    mode := pool.mode }

@[simp] theorem compact_empty {Old : Type*} (pool : Pool Old) :
    (compactRows pool []).mixed = none := by simp [compactRows]

/-- Each copied legacy record and sidecar row uses the same retained index. -/
theorem compact_card {Old : Type*} (pool : Pool Old) (kept : List Old) (id : Fin kept.length) :
    (compactRows pool kept).card id = pool.card kept[id] := by
  have hn : kept.length ≠ 0 := by
    have hi := id.isLt
    omega
  unfold compactRows Pool.card
  simp only [if_neg hn]
  cases pool.mixed <;> rfl

/-- The dense loop visits all indices in increasing order and only keeps true
bits. Its next_idx is the rank within this stable filtered list. -/
def retainedIndices (count : Nat) (keep : Fin count → Bool) : List (Fin count) :=
  (List.finRange count).filter keep

def compact {count : Nat} (pool : Pool (Fin count)) (keep : Fin count → Bool) :
    Pool (Fin (retainedIndices count keep).length) :=
  compactRows pool (retainedIndices count keep)

@[simp] theorem retained_iff (count : Nat) (keep : Fin count → Bool) (card : Fin count) :
    card ∈ retainedIndices count keep ↔ keep card = true := by
  simp [retainedIndices]

theorem compact_effective {count : Nat} (pool : Pool (Fin count)) (keep : Fin count → Bool)
    (id : Fin (retainedIndices count keep).length)
    (mixed : Bool) (common : Finset UnitId) (attr : Bool) :
    effective (compact pool keep).mode mixed common attr ((compact pool keep).card id) =
      effective pool.mode mixed common attr (pool.card (retainedIndices count keep)[id]) := by
  change effective pool.mode mixed common attr
    ((compactRows pool (retainedIndices count keep)).card id) = _
  rw [compact_card]

theorem common_units_map {Old New : Type*} (data : Old → CardData) (index : New → Old)
    (deck : List New) :
    commonUnits (fun id => data (index id)) deck = commonUnits data (deck.map index) := by
  induction deck with
  | nil => rfl
  | cons first rest ih => simp [commonUnits, ih]

theorem attribute_map {Old New : Type*} (data : Old → CardData) (index : New → Old)
    (deck : List New) :
    sharesAttribute (fun id => data (index id)) deck = sharesAttribute data (deck.map index) := by
  classical
  simp [sharesAttribute, UniformAt]

/-- Moving rows preserves the complete evaluator, including the five-card
ALL_MATCH guard and the order-sensitive mixed-unit predicate. -/
theorem card_power_map {Old New : Type*} (mode : Mode) (data : Old → Card)
    (index : New → Old) (deck : List New) (id : New) :
    MixedPower.cardPower mode (fun card => data (index card)) deck id =
      MixedPower.cardPower mode data (deck.map index) (index id) := by
  unfold MixedPower.cardPower
  rw [common_units_map (fun card => legacy (data card)) index deck,
    attribute_map (fun card => legacy (data card)) index deck]
  simp [deckFacts, Function.comp_def]

theorem total_map {Old New : Type*} (mode : Mode) (data : Old → Card)
    (index : New → Old) (deck : List New) :
    MixedPower.total mode (fun card => data (index card)) deck =
      MixedPower.total mode data (deck.map index) := by
  simp only [MixedPower.total, List.map_map]
  apply congrArg List.sum
  apply List.map_congr_left
  intro card _
  exact card_power_map mode data index deck card

theorem reindex_total {Old New : Type*} (pool : Pool Old) (index : New → Old) (deck : List New) :
    MixedPower.total (reindex pool index).mode (reindex pool index).card deck =
      MixedPower.total pool.mode pool.card (deck.map index) := by
  have hd : (reindex pool index).card = fun id => pool.card (index id) :=
    funext (reindex_card pool index)
  rw [reindex_mode, hd]
  exact total_map pool.mode pool.card index deck

/-- Stable keep-filter compaction preserves full-deck effective power, not
only a single card evaluated against externally supplied flags. -/
theorem compact_total {count : Nat} (pool : Pool (Fin count)) (keep : Fin count → Bool)
    (deck : List (Fin (retainedIndices count keep).length)) :
    MixedPower.total (compact pool keep).mode (compact pool keep).card deck =
      MixedPower.total pool.mode pool.card
        (deck.map (fun id => (retainedIndices count keep)[id])) := by
  change MixedPower.total pool.mode (compactRows pool (retainedIndices count keep)).card deck = _
  have hd : (compactRows pool (retainedIndices count keep)).card =
      fun id => pool.card (retainedIndices count keep)[id] :=
    funext (compact_card pool (retainedIndices count keep))
  rw [hd]
  exact total_map pool.mode pool.card (fun id => (retainedIndices count keep)[id]) deck

end Allium.SidecarLayout
