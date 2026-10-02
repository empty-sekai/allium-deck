import Allium.SuffixDelta

/-!
# Refill position monotonicity

After the counted support prefix, whole-tick losses are non-negative and
non-decreasing. Holding more tail positions can only move each refill choice
to the right, provided the tail contains enough unheld positions. The explicit
capacity condition is essential: a truncated exhausted tail is not a valid
monotonicity premise.
-/
namespace Allium.Refill
open SuffixDelta

/-- A zero-padded whole-tick array read; valid indices retain exact values. -/
theorem sample_leading_lower (first : Nat) (rest : List Nat) (index : Nat)
    (ordered : ∀ value ∈ rest, first ≤ value) (inside : index < rest.length) :
    first ≤ sample id rest index := by
  have he : rest[index]? = some rest[index] := List.getElem?_eq_getElem inside
  have hm : rest[index] ∈ rest := List.getElem_mem inside
  simpa only [sample, he, Option.map_some, Option.getD_some, id_eq] using ordered rest[index] hm

/-- Removing an earlier no-larger value raises the full prefix sum. -/
theorem drop_first_prefix (first : Nat) (rest : List Nat) (count : Nat)
    (ordered : ∀ value ∈ rest, first ≤ value) (enough : count ≤ rest.length) :
    prefixSum id (first :: rest) count ≤ prefixSum id rest count := by
  cases count with
  | zero => simp
  | succ count =>
      rw [prefix_cons, prefix_succ]
      simp only [id_eq]
      have hl := sample_leading_lower first rest count ordered (by omega)
      omega

/-- For a non-decreasing sequence, dropping positions raises rankwise prefix
sums whenever the shorter sequence still fills the requested prefix. -/
theorem prefix_sublist (long short : List Nat) (ordered : long.Pairwise (· ≤ ·))
    (sub : short.Sublist long) (count : Nat) (enough : count ≤ short.length) :
    prefixSum id long count ≤ prefixSum id short count := by
  induction sub generalizing count with
  | slnil => simp
  | @cons short long value sub ih =>
      have hp := List.pairwise_cons.mp ordered
      have hcount : count ≤ long.length := enough.trans sub.length_le
      exact (drop_first_prefix value long count hp.1 hcount).trans (ih hp.2 count enough)
  | @cons₂ short long value sub ih =>
      cases count with
      | zero => simp
      | succ count =>
          simp only [prefix_cons]
          exact Nat.add_le_add_left (ih (List.pairwise_cons.mp ordered).2 count (by simpa using enough)) value

/-- Increasing the number of refilled entries is monotone even without order. -/
theorem prefix_count_mono (values : List Nat) {small large : Nat} (h : small ≤ large) :
    prefixSum id values small ≤ prefixSum id values large := by
  induction large with
  | zero =>
      have hs : small = 0 := by omega
      subst small
      exact Nat.le_refl _
  | succ large ih =>
      by_cases he : small = large + 1
      · subst small; exact Nat.le_refl _
      · have hs : small ≤ large := by omega
        rw [prefix_succ]
        exact (ih hs).trans (Nat.le_add_right _ _)

/-- A bitmask records only positions below 64; positions beyond it are unheld. -/
def available (length : Nat) (held : Finset Nat) : List Nat :=
  (List.range length).filter (fun index => decide (index ∉ held))

def refill (steps : Nat → Nat) (length removed : Nat) (held : Finset Nat) : Nat :=
  prefixSum id ((available length held).map steps) removed

theorem available_mono (length : Nat) (old fresh : Finset Nat) (h : old ⊆ fresh) :
    (available length fresh).Sublist (available length old) := by
  unfold available
  generalize List.range length = entries
  induction entries with
  | nil => simp
  | cons index rest ih =>
      by_cases hf : index ∈ fresh
      · by_cases ho : index ∈ old
        · simpa [hf, ho] using ih
        · simpa [hf, ho] using List.Sublist.cons index ih
      · have ho : index ∉ old := fun member => hf (h member)
        simpa [hf, ho] using List.Sublist.cons₂ index ih

/-- Actual held-bit truncation is a subset of the complete held-position set. -/
def heldMask (held : Finset Nat) : Finset Nat := held.filter (fun position => position < 64)

theorem held_mask_subset (held : Finset Nat) : heldMask held ⊆ held := Finset.filter_subset _ _

theorem refill_mono (steps : Nat → Nat) (length : Nat) (old fresh : Finset Nat)
    (removed completeRemoved : Nat) (ordered : Monotone steps)
    (moreHeld : old ⊆ fresh) (moreRemoved : removed ≤ completeRemoved)
    (enough : completeRemoved ≤ (available length fresh).length) :
    refill steps length removed old ≤ refill steps length completeRemoved fresh := by
  have hsorted : ((available length old).map steps).Pairwise (· ≤ ·) := by
    exact List.Pairwise.map steps (fun a b hab => ordered (Nat.le_of_lt hab))
      ((List.pairwise_lt_range (n := length)).filter _)
  have hsub := (available_mono length old fresh moreHeld).map steps
  have hfirst := prefix_sublist ((available length old).map steps)
    ((available length fresh).map steps) hsorted hsub removed (by simpa using moreRemoved.trans enough)
  exact hfirst.trans (prefix_count_mono _ moreRemoved)

/-- Ignoring held bits beyond the 64-bit mask is a lower relaxation, never an
exact refill claim unless every held position is represented. -/
theorem held_mask_lower (steps : Nat → Nat) (length removed : Nat) (held : Finset Nat)
    (ordered : Monotone steps) (enough : removed ≤ (available length held).length) :
    refill steps length removed (heldMask held) ≤ refill steps length removed held :=
  refill_mono steps length _ _ removed removed ordered (held_mask_subset held) (Nat.le_refl _) enough

/-- Whole-tick losses are built from the first uncounted entry minus each
later entry. Descending support values therefore give non-decreasing losses. -/
def lossSteps (entries : List Nat) (counted : Nat) (index : Nat) : Nat :=
  sample id entries counted - sample id entries (counted + index)

theorem loss_steps_monotone (entries : List Nat) (counted : Nat)
    (ordered : Descending id entries) : Monotone (lossSteps entries counted) := by
  intro i j hij
  exact Nat.sub_le_sub_left (ordered (Nat.add_le_add_left hij counted)) _

theorem constructed_refill_mono (entries : List Nat) (counted length : Nat)
    (old fresh : Finset Nat) (removed completeRemoved : Nat)
    (ordered : Descending id entries) (moreHeld : old ⊆ fresh)
    (moreRemoved : removed ≤ completeRemoved)
    (enough : completeRemoved ≤ (available length fresh).length) :
    refill (lossSteps entries counted) length removed old ≤
      refill (lossSteps entries counted) length completeRemoved fresh :=
  refill_mono _ length old fresh removed completeRemoved
    (loss_steps_monotone entries counted ordered) moreHeld moreRemoved enough

/-- Removing held positions loses no more positions than the held set size. -/
theorem available_length_bound (length : Nat) (held : Finset Nat) :
    length ≤ (available length held).length + held.card := by
  have hset : (available length held).toFinset = Finset.range length \ held := by
    ext index
    simp [available]
  have hn : (available length held).Nodup := (List.nodup_range (n := length)).filter _
  have hcard := List.toFinset_card_of_nodup hn
  rw [hset] at hcard
  have hsplit := Finset.card_sdiff_add_card_inter (Finset.range length) held
  have hcap : (Finset.range length ∩ held).card ≤ held.card :=
    Finset.card_le_card Finset.inter_subset_right
  simp only [Finset.card_range] at hsplit
  omega

/-- Two times the removable-entry budget fills every legal refill, including
entries skipped by the held-position mask. -/
theorem twice_budget_enough (removable removed : Nat) (held : Finset Nat)
    (hremoved : removed ≤ removable) (hheld : held.card ≤ removable) :
    removed ≤ (available (2 * removable) held).length := by
  have h := available_length_bound (2 * removable) held
  omega

theorem budgeted_refill_mono (entries : List Nat) (counted removable : Nat)
    (old fresh : Finset Nat) (removed completeRemoved : Nat)
    (ordered : Descending id entries) (moreHeld : old ⊆ fresh)
    (moreRemoved : removed ≤ completeRemoved) (removedBound : completeRemoved ≤ removable)
    (heldBound : fresh.card ≤ removable) :
    refill (lossSteps entries counted) (2 * removable) removed old ≤
      refill (lossSteps entries counted) (2 * removable) completeRemoved fresh :=
  constructed_refill_mono entries counted (2 * removable) old fresh removed completeRemoved
    ordered moreHeld moreRemoved (twice_budget_enough removable completeRemoved fresh removedBound heldBound)

end Allium.Refill
