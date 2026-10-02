import Allium.SuffixDelta
import Allium.SimdMask

/-!
# Compact exclusion storage

The storage position is the number of selected character IDs below the queried
ID, i.e. the population count below its mask bit. Distinct selected IDs have
distinct positions; writing the compact rows in any selection order preserves
the per-character delta.
-/
namespace Allium.CompactDelta

/-- Mathematical population count of the selected bits below `character`. -/
def position (selected : Finset Nat) (character : Nat) : Nat :=
  (selected.filter (fun other => other < character)).card

/-- Character masks store exactly the selected low 32 bits. -/
def selectedMask (selected : Finset Nat) : BitVec 32 :=
  SimdMask.encode 32 (fun character => decide (character ∈ selected))

/-- The arithmetic mask `(1 << character) - 1`, before intersection. -/
def lowerMask (character : Nat) : BitVec 32 :=
  BitVec.ofNat 32 (2 ^ character - 1)

/-- Mathematical count_ones of a fixed-width mask. -/
def population (mask : BitVec 32) : Nat :=
  ((Finset.range 32).filter (fun bit => mask.getLsbD bit = true)).card

@[simp] theorem lower_mask_bit (character bit : Nat) :
    (lowerMask character).getLsbD bit = (decide (bit < 32) && decide (bit < character)) := by
  change ((2 ^ character - 1) % (2 ^ 32)).testBit bit = _
  simp only [Nat.testBit_mod_two_pow, Nat.testBit_two_pow_sub_one]

/-- The compact rank is exactly the mask/intersection/popcount expression,
not merely an abstract collision-free indexing function. -/
theorem position_eq_population (selected : Finset Nat) (character : Nat)
    (width : ∀ card ∈ selected, card < 32) :
    position selected character = population (selectedMask selected &&& lowerMask character) := by
  unfold position population
  congr 1
  ext bit
  simp only [Finset.mem_filter, Finset.mem_range, BitVec.getLsbD_and, selectedMask,
    SimdMask.encode_bit, lower_mask_bit, Bool.and_eq_true, decide_eq_true_eq]
  constructor
  · rintro ⟨hs, hl⟩
    exact ⟨width bit hs, ⟨width bit hs, hs⟩, width bit hs, hl⟩
  · rintro ⟨_, ⟨_, hs⟩, _, hl⟩
    exact ⟨hs, hl⟩

theorem position_lt (selected : Finset Nat) (a b : Nat)
    (ha : a ∈ selected) (hab : a < b) : position selected a < position selected b := by
  apply Finset.card_lt_card
  apply Finset.ssubset_iff_subset_ne.mpr
  constructor
  · intro c hc
    rcases Finset.mem_filter.mp hc with ⟨hm, hca⟩
    exact Finset.mem_filter.mpr ⟨hm, hca.trans hab⟩
  · intro he
    have hm : a ∈ selected.filter (fun other => other < b) := by simp [ha, hab]
    rw [← he] at hm
    simp at hm

theorem position_lt_card (selected : Finset Nat) (card : Nat) (member : card ∈ selected) :
    position selected card < selected.card := by
  apply Finset.card_lt_card
  apply Finset.ssubset_iff_subset_ne.mpr
  refine ⟨Finset.filter_subset _ _, ?_⟩
  intro he
  have hm : card ∈ selected.filter (fun other => other < card) := by rw [he]; exact member
  simp at hm

/-- The five-entry packed storage has room for every selected row. -/
theorem position_fits (rows : List Nat) (count card : Nat) (capacity : count ≤ 5)
    (member : card ∈ rows.take count) :
    position (rows.take count).toFinset card < 5 := by
  have hp := position_lt_card (rows.take count).toFinset card (List.mem_toFinset.mpr member)
  have hc := List.toFinset_card_le (rows.take count)
  have hl : (rows.take count).length ≤ count := by simp [List.length_take]
  omega

theorem position_injective (selected : Finset Nat) :
    Set.InjOn (position selected) selected := by
  intro a ha b hb he
  rcases lt_trichotomy a b with hab | hab | hab
  · have hlt := position_lt selected a b ha hab
    omega
  · exact hab
  · have hlt := position_lt selected b a hb hab
    omega

def writeRows (selected : Finset Nat) (delta : Nat → Nat) : List Nat → (Nat → Nat)
  | [] => fun _ => 0
  | character :: rest => Function.update (writeRows selected delta rest)
      (position selected character) (delta character)

theorem written_lookup (selected : Finset Nat) (delta : Nat → Nat) (order : List Nat)
    (inside : ∀ card ∈ order, card ∈ selected) (card : Nat) (member : card ∈ order) :
    writeRows selected delta order (position selected card) = delta card := by
  induction order with
  | nil => simp at member
  | cons first rest ih =>
      by_cases he : card = first
      · subst card
        simp [writeRows]
      · have hr : card ∈ rest := (List.mem_cons.mp member).resolve_left he
        have hne : position selected card ≠ position selected first := by
          intro hpos
          exact he (position_injective selected (inside card member)
            (inside first List.mem_cons_self) hpos)
        simp only [writeRows, Function.update_of_ne hne]
        exact ih (fun x hx => inside x (List.mem_cons_of_mem first hx)) hr

noncomputable def compact (selected : Finset Nat) (delta : Nat → Nat) : Nat → Nat :=
  writeRows selected delta selected.toList

noncomputable def lookup (selected : Finset Nat) (delta : Nat → Nat) (card : Nat) : Nat :=
  if card ∈ selected then compact selected delta (position selected card) else 0

theorem compact_lookup (selected : Finset Nat) (delta : Nat → Nat) (card : Nat) :
    lookup selected delta card = if card ∈ selected then delta card else 0 := by
  unfold lookup
  split
  next hm =>
    apply written_lookup
    · intro x hx
      exact Finset.mem_toList.mp hx
    · exact Finset.mem_toList.mpr hm
  next => rfl

/-- Connecting compact lookup, first-r selection, replacement, and the budget. -/
theorem compact_exclusion_exact (weight : Nat → Nat) (cards : List Nat)
    (used : Finset Nat) (count card : Nat) :
    let rows := SuffixDelta.prepared weight cards used
    let selected := (rows.take count).toFinset
    SuffixDelta.prefixSum weight (rows.erase card) count =
      SuffixDelta.prefixSum weight rows count -
        lookup selected (fun c => weight c - SuffixDelta.sample weight rows count) card := by
  dsimp only
  rw [compact_lookup, SuffixDelta.prepared_exclusion_exact]
  simp [SuffixDelta.exclusion]

theorem compact_used_update_exact (weight : Nat → Nat) (cards : List Nat)
    (used : Finset Nat) (count card : Nat) (unique : cards.Nodup) :
    let rows := SuffixDelta.prepared weight cards used
    let selected := (rows.take count).toFinset
    SuffixDelta.prefixSum weight (SuffixDelta.prepared weight cards (insert card used)) count =
      SuffixDelta.prefixSum weight rows count -
        lookup selected (fun c => weight c - SuffixDelta.sample weight rows count) card := by
  dsimp only
  rw [SuffixDelta.prepared_insert_eq_erase weight cards used card unique]
  exact compact_exclusion_exact weight cards used count card

/-- The source suffix summaries sort all 27 character slots, including zero
valued slots; no candidate list is truncated to form this character order. -/
def productionRows (weight : Nat → Nat) (used : Finset Nat) : List Nat :=
  SuffixDelta.prepared weight (List.range 27) used

noncomputable def productionDelta (weight : Nat → Nat) (used : Finset Nat) (count card : Nat) : Nat :=
  let rows := productionRows weight used
  lookup (rows.take count).toFinset (fun c => weight c - SuffixDelta.sample weight rows count) card

theorem production_delta_exact (weight : Nat → Nat) (used : Finset Nat) (count card : Nat) :
    SuffixDelta.prefixSum weight (productionRows weight (insert card used)) count =
      SuffixDelta.prefixSum weight (productionRows weight used) count -
        productionDelta weight used count card :=
  compact_used_update_exact weight (List.range 27) used count card List.nodup_range

theorem production_member_range (weight : Nat → Nat) (used : Finset Nat) (card : Nat)
    (member : card ∈ productionRows weight used) : card < 27 := by
  have ho : card ∈ SuffixDelta.ordered weight (List.range 27) := (List.mem_filter.mp member).1
  have hr := (List.mergeSort_perm (List.range 27)
    (fun a b => decide (weight b ≤ weight a))).mem_iff.mp ho
  exact List.mem_range.mp hr

/-- The complete population-count address is safe in the fixed five-row buffer. -/
theorem production_address (weight : Nat → Nat) (used : Finset Nat) (count card : Nat)
    (capacity : count ≤ 5) (member : card ∈ (productionRows weight used).take count) :
    let selected := ((productionRows weight used).take count).toFinset
    population (selectedMask selected &&& lowerMask card) = position selected card ∧
      position selected card < 5 := by
  dsimp only
  constructor
  · symm
    apply position_eq_population
    intro character hm
    have hc := List.mem_of_mem_take (List.mem_toFinset.mp hm)
    have hbound := production_member_range weight used character hc
    omega
  · exact position_fits _ count card capacity member

theorem production_budget_iff (weight : Nat → Nat) (used : Finset Nat) (count card : Nat)
    (selected candidate threshold : Nat) :
    selected + candidate + SuffixDelta.prefixSum weight (productionRows weight (insert card used)) count < threshold ↔
      selected + candidate + (SuffixDelta.prefixSum weight (productionRows weight used) count -
        productionDelta weight used count card) < threshold := by
  rw [production_delta_exact]

end Allium.CompactDelta
