import Allium.NumericPower
import Allium.Enumeration

/-!
# Numeric search slot guards

Fixed characters begin after the fixed-card prefix. A repeated character may
occupy an explicitly fixed-character slot, but a free slot never gets that
exception. Public card IDs remain distinct in every slot. The free-frontier
uniqueness assumptions follow from these guards after the fixed prefix.
-/
namespace Allium.NumericSlots
open Enumeration

variable {Card : Type*}

def fixedPrefix (roles : Roles) : Nat := roles.fixedCards.length + roles.fixedCharacters.length

theorem character_after_prefix (roles : Roles) (slot : Nat) (free : fixedPrefix roles ≤ slot) :
    roles.characterAt slot = none := by
  have hc : roles.fixedCards.length ≤ slot := by unfold fixedPrefix at free; omega
  have hr : roles.fixedCharacters.length ≤ slot - roles.fixedCards.length := by
    unfold fixedPrefix at free
    omega
  simp only [Roles.characterAt, if_pos hc]
  exact List.getElem?_eq_none hr

def Allowed (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (unique : Bool) (selected : List Card) (card : Card) : Prop :=
  selected.length < 5 ∧ card ∈ pool ∧
  matchesValue roles.fixedCards[selected.length]? (publicId card) ∧
  matchesValue (roles.characterAt selected.length) (character card) ∧
  publicId card ∉ selected.map publicId ∧
  (unique = true → character card ∈ selected.map character →
    roles.characterAt selected.length = some (character card))

/-- Guard sequence of the numeric recursion; final scoring constraints remain
leaf predicates and are not conflated with candidate filtering. -/
def Steps (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (unique : Bool) (selected : List Card) : List Card → Prop
  | [] => True
  | card :: rest => Allowed pool publicId character roles unique selected card ∧
      Steps pool publicId character roles unique (selected ++ [card]) rest

theorem free_character_fresh (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (selected : List Card) (card : Card) (free : fixedPrefix roles ≤ selected.length)
    (allowed : Allowed pool publicId character roles true selected card) :
    character card ∉ selected.map character := by
  intro repeated
  have required := allowed.2.2.2.2.2 rfl repeated
  rw [character_after_prefix roles selected.length free] at required
  contradiction

/-- After entering the free suffix, the remaining characters are pairwise
unique and disjoint from every selected character, even if fixed slots used
an explicit repeated-character exception earlier. -/
theorem free_suffix_unique (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (selected free : List Card) (afterFixed : fixedPrefix roles ≤ selected.length)
    (legal : Steps pool publicId character roles true selected free) :
    (free.map character).Nodup ∧
      ∀ card ∈ free, character card ∉ selected.map character := by
  induction free generalizing selected with
  | nil => simp
  | cons card rest ih =>
      rcases legal with ⟨allowed, restLegal⟩
      have fresh := free_character_fresh pool publicId character roles selected card afterFixed allowed
      have depth : fixedPrefix roles ≤ (selected ++ [card]).length := by
        simp only [List.length_append, List.length_singleton]
        omega
      obtain ⟨uniqueRest, outside⟩ := ih (selected ++ [card]) depth restLegal
      constructor
      · simp only [List.map_cons, List.nodup_cons]
        refine ⟨?_, uniqueRest⟩
        intro member
        obtain ⟨other, ho, he⟩ := List.mem_map.mp member
        have hn := outside other ho
        apply hn
        simp [he]
      · intro other ho
        rcases List.mem_cons.mp ho with he | hr
        · subst other
          exact fresh
        · intro hm
          apply outside other hr
          exact List.mem_map.mpr (by
            obtain ⟨old, hold, he⟩ := List.mem_map.mp hm
            exact ⟨old, List.mem_append_left _ hold, he⟩)

/-- The public-ID guard preserves all cultivation variants as alternatives
while preventing two variants of the same public card in one deck. -/
theorem public_ids_unique (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (unique : Bool) (selected rest : List Card)
    (initial : (selected.map publicId).Nodup)
    (legal : Steps pool publicId character roles unique selected rest) :
    ((selected ++ rest).map publicId).Nodup := by
  induction rest generalizing selected with
  | nil => simpa using initial
  | cons card rest ih =>
      rcases legal with ⟨allowed, restLegal⟩
      have fresh := allowed.2.2.2.2.1
      have next : ((selected ++ [card]).map publicId).Nodup := by
        simp only [List.map_append, List.map_singleton, List.nodup_append, List.nodup_singleton, true_and]
        refine ⟨initial, ?_⟩
        intro id hi other ho he
        have eq : other = publicId card := List.mem_singleton.mp ho
        exact fresh (he.trans eq ▸ hi)
      simpa only [List.append_assoc, List.singleton_append] using ih (selected ++ [card]) next restLegal

theorem steps_members (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (unique : Bool) (selected rest : List Card)
    (legal : Steps pool publicId character roles unique selected rest) :
    ∀ card ∈ rest, card ∈ pool := by
  induction rest generalizing selected with
  | nil => simp
  | cons first rest ih =>
      intro card member
      rcases List.mem_cons.mp member with he | hr
      · subst card
        exact legal.1.2.1
      · exact ih (selected ++ [first]) legal.2 card hr

/-- A five-card guarded traversal satisfies the concrete numeric acceptance
theorem's pool and distinct-card domain; these are derived, not assumed. -/
theorem complete_pool_domain (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (unique : Bool) (deck : List Card) (length : deck.length = 5)
    (legal : Steps pool publicId character roles unique [] deck) :
    NumericPower.LegalPoolDeck pool deck := by
  refine ⟨length, ?_, steps_members pool publicId character roles unique [] deck legal⟩
  have hids := public_ids_unique pool publicId character roles unique [] deck (by simp) legal
  have hmap : (deck.map publicId).Nodup := by simpa using hids
  exact List.Nodup.of_map publicId hmap

/-- Fixed slots can repeat an explicitly requested character. This guard is
not the global pairwise-character constraint of ordinary free selection. -/
theorem explicit_repeat_allowed (pool : Finset Card) (publicId character : Card → Nat)
    (roles : Roles) (selected : List Card) (card : Card)
    (space : selected.length < 5) (member : card ∈ pool)
    (fixedCard : matchesValue roles.fixedCards[selected.length]? (publicId card))
    (fixedCharacter : roles.characterAt selected.length = some (character card))
    (fresh : publicId card ∉ selected.map publicId) :
    Allowed pool publicId character roles true selected card := by
  refine ⟨space, member, fixedCard, ?_, fresh, ?_⟩
  · simp [matchesValue, fixedCharacter]
  · intro _ _
    exact fixedCharacter

/-- Concatenating guarded traversals carries the selected prefix forward. -/
theorem steps_append (pool : Finset Card) (publicId character : Card → Nat) (roles : Roles)
    (unique : Bool) (selected first second : List Card) :
    Steps pool publicId character roles unique selected (first ++ second) ↔
      Steps pool publicId character roles unique selected first ∧
        Steps pool publicId character roles unique (selected ++ first) second := by
  induction first generalizing selected with
  | nil => simp [Steps]
  | cons card rest ih =>
      simp only [List.cons_append, Steps, ih, List.append_assoc, List.nil_append, and_assoc]

/-- Distinct mapped labels imply injectivity among the actual list members. -/
theorem map_nodup_injective {Label : Type*} (labels : Card → Label) (cards : List Card)
    (unique : (cards.map labels).Nodup) :
    ∀ a ∈ cards, ∀ b ∈ cards, labels a = labels b → a = b := by
  induction cards with
  | nil => simp
  | cons first rest ih =>
      have hn := List.nodup_cons.mp unique
      intro a ha b hb same
      rcases List.mem_cons.mp ha with ha | ha
      · subst a
        rcases List.mem_cons.mp hb with hb | hb
        · exact hb.symm
        · exact False.elim (hn.1 (List.mem_map.mpr ⟨b, hb, same.symm⟩))
      · rcases List.mem_cons.mp hb with hb | hb
        · subst b
          exact False.elim (hn.1 (List.mem_map.mpr ⟨a, ha, same⟩))
        · exact ih hn.2 a ha b hb same

/-- The numeric slot guards supply the character injectivity, unused-label
set, and remaining-slot capacity required by the concrete suffix scan. -/
theorem guarded_free_scan [DecidableEq Card]
    (pool : Finset Card) (publicId : Card → Nat) (character : Card → Fin 27) (roles : Roles)
    (mode : MixedPower.Mode) (data : Card → MixedPower.Card)
    (selected free cards : List Card) (start : Nat)
    (afterFixed : fixedPrefix roles ≤ selected.length)
    (legal : Steps pool publicId (fun card => (character card).val) roles true selected free)
    (complete : (selected ++ free).length = 5)
    (suffix : ∀ card ∈ free, card ∈ SuffixScan.densePool cards start) :
    (free.map (MixedPower.cardPower mode data (selected ++ free))).sum ≤
      SuffixScan.scanBound cards start ((selected.map (fun card => (character card).val)).toFinset)
        (5 - selected.length) character (fun card => MixedPower.maximum (data card)) := by
  obtain ⟨unique, outside⟩ := free_suffix_unique pool publicId (fun card => (character card).val)
    roles selected free afterFixed legal
  have distinct : free.Nodup := List.Nodup.of_map (fun card => (character card).val) unique
  rw [← SuffixScan.sum_toFinset (MixedPower.cardPower mode data (selected ++ free)) free distinct]
  apply SuffixScan.effective_scan_bound
  · intro card member
    exact suffix card (List.mem_toFinset.mp member)
  · have size := List.toFinset_card_le free
    simp only [List.length_append] at complete
    omega
  · intro a ha b hb same
    exact map_nodup_injective (fun card => (character card).val) free unique
      a (List.mem_toFinset.mp ha) b (List.mem_toFinset.mp hb) (congrArg Fin.val same)
  · intro card member selectedMember
    exact outside card (List.mem_toFinset.mp member) (List.mem_toFinset.mp selectedMember)

/-- Selected cards use their full effective-table maxima, while free cards
use the actual dense-suffix character scan. No character-uniqueness premise
is added beyond the numeric recursion's own slot checks. -/
theorem guarded_total_scan [DecidableEq Card]
    (pool : Finset Card) (publicId : Card → Nat) (character : Card → Fin 27) (roles : Roles)
    (mode : MixedPower.Mode) (data : Card → MixedPower.Card)
    (selected free cards : List Card) (start : Nat)
    (afterFixed : fixedPrefix roles ≤ selected.length)
    (legal : Steps pool publicId (fun card => (character card).val) roles true selected free)
    (complete : (selected ++ free).length = 5)
    (suffix : ∀ card ∈ free, card ∈ SuffixScan.densePool cards start) :
    MixedPower.total mode data (selected ++ free) ≤
      (selected.map (fun card => MixedPower.maximum (data card))).sum +
        SuffixScan.scanBound cards start ((selected.map (fun card => (character card).val)).toFinset)
          (5 - selected.length) character (fun card => MixedPower.maximum (data card)) := by
  have selectedBound : (selected.map (MixedPower.cardPower mode data (selected ++ free))).sum ≤
      (selected.map (fun card => MixedPower.maximum (data card))).sum :=
    List.sum_le_sum (fun card _ => MixedPower.effective_upper mode _ _ _ (data card))
  have freeBound := guarded_free_scan pool publicId character roles mode data selected free cards start
    afterFixed legal complete suffix
  simpa only [MixedPower.total, List.map_append, List.sum_append] using Nat.add_le_add selectedBound freeBound

end Allium.NumericSlots
