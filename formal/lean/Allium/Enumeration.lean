import Allium.DynamicProgramming
import Allium.Canonical

/-!
# Ordered five-card decks and public slot roles

The specification uses `List.Forall₂` (one membership proposition per slot),
while the enumerator uses the Cartesian skip/take implementation proved in
DynamicProgramming. Cultivation variants are NOT deduplicated before scoring.
Fixed characters begin AFTER fixed cards, matching context.rs. An ordinary
forced leader needs membership; a Final leader additionally occupies slot 0.

Source: context.rs::{card_matches_slot,deck_matches_forced_leader}, placement.rs.
-/
namespace Allium.Enumeration

variable {Card : Type*}

structure Roles where
  fixedCards : List ℕ
  fixedCharacters : List ℕ
  forcedLeader : Option ℕ
  finalChapter : Bool
  /-- Invalid requests are rejected before forming a mathematical input. -/
  capacity : fixedCards.length + fixedCharacters.length ≤ 5

def matchesValue (wanted : Option ℕ) (actual : ℕ) : Prop :=
  match wanted with
  | none => True
  | some value => actual = value

/-- Fixed characters are offset by the number of fixed CARD slots. -/
def Roles.characterAt (roles : Roles) (slot : ℕ) : Option ℕ :=
  if roles.fixedCards.length ≤ slot then
    roles.fixedCharacters[slot - roles.fixedCards.length]?
  else none

def slotAllowed (publicId character : Card → ℕ) (roles : Roles)
    (slot : ℕ) (card : Card) : Prop :=
  matchesValue roles.fixedCards[slot]? (publicId card) ∧
  matchesValue (roles.characterAt slot) (character card) ∧
  (roles.finalChapter = true ∧ slot = 0 → matchesValue roles.forcedLeader (character card))

noncomputable def slotPool (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) (slot : ℕ) : Finset Card := by
  classical
  exact pool.filter (slotAllowed publicId character roles slot)

noncomputable def choices (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) : List (Finset Card) :=
  (List.range 5).map (slotPool pool publicId character roles)

@[simp] theorem choices_length (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) : (choices pool publicId character roles).length = 5 := by simp [choices]

/-- Legality is independent of the enumerator's recursion. -/
def ValidDeck (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) (uniqueCharacters : Bool) (deck : List Card) : Prop :=
  List.Forall₂ (fun options card => card ∈ options)
    (choices pool publicId character roles) deck ∧
  (deck.map publicId).Nodup ∧
  (uniqueCharacters = true → (deck.map character).Nodup) ∧
  (∀ leader, roles.forcedLeader = some leader → ∃ card ∈ deck, character card = leader)

noncomputable def allValidDecks (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) (uniqueCharacters : Bool) : Finset (List Card) := by
  classical
  exact (DP.assignments (choices pool publicId character roles)).filter (fun deck =>
    (deck.map publicId).Nodup ∧
    (uniqueCharacters = true → (deck.map character).Nodup) ∧
    (∀ leader, roles.forcedLeader = some leader → ∃ card ∈ deck, character card = leader))

/-- The actual finite enumerator contains every and only legal ordered deck. -/
@[simp] theorem exhaustive_complete (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) (uniqueCharacters : Bool) (deck : List Card) :
    deck ∈ allValidDecks pool publicId character roles uniqueCharacters ↔
      ValidDeck pool publicId character roles uniqueCharacters deck := by
  classical
  simp only [allValidDecks, Finset.mem_filter, DP.mem_assignments, ValidDeck]

/-- Five positions are enforced even when every slot is unconstrained. -/
theorem valid_length (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) (uniqueCharacters : Bool) (deck : List Card)
    (h : ValidDeck pool publicId character roles uniqueCharacters deck) : deck.length = 5 := by
  have hl := h.1.length_eq
  simpa using hl.symm

/-- Forall₂ cannot introduce a card that belongs to none of the slot pools. -/
theorem member_from_choices (groups : List (Finset Card)) (deck : List Card)
    (h : List.Forall₂ (fun options card => card ∈ options) groups deck)
    (card : Card) (hc : card ∈ deck) : ∃ options ∈ groups, card ∈ options := by
  induction h with
  | nil => simp at hc
  | @cons options member groups deck hm hrest ih =>
      rcases List.mem_cons.mp hc with heq | ht
      · subst card
        exact ⟨options, List.mem_cons_self, hm⟩
      · rcases ih ht with ⟨group, hg, hcard⟩
        exact ⟨group, List.mem_cons_of_mem _ hg, hcard⟩

theorem valid_card_in_pool (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) (uniqueCharacters : Bool) (deck : List Card)
    (h : ValidDeck pool publicId character roles uniqueCharacters deck)
    (card : Card) (hc : card ∈ deck) : card ∈ pool := by
  classical
  rcases member_from_choices _ _ h.1 card hc with ⟨options, ho, hm⟩
  rcases List.mem_map.mp ho with ⟨slot, _, heq⟩
  subst options
  exact (Finset.mem_filter.mp hm).1

/-- Fixing a public ID keeps ALL of its variants that meet the same role. -/
theorem same_public_role (pool : Finset Card) (publicId character : Card → ℕ)
    (roles : Roles) (slot : ℕ) (a b : Card) (hpool : b ∈ pool)
    (hid : publicId a = publicId b) (hchar : character a = character b)
    (ha : a ∈ slotPool pool publicId character roles slot) :
    b ∈ slotPool pool publicId character roles slot := by
  classical
  rcases Finset.mem_filter.mp ha with ⟨_, hrole⟩
  apply Finset.mem_filter.mpr
  exact ⟨hpool, by simpa only [slotAllowed, hid, hchar] using hrole⟩

/-- Dense-variant identity is preserved through the full ordered enumeration;
only the final canonical collection identifies equal public card sets. -/
noncomputable def exhaustiveKeys (pool : Finset Card) (publicId denseId character : Card → ℕ)
    (roles : Roles) (uniqueCharacters : Bool) (score power : List Card → ℕ) :
    Finset Canonical.Key :=
  (allValidDecks pool publicId character roles uniqueCharacters).image
    (Canonical.ofDeck publicId denseId score power)

@[simp] theorem exhaustive_keys_complete (pool : Finset Card)
    (publicId denseId character : Card → ℕ) (roles : Roles) (uniqueCharacters : Bool)
    (score power : List Card → ℕ) (key : Canonical.Key) :
    key ∈ exhaustiveKeys pool publicId denseId character roles uniqueCharacters score power ↔
      ∃ deck, ValidDeck pool publicId character roles uniqueCharacters deck ∧
        Canonical.ofDeck publicId denseId score power deck = key := by
  simp [exhaustiveKeys, Finset.mem_image]

/-- Public IDs 0 and 65535 and character 0 have no sentinel meaning. -/
theorem boundary_ids_are_values :
    matchesValue (some 0) 0 ∧ matchesValue (some 65535) 65535 ∧ ¬ matchesValue (some 0) 1 := by simp [matchesValue]

end Allium.Enumeration
