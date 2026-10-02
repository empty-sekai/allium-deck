import Allium.FiniteBounds
import Mathlib.Data.List.Sort

/-!
# Sorted suffix exclusion deltas

The suffix scan retains its first r unused characters and one replacement.
Excluding a retained character subtracts its value minus the replacement;
excluding any other character leaves the prefix unchanged. Natural subtraction
is justified by the descending order, not used as a saturation heuristic.
-/
namespace Allium.SuffixDelta

variable {Card : Type*}

def sample (weight : Card → Nat) (cards : List Card) (index : Nat) : Nat :=
  (cards[index]?.map weight).getD 0

def prefixSum (weight : Card → Nat) (cards : List Card) (count : Nat) : Nat :=
  ((cards.take count).map weight).sum

def Descending (weight : Card → Nat) (cards : List Card) : Prop :=
  Antitone (sample weight cards)

@[simp] theorem at_cons_zero (weight : Card → Nat) (card : Card) (rest : List Card) :
    sample weight (card :: rest) 0 = weight card := rfl

@[simp] theorem at_cons_succ (weight : Card → Nat) (card : Card) (rest : List Card) (n : Nat) :
    sample weight (card :: rest) (n + 1) = sample weight rest n := rfl

@[simp] theorem prefix_zero (weight : Card → Nat) (cards : List Card) :
    prefixSum weight cards 0 = 0 := by simp [prefixSum]

@[simp] theorem prefix_nil (weight : Card → Nat) (n : Nat) :
    prefixSum weight [] n = 0 := by simp [prefixSum]

@[simp] theorem prefix_cons (weight : Card → Nat) (card : Card) (rest : List Card) (n : Nat) :
    prefixSum weight (card :: rest) (n + 1) = weight card + prefixSum weight rest n := by
  simp [prefixSum]

theorem prefix_succ (weight : Card → Nat) (cards : List Card) (n : Nat) :
    prefixSum weight cards (n + 1) = prefixSum weight cards n + sample weight cards n := by
  induction cards generalizing n with
  | nil => simp [sample]
  | cons card rest ih =>
      cases n with
      | zero => simp
      | succ n => simp only [prefix_cons, at_cons_succ, ih]; omega

theorem descending_tail (weight : Card → Nat) (card : Card) (rest : List Card)
    (h : Descending weight (card :: rest)) : Descending weight rest := by
  intro i j hij
  exact h (Nat.add_le_add_right hij 1)

theorem replacement_le (weight : Card → Nat) (cards : List Card) (n : Nat) (card : Card)
    (sorted : Descending weight cards) (member : card ∈ cards.take n) :
    sample weight cards n ≤ weight card := by
  induction cards generalizing n with
  | nil => simp at member
  | cons first rest ih =>
      cases n with
      | zero => simp at member
      | succ n =>
          simp only [List.take_succ_cons, List.mem_cons] at member
          rcases member with he | hm
          · subst card
            exact sorted (Nat.zero_le (n + 1))
          · exact ih n (descending_tail weight first rest sorted) hm

variable [DecidableEq Card]

theorem prefix_erase_absent (weight : Card → Nat) (cards : List Card) (n : Nat) (card : Card)
    (absent : card ∉ cards.take n) :
    prefixSum weight (cards.erase card) n = prefixSum weight cards n := by
  induction cards generalizing n with
  | nil => simp
  | cons first rest ih =>
      cases n with
      | zero => simp
      | succ n =>
          simp only [List.take_succ_cons, List.mem_cons, not_or] at absent
          rw [List.erase_cons_tail (by simpa only [beq_iff_eq] using Ne.symm absent.1)]
          simp only [prefix_cons]
          rw [ih n absent.2]

/-- An exact accounting identity before making any subtraction. -/
theorem prefix_erase_accounting (weight : Card → Nat) (cards : List Card) (n : Nat) (card : Card)
    (member : card ∈ cards.take n) :
    prefixSum weight cards n + sample weight cards n =
      prefixSum weight (cards.erase card) n + weight card := by
  induction cards generalizing n with
  | nil => simp at member
  | cons first rest ih =>
      cases n with
      | zero => simp at member
      | succ n =>
          by_cases he : card = first
          · subst card
            simp only [List.erase_cons_head, prefix_cons, at_cons_succ]
            rw [prefix_succ]
            omega
          · simp only [List.take_succ_cons, List.mem_cons, he, false_or] at member
            rw [List.erase_cons_tail (by simpa only [beq_iff_eq] using Ne.symm he)]
            simp only [prefix_cons, at_cons_succ]
            have h := ih n member
            omega

def exclusion (weight : Card → Nat) (cards : List Card) (count : Nat) (card : Card) : Nat :=
  if card ∈ cards.take count then weight card - sample weight cards count else 0

/-- This is the delta read by both power and skill/bonus suffix summaries. -/
theorem exclusion_exact (weight : Card → Nat) (cards : List Card) (count : Nat) (card : Card)
    (sorted : Descending weight cards) :
    prefixSum weight (cards.erase card) count =
      prefixSum weight cards count - exclusion weight cards count card := by
  unfold exclusion
  by_cases hm : card ∈ cards.take count
  · rw [if_pos hm]
    have hr := replacement_le weight cards count card sorted hm
    have ha := prefix_erase_accounting weight cards count card hm
    omega
  · rw [if_neg hm, Nat.sub_zero]
    exact prefix_erase_absent weight cards count card hm

/-- Delta is charged before comparing a candidate-specific suffix budget. -/
theorem delta_budget_iff (weight : Card → Nat) (cards : List Card) (count : Nat)
    (card : Card) (selected candidate threshold : Nat) (sorted : Descending weight cards) :
    selected + candidate + prefixSum weight (cards.erase card) count < threshold ↔
      selected + candidate + (prefixSum weight cards count - exclusion weight cards count card) < threshold := by
  rw [exclusion_exact weight cards count card sorted]

omit [DecidableEq Card] in
theorem descending_of_pairwise (weight : Card → Nat) (cards : List Card)
    (sorted : cards.Pairwise (fun a b => weight b ≤ weight a)) : Descending weight cards := by
  induction cards with
  | nil => intro i j _; simp [sample]
  | cons first rest ih =>
      rcases List.pairwise_cons.mp sorted with ⟨hfirst, hrest⟩
      intro i j hij
      cases i with
      | zero =>
          cases j with
          | zero => exact Nat.le_refl _
          | succ j =>
              simp only [at_cons_zero, at_cons_succ]
              cases hv : rest[j]? with
              | none => simp [sample, hv]
              | some value =>
                  simpa only [sample, hv, Option.map_some, Option.getD_some] using
                    hfirst value (List.mem_of_getElem? hv)
      | succ i =>
          cases j with
          | zero => omega
          | succ j => exact ih hrest (Nat.le_of_succ_le_succ hij)

def ordered (weight : Card → Nat) (cards : List Card) : List Card :=
  cards.mergeSort (fun a b => decide (weight b ≤ weight a))

omit [DecidableEq Card] in
theorem ordered_pairwise (weight : Card → Nat) (cards : List Card) :
    (ordered weight cards).Pairwise (fun a b => weight b ≤ weight a) := by
  simpa only [ordered, decide_eq_true_eq] using
    List.sorted_mergeSort (le := fun a b => decide (weight b ≤ weight a))
      (by
        intro a b c hab hbc
        simp only [decide_eq_true_eq] at hab hbc ⊢
        exact hbc.trans hab)
      (by
        intro a b
        simpa using (le_total (weight b) (weight a)))
      cards

def prepared (weight : Card → Nat) (cards : List Card) (used : Finset Card) : List Card :=
  (ordered weight cards).filter (fun card => decide (card ∉ used))

theorem prepared_descending (weight : Card → Nat) (cards : List Card) (used : Finset Card) :
    Descending weight (prepared weight cards used) := by
  apply descending_of_pairwise
  exact (ordered_pairwise weight cards).filter _

/-- The sorted-and-filtered production scan discharges the order premise. -/
theorem prepared_exclusion_exact (weight : Card → Nat) (cards : List Card)
    (used : Finset Card) (count : Nat) (card : Card) :
    prefixSum weight ((prepared weight cards used).erase card) count =
      prefixSum weight (prepared weight cards used) count -
        exclusion weight (prepared weight cards used) count card :=
  exclusion_exact weight _ count card (prepared_descending weight cards used)

theorem prepared_nodup (weight : Card → Nat) (cards : List Card) (used : Finset Card)
    (unique : cards.Nodup) : (prepared weight cards used).Nodup := by
  exact ((List.mergeSort_perm cards (fun a b => decide (weight b ≤ weight a))).nodup_iff.mpr unique).filter _

theorem filter_ne_eq_erase (cards : List Card) (card : Card) (unique : cards.Nodup) :
    cards.filter (fun other => decide (other ≠ card)) = cards.erase card := by
  induction cards with
  | nil => simp
  | cons first rest ih =>
      have hn := List.nodup_cons.mp unique
      by_cases he : first = card
      · subst first
        have hall : ∀ other ∈ rest, decide (other ≠ card) = true := by
          intro other hm
          simp only [decide_eq_true_eq]
          intro heq
          subst other
          exact hn.1 hm
        simp only [List.filter_cons, ne_eq, not_true_eq_false, decide_false, Bool.false_eq_true,
          ↓reduceIte, List.erase_cons_head]
        exact List.filter_eq_self.mpr hall
      · simp only [List.filter_cons, he, ne_eq, not_false_eq_true, decide_true, ↓reduceIte,
          List.erase_cons_tail (a := card) (b := first) (l := rest)
            (by simpa only [beq_iff_eq] using he)]
        rw [ih hn.2]

theorem prepared_insert_eq_erase (weight : Card → Nat) (cards : List Card)
    (used : Finset Card) (card : Card) (unique : cards.Nodup) :
    prepared weight cards (insert card used) = (prepared weight cards used).erase card := by
  rw [← filter_ne_eq_erase _ card (prepared_nodup weight cards used unique)]
  simp only [prepared, List.filter_filter]
  apply List.filter_congr
  intro other _
  simp [Finset.mem_insert]

/-- Candidate exclusion updates the used-character set of the same sorted
scan, rather than deleting an arbitrary entry from an unrelated relaxation. -/
theorem used_update_exact (weight : Card → Nat) (cards : List Card)
    (used : Finset Card) (count : Nat) (card : Card) (unique : cards.Nodup) :
    prefixSum weight (prepared weight cards (insert card used)) count =
      prefixSum weight (prepared weight cards used) count -
        exclusion weight (prepared weight cards used) count card := by
  rw [prepared_insert_eq_erase weight cards used card unique]
  exact prepared_exclusion_exact weight cards used count card

end Allium.SuffixDelta
