import Allium.CompactDelta
import Allium.MixedPower

/-!
# Sorted suffix scans and per-character maxima

The executable scan sorts the 27 character slots, removes already-used
characters, and sums its first r entries. This module connects that scan to
finite subset optimization and to maxima of actual dense suffix cards.
-/
namespace Allium.SuffixScan
open SuffixDelta FiniteBounds

variable {Id : Type*} [DecidableEq Id]

theorem sum_toFinset (weight : Id → Nat) (cards : List Id) (unique : cards.Nodup) :
    (∑ card ∈ cards.toFinset, weight card) = (cards.map weight).sum := by
  induction cards with
  | nil => simp
  | cons first rest ih =>
      have hn := List.nodup_cons.mp unique
      simp only [List.toFinset_cons, List.map_cons, List.sum_cons]
      rw [Finset.sum_insert (by simpa using hn.1), ih hn.2]

/-- Exchange with the first sorted entry proves the actual prefix is optimal;
no premise asserts that the desired sum is already below a supplied bound. -/
theorem prefix_dominates (weight : Id → Nat) (cards : List Id)
    (ordered : cards.Pairwise (fun a b => weight b ≤ weight a))
    (unique : cards.Nodup) (count : Nat) (picks : Finset Id)
    (inside : picks ⊆ cards.toFinset) (capacity : picks.card ≤ count) :
    (∑ card ∈ picks, weight card) ≤ prefixSum weight cards count := by
  induction cards generalizing count picks with
  | nil =>
      have he : picks = ∅ := Finset.Subset.antisymm inside (Finset.empty_subset picks)
      simp [he]
  | cons first rest ih =>
      have hn := List.nodup_cons.mp unique
      have ho := List.pairwise_cons.mp ordered
      cases count with
      | zero =>
          have he : picks = ∅ := Finset.card_eq_zero.mp (by omega)
          simp [he]
      | succ count =>
          rw [prefix_cons]
          by_cases hf : first ∈ picks
          · have sub : picks.erase first ⊆ rest.toFinset := by
              intro card hc
              have hp := Finset.mem_erase.mp hc
              have hm := inside hp.2
              simp only [List.toFinset_cons, Finset.mem_insert] at hm
              rcases hm with he | ht
              · exact False.elim (hp.1 he)
              · exact ht
            have hs : (picks.erase first).card ≤ count := by
              have hc := Finset.card_erase_add_one hf
              omega
            have hsum : (∑ card ∈ picks.erase first, weight card) + weight first =
                ∑ card ∈ picks, weight card := Finset.sum_erase_add _ _ hf
            have hb := ih ho.2 hn.2 count (picks.erase first) sub hs
            omega
          · have sub : picks ⊆ rest.toFinset := by
              intro card hc
              have hm := inside hc
              simp only [List.toFinset_cons, Finset.mem_insert] at hm
              rcases hm with he | ht
              · subst card
                exact False.elim (hf hc)
              · exact ht
            by_cases hs : picks.card ≤ count
            · have hb := ih ho.2 hn.2 count picks sub hs
              omega
            · have nonempty : picks.Nonempty := Finset.card_pos.mp (by omega)
              obtain ⟨last, hl⟩ := nonempty
              have subErase : picks.erase last ⊆ rest.toFinset :=
                (Finset.erase_subset _ _).trans sub
              have hsErase : (picks.erase last).card ≤ count := by
                have hc := Finset.card_erase_add_one hl
                omega
              have hsum : (∑ card ∈ picks.erase last, weight card) + weight last =
                  ∑ card ∈ picks, weight card := Finset.sum_erase_add _ _ hl
              have hb := ih ho.2 hn.2 count (picks.erase last) subErase hsErase
              have hw := ho.1 last (List.mem_toFinset.mp (sub hl))
              omega

/-- The list scan equals the finite top-r subset maximum, including short
pools and zero-valued padding. -/
theorem prefix_eq_maxSum (weight : Id → Nat) (cards : List Id)
    (ordered : cards.Pairwise (fun a b => weight b ≤ weight a))
    (unique : cards.Nodup) (count : Nat) :
    prefixSum weight cards count = maxSum count cards.toFinset weight := by
  apply Nat.le_antisymm
  · have hn : (cards.take count).Nodup := unique.take
    have he := sum_toFinset weight (cards.take count) hn
    change ((cards.take count).map weight).sum ≤ _
    rw [← he]
    apply sum_le_maxSum
    · intro card hc
      exact List.mem_toFinset.mpr (List.mem_of_mem_take (List.mem_toFinset.mp hc))
    · have hcard := List.toFinset_card_le (cards.take count)
      have hlen : (cards.take count).length ≤ count := by simp
      omega
  · apply Finset.sup_le
    intro picks hp
    rcases (mem_selections _ _ _).mp hp with ⟨inside, capacity⟩
    exact prefix_dominates weight cards ordered unique count picks inside capacity

@[simp] theorem production_mem (weight : Nat → Nat) (used : Finset Nat) (character : Nat) :
    character ∈ CompactDelta.productionRows weight used ↔ character < 27 ∧ character ∉ used := by
  simp only [CompactDelta.productionRows, prepared, List.mem_filter, decide_eq_true_eq]
  have hp : character ∈ ordered weight (List.range 27) ↔ character ∈ List.range 27 :=
    (List.mergeSort_perm (List.range 27) (fun a b => decide (weight b ≤ weight a))).mem_iff
  rw [hp, List.mem_range]

theorem production_prefix_exact (weight : Nat → Nat) (used : Finset Nat) (count : Nat) :
    prefixSum weight (CompactDelta.productionRows weight used) count =
      maxSum count (Finset.range 27 \ used) weight := by
  have hordered := (ordered_pairwise weight (List.range 27)).filter
    (fun card => decide (card ∉ used))
  have hunique := prepared_nodup weight (List.range 27) used List.nodup_range
  have he := prefix_eq_maxSum weight (CompactDelta.productionRows weight used) hordered hunique count
  have hset : (CompactDelta.productionRows weight used).toFinset = Finset.range 27 \ used := by
    ext card
    simp
  simpa [hset] using he

def densePool (cards : List Id) (start : Nat) : Finset Id := (cards.drop start).toFinset

def characterValues (pool : Finset Id) (character : Id → Fin 27) (weight : Id → Nat) : Nat → Nat :=
  characterMax pool (fun card => (character card).val) weight

def scanBound (cards : List Id) (start : Nat) (used : Finset Nat) (slots : Nat)
    (character : Id → Fin 27) (weight : Id → Nat) : Nat :=
  let values := characterValues (densePool cards start) character weight
  prefixSum values (CompactDelta.productionRows values used) slots

/-- Every unused unique-character completion is bounded by the actual sorted
character array built from maxima of this dense suffix. -/
theorem scan_bound_sound (cards : List Id) (start : Nat) (used : Finset Nat) (slots : Nat)
    (character : Id → Fin 27) (weight : Id → Nat) (picks : Finset Id)
    (inside : picks ⊆ densePool cards start) (capacity : picks.card ≤ slots)
    (unique : Set.InjOn character picks)
    (unused : ∀ card ∈ picks, (character card).val ∉ used) :
    (∑ card ∈ picks, weight card) ≤ scanBound cards start used slots character weight := by
  let values := characterValues (densePool cards start) character weight
  let labels := picks.image (fun card => (character card).val)
  have hinj : Set.InjOn (fun card => (character card).val) picks := by
    intro a ha b hb he
    exact unique ha hb (Fin.ext he)
  have hs : labels ⊆ Finset.range 27 \ used := by
    intro label hl
    obtain ⟨card, hc, he⟩ := Finset.mem_image.mp hl
    subst label
    exact Finset.mem_sdiff.mpr ⟨Finset.mem_range.mpr (character card).isLt, unused card hc⟩
  have hc : labels.card ≤ slots := Finset.card_image_le.trans capacity
  change (∑ card ∈ picks, weight card) ≤ prefixSum values (CompactDelta.productionRows values used) slots
  rw [production_prefix_exact]
  calc
    (∑ card ∈ picks, weight card) ≤ ∑ card ∈ picks, values (character card).val :=
      Finset.sum_le_sum (fun card hp => le_characterMax _ _ _ card (inside hp))
    _ = ∑ label ∈ labels, values label := by
      rw [Finset.sum_image]
      exact hinj
    _ ≤ maxSum slots (Finset.range 27 \ used) values := sum_le_maxSum _ _ _ _ hs hc

/-- Applied to the effective-power selector, the suffix proof does not assume
that mixed-unit, shared-unit, or shared-attribute bonuses are monotone. -/
theorem effective_scan_bound (mode : MixedPower.Mode) (data : Id → MixedPower.Card)
    (deck cards : List Id) (start : Nat) (used : Finset Nat) (slots : Nat)
    (character : Id → Fin 27) (picks : Finset Id)
    (inside : picks ⊆ densePool cards start) (capacity : picks.card ≤ slots)
    (unique : Set.InjOn character picks)
    (unused : ∀ card ∈ picks, (character card).val ∉ used) :
    (∑ card ∈ picks, MixedPower.cardPower mode data deck card) ≤
      scanBound cards start used slots character (fun card => MixedPower.maximum (data card)) := by
  apply le_trans (Finset.sum_le_sum (fun card _ => MixedPower.effective_upper mode _ _ _ (data card)))
  exact scan_bound_sound cards start used slots character _ picks inside capacity unique unused

theorem dense_pool_mono (cards : List Id) {start stop : Nat} (order : start ≤ stop) :
    densePool cards stop ⊆ densePool cards start := by
  have he : (cards.drop start).drop (stop - start) = cards.drop stop := by
    rw [List.drop_drop]
    congr 1
    omega
  intro card hc
  have hm : card ∈ cards.drop stop := List.mem_toFinset.mp hc
  rw [← he] at hm
  exact List.mem_toFinset.mpr (List.mem_of_mem_drop hm)

omit [DecidableEq Id] in
theorem character_values_mono (character : Id → Fin 27) (weight : Id → Nat)
    {small large : Finset Id} (inside : small ⊆ large) (label : Nat) :
    characterValues small character weight label ≤ characterValues large character weight label := by
  apply Finset.sup_le
  intro card hc
  rcases Finset.mem_filter.mp hc with ⟨hm, he⟩
  exact Finset.le_sup (Finset.mem_filter.mpr ⟨inside hm, he⟩)

/-- Every later dense start has a nested candidate pool; sorting its summary
does not break the direction required for a loop-wide break. -/
theorem scan_bound_antitone (cards : List Id) (used : Finset Nat) (slots : Nat)
    (character : Id → Fin 27) (weight : Id → Nat) :
    Antitone (fun start => scanBound cards start used slots character weight) := by
  intro start stop order
  dsimp only [scanBound]
  rw [production_prefix_exact, production_prefix_exact]
  apply maxSum_mono_weight
  intro label _
  exact character_values_mono character weight (dense_pool_mono cards order) label

theorem used_bound_antitone (cards : List Id) (start slots : Nat)
    (character : Id → Fin 27) (weight : Id → Nat) {old fresh : Finset Nat} (held : old ⊆ fresh) :
    scanBound cards start fresh slots character weight ≤ scanBound cards start old slots character weight := by
  unfold scanBound
  rw [production_prefix_exact, production_prefix_exact]
  apply maxSum_mono_pool
  intro label hl
  rcases Finset.mem_sdiff.mp hl with ⟨hr, hf⟩
  exact Finset.mem_sdiff.mpr ⟨hr, fun ho => hf (held ho)⟩

theorem dense_effective_break (mode : MixedPower.Mode) (data : Id → MixedPower.Card)
    (deck cards : List Id) (start stop : Nat) (used : Finset Nat) (slots threshold : Nat)
    (character : Id → Fin 27) (picks : Finset Id)
    (order : start ≤ stop) (inside : picks ⊆ densePool cards stop) (capacity : picks.card ≤ slots)
    (unique : Set.InjOn character picks) (unused : ∀ card ∈ picks, (character card).val ∉ used)
    (cut : scanBound cards start used slots character (fun card => MixedPower.maximum (data card)) < threshold) :
    (∑ card ∈ picks, MixedPower.cardPower mode data deck card) < threshold := by
  have hb := effective_scan_bound mode data deck cards stop used slots character picks
    inside capacity unique unused
  exact lt_of_le_of_lt (hb.trans (scan_bound_antitone cards used slots character _ order)) cut

end Allium.SuffixScan
