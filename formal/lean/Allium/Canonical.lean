import Allium.Collection
import Mathlib.Data.Prod.Lex
import Mathlib.Data.List.Lex

/-!
# The five-field canonical result key

Descending objective, descending resolved MySekai power, sorted public IDs,
ordered public IDs, and ordered dense cultivation variants. Naturals model the
validated unsigned fields; no value is reserved as an empty-slot sentinel.

Source: tracker.rs ResultKey / PublicSetKey; problem.rs Objective.
-/
namespace Allium.Canonical

abbrev Key := OrderDual ℕ ×ₗ (OrderDual ℕ ×ₗ (List ℕ ×ₗ (List ℕ ×ₗ List ℕ)))

def make (objective resolvedPower : ℕ) (sortedIds orderedIds variants : List ℕ) : Key :=
  toLex (OrderDual.toDual objective,
    toLex (OrderDual.toDual resolvedPower, toLex (sortedIds, toLex (orderedIds, variants))))

def score (key : Key) : ℕ := key.1
def power (key : Key) : ℕ := key.2.1
def identity (key : Key) : List ℕ := key.2.2.1
def orderedIds (key : Key) : List ℕ := key.2.2.2.1
def variants (key : Key) : List ℕ := key.2.2.2.2

/-- Strict objective improvement wins before any tie-break field is read. -/
theorem score_order (a b : Key) (h : score b < score a) : a < b :=
  Prod.Lex.lt_iff.mpr (Or.inl h)

/-- Full priority order; equal objective values must keep all later fields. -/
theorem key_lt_iff (s p s' p' : ℕ) (ids order dense ids' order' dense' : List ℕ) :
    make s p ids order dense < make s' p' ids' order' dense' ↔
      s' < s ∨ (s = s' ∧ (p' < p ∨ (p = p' ∧
        (ids < ids' ∨ (ids = ids' ∧ (order < order' ∨ (order = order' ∧ dense < dense'))))))) := by
  simp only [make, Prod.Lex.toLex_lt_toLex, OrderDual.toDual_lt_toDual,
    OrderDual.toDual_inj]

/-- Negation within a validated unsigned width represents ascending Power. -/
theorem minimizing_power_order (width a b : ℕ) (ha : a ≤ width) (hb : b ≤ width) :
    width - b < width - a ↔ a < b := by omega

/-- A deck key retains dense variants even when its public card set is equal. -/
def ofDeck {Card : Type*} (publicId denseId : Card → ℕ)
    (objective resolvedPower : List Card → ℕ) (deck : List Card) : Key :=
  make (objective deck) (resolvedPower deck)
    ((deck.map publicId).toFinset.sort (· ≤ ·)) (deck.map publicId) (deck.map denseId)

@[simp] theorem deck_variants {Card : Type*} (publicId denseId : Card → ℕ)
    (objective resolvedPower : List Card → ℕ) (deck : List Card) :
    variants (ofDeck publicId denseId objective resolvedPower deck) = deck.map denseId := rfl

@[simp] theorem deck_score {Card : Type*} (publicId denseId : Card → ℕ)
    (objective resolvedPower : List Card → ℕ) (deck : List Card) :
    score (ofDeck publicId denseId objective resolvedPower deck) = objective deck := rfl

/-- Public identity is exactly the sorted public-ID set, independent of
placement, cultivation choice, objective score or result insertion order. -/
@[simp] theorem deck_identity {Card : Type*} (publicId denseId : Card → ℕ)
    (objective resolvedPower : List Card → ℕ) (deck : List Card) :
    (identity (ofDeck publicId denseId objective resolvedPower deck)).toFinset =
      (deck.map publicId).toFinset := by
  change (((deck.map publicId).toFinset).sort (· ≤ ·)).toFinset =
    (deck.map publicId).toFinset
  exact Finset.sort_toFinset (· ≤ ·) _

/-- No two distinct ordered decks collapse to one result key when dense
indices identify variants injectively. -/
theorem deck_key_injective {Card : Type*} (publicId denseId : Card → ℕ)
    (hdense : Function.Injective denseId) (objective resolvedPower : List Card → ℕ) :
    Function.Injective (ofDeck publicId denseId objective resolvedPower) := by
  intro a b h
  have hmap : a.map denseId = b.map denseId := congrArg variants h
  exact List.map_injective_iff.mpr hdense hmap

/-- Instantiation of generic Top-K cardinality at the actual five-field key. -/
theorem result_count (k : ℕ) (keys : Finset Key) :
    (topK identity k keys).card = min k (keys.image identity).card :=
  topK_card identity k keys

/-- Instantiation of generic collection exactness at the actual key. -/
theorem result_collection_exact (k : ℕ) (keys : List Key) :
    collect identity k keys = topK identity k keys.toFinset := collect_exact identity k keys

/-- Equal-score IDs 65535 and 0 are both ordinary identity values. -/
theorem max_u16_is_not_sentinel :
    identity (make 1 0 [65535] [65535] [0]) ≠ identity (make 1 0 [] [] []) := by decide

/-- Equal score and power are decided by public IDs before dense variants. -/
theorem public_id_tie_regression :
    make 100 20 [0, 1, 2, 3, 65535] [0, 1, 2, 3, 65535] [9, 8, 7, 6, 5] <
      make 100 20 [0, 1, 2, 4, 5] [0, 1, 2, 4, 5] [0, 1, 2, 3, 4] := by decide

end Allium.Canonical
