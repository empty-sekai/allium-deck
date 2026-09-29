import Allium.FiniteBounds
import Allium.Arithmetic
import Mathlib.Data.Finset.Max

/-!
# Support-pool relaxation and cutoff algebra

The support objective is the largest sum of at most W eligible public IDs.
Non-negative support weights make unused slots equivalent to zero padding.
This module proves prefix-exclusion monotonicity, a sorted-prefix optimality
certificate, and the compensated replacement inequality. The concrete
q+5-rank cutoff certificate and binary64 error budget remain separate duties.

Source: evaluate.rs::calc_support_bonus, suffix.rs, dominance.rs, bonus_tiers.rs.
-/
namespace Allium.Support

variable {Id : Type*}

/-- Partition a support sum without assuming rounded subtraction is exact. -/
theorem sum_split [DecidableEq Id] (s t : Finset Id) (weight : Id → ℝ) :
    (∑ id ∈ s \ t, weight id) + (∑ id ∈ s ∩ t, weight id) = ∑ id ∈ s, weight id := by
  have hd : Disjoint (s \ t) (s ∩ t) := by
    apply Finset.disjoint_left.mpr
    intro id hs ht
    exact (Finset.mem_sdiff.mp hs).2 (Finset.mem_inter.mp ht).2
  have hu : (s \ t) ∪ (s ∩ t) = s := by
    ext id
    simp only [Finset.mem_union, Finset.mem_sdiff, Finset.mem_inter]
    tauto
  calc
    (∑ id ∈ s \ t, weight id) + (∑ id ∈ s ∩ t, weight id) =
        ∑ id ∈ (s \ t) ∪ (s ∩ t), weight id := (Finset.sum_union hd).symm
    _ = ∑ id ∈ s, weight id := by rw [hu]

noncomputable def totals (slots : ℕ) (eligible : Finset Id) (weight : Id → ℝ) : Finset ℝ :=
  (FiniteBounds.selections slots eligible).image (fun picks => ∑ id ∈ picks, weight id)

theorem totals_nonempty (slots : ℕ) (eligible : Finset Id) (weight : Id → ℝ) :
    (totals slots eligible weight).Nonempty := by
  refine ⟨0, Finset.mem_image.mpr ⟨∅, ?_, by simp⟩⟩
  exact (FiniteBounds.mem_selections _ _ _).mpr ⟨Finset.empty_subset _, by simp⟩

noncomputable def bestSum (slots : ℕ) (eligible : Finset Id) (weight : Id → ℝ) : ℝ :=
  (totals slots eligible weight).max' (totals_nonempty slots eligible weight)

theorem sum_le_bestSum (slots : ℕ) (eligible picks : Finset Id) (weight : Id → ℝ)
    (hsub : picks ⊆ eligible) (hsize : picks.card ≤ slots) :
    (∑ id ∈ picks, weight id) ≤ bestSum slots eligible weight := by
  apply Finset.le_max'
  exact Finset.mem_image.mpr
    ⟨picks, (FiniteBounds.mem_selections _ _ _).mpr ⟨hsub, hsize⟩, rfl⟩

theorem bestSum_attained (slots : ℕ) (eligible : Finset Id) (weight : Id → ℝ) :
    ∃ picks ⊆ eligible, picks.card ≤ slots ∧ (∑ id ∈ picks, weight id) = bestSum slots eligible weight := by
  have h := Finset.max'_mem (totals slots eligible weight) (totals_nonempty slots eligible weight)
  rcases Finset.mem_image.mp h with ⟨picks, hp, heq⟩
  rcases (FiniteBounds.mem_selections _ _ _).mp hp with ⟨hsub, hsize⟩
  exact ⟨picks, hsub, hsize, heq⟩

theorem bestSum_nonneg (slots : ℕ) (eligible : Finset Id) (weight : Id → ℝ) :
    0 ≤ bestSum slots eligible weight := by
  simpa using sum_le_bestSum slots eligible ∅ weight (Finset.empty_subset _) (by simp)

theorem bestSum_mono_pool (slots : ℕ) (weight : Id → ℝ)
    {small large : Finset Id} (h : small ⊆ large) :
    bestSum slots small weight ≤ bestSum slots large weight := by
  rcases bestSum_attained slots small weight with ⟨picks, hp, hs, heq⟩
  rw [← heq]
  exact sum_le_bestSum slots large picks weight (hp.trans h) hs

theorem bestSum_mono_weight (slots : ℕ) (eligible : Finset Id) (a b : Id → ℝ)
    (h : ∀ id ∈ eligible, a id ≤ b id) : bestSum slots eligible a ≤ bestSum slots eligible b := by
  rcases bestSum_attained slots eligible a with ⟨picks, hp, hs, heq⟩
  rw [← heq]
  exact (Finset.sum_le_sum (fun id hi => h id (hp hi))).trans
    (sum_le_bestSum slots eligible picks b hp hs)

theorem bestSum_mono_slots (eligible : Finset Id) (weight : Id → ℝ)
    {small large : ℕ} (h : small ≤ large) :
    bestSum small eligible weight ≤ bestSum large eligible weight := by
  rcases bestSum_attained small eligible weight with ⟨picks, hp, hs, heq⟩
  rw [← heq]
  exact sum_le_bestSum large eligible picks weight hp (hs.trans h)

/-- Excluding more main-deck public IDs never increases remaining support. -/
theorem prefix_exclusion [DecidableEq Id] (pool : Finset Id) (weight : Id → ℝ) (slots : ℕ)
    {selected complete : Finset Id} (h : selected ⊆ complete) :
    bestSum slots (pool \ complete) weight ≤ bestSum slots (pool \ selected) weight := by
  apply bestSum_mono_pool
  intro id hi
  rcases Finset.mem_sdiff.mp hi with ⟨hp, hn⟩
  exact Finset.mem_sdiff.mpr ⟨hp, fun hs => hn (h hs)⟩

/-- A per-ID maximum over leader profiles, with a maximum slot count, is an
upper bound even when the selected maxima come from incompatible profiles. -/
theorem leader_profile_envelope (pool : Finset Id) (weight envelope : Id → ℝ)
    (slots maxSlots : ℕ) (hs : slots ≤ maxSlots) (hw : ∀ id ∈ pool, weight id ≤ envelope id) :
    bestSum slots pool weight ≤ bestSum maxSlots pool envelope :=
  (bestSum_mono_weight slots pool weight envelope hw).trans (bestSum_mono_slots pool envelope hs)

/-- Threshold form of a top-r certificate. Both cardinality and the ordering
of UNCHOSEN entries matter; merely counting r chosen entries is not enough. -/
theorem sorted_prefix_dominates [DecidableEq Id] (chosen picks : Finset Id) (weight : Id → ℝ) (cutoff : ℝ)
    (hsize : picks.card ≤ chosen.card) (hc : 0 ≤ cutoff)
    (hchosen : ∀ id ∈ chosen, cutoff ≤ weight id)
    (houtside : ∀ id ∈ picks, id ∉ chosen → weight id ≤ cutoff) :
    (∑ id ∈ picks, weight id) ≤ ∑ id ∈ chosen, weight id := by
  classical
  let common := picks ∩ chosen
  have hsplitP : (∑ id ∈ picks \ chosen, weight id) + (∑ id ∈ common, weight id) =
      ∑ id ∈ picks, weight id := by
    exact sum_split picks chosen weight
  have hsplitC : (∑ id ∈ chosen \ picks, weight id) + (∑ id ∈ common, weight id) =
      ∑ id ∈ chosen, weight id := by
    simpa [common, Finset.inter_comm] using sum_split chosen picks weight
  have hcardP := Finset.card_sdiff_add_card_inter picks chosen
  have hcardC := Finset.card_sdiff_add_card_inter chosen picks
  have hcard : (picks \ chosen).card ≤ (chosen \ picks).card := by
    rw [Finset.inter_comm chosen picks] at hcardC
    omega
  have hcardR : ((picks \ chosen).card : ℝ) ≤ (chosen \ picks).card := by exact_mod_cast hcard
  have hupper : (∑ id ∈ picks \ chosen, weight id) ≤ (picks \ chosen).card * cutoff := by
    calc
      (∑ id ∈ picks \ chosen, weight id) ≤ ∑ _id ∈ picks \ chosen, cutoff := by
        apply Finset.sum_le_sum
        intro id hi
        rcases Finset.mem_sdiff.mp hi with ⟨hp, hn⟩
        exact houtside id hp hn
      _ = (picks \ chosen).card * cutoff := by simp
  have hlower : (chosen \ picks).card * cutoff ≤ ∑ id ∈ chosen \ picks, weight id := by
    calc
      (chosen \ picks).card * cutoff = ∑ _id ∈ chosen \ picks, cutoff := by simp
      _ ≤ ∑ id ∈ chosen \ picks, weight id :=
        Finset.sum_le_sum (fun id hi => hchosen id (Finset.mem_sdiff.mp hi).1)
  have hmul := mul_le_mul_of_nonneg_right hcardR hc
  linarith

/-- Gain above the support cutoff. The algebra works with fractional values. -/
noncomputable def gain (value cutoff : ℝ) : ℝ := max 0 (value - cutoff)

theorem gain_gap (a b cutoff : ℝ) : gain a cutoff - gain b cutoff ≤ max 0 (a - b) := by
  have h0 : 0 ≤ max 0 (b - cutoff) + max 0 (a - b) :=
    add_nonneg (le_max_left _ _) (le_max_left _ _)
  have ha : a - cutoff ≤ max 0 (b - cutoff) + max 0 (a - b) := by
    linarith [le_max_right 0 (b - cutoff), le_max_right 0 (a - b)]
  have h := max_le h0 ha
  unfold gain
  linarith

/-- The rank-derived floor f only strengthens the loss bound in the b<f
case. Its rank premise is explicit, not inferred from an arbitrary f. -/
theorem replacement_loss (a b cutoff floor : ℝ) (hfloor : b < floor → floor ≤ cutoff) :
    gain a cutoff - gain b cutoff ≤ max 0 (a - max b floor) := by
  by_cases hb : floor ≤ b
  · simpa only [max_eq_left hb] using gain_gap a b cutoff
  · have hfb : b < floor := lt_of_not_ge hb
    have ht := hfloor hfb
    have hgb : gain b cutoff = 0 := by
      unfold gain
      exact max_eq_left (by linarith)
    have ha : gain a cutoff ≤ max 0 (a - floor) :=
      max_le_max_left 0 (by linarith)
    simpa only [hgb, sub_zero, max_eq_right hfb.le] using ha

/-- Coordinatewise dominance remains safe under monotone rounded additions;
there is no invalid rewrite of a rounded sum as an exact support subtraction. -/
theorem rounded_sum_mono (round : ℝ → ℝ) (hround : Monotone round)
    (old new : List ℝ) (hvalues : List.Forall₂ (· ≤ ·) old new)
    (oldStart newStart : ℝ) (hstart : oldStart ≤ newStart) :
    old.foldl (fun sum value => round (sum + value)) oldStart ≤
      new.foldl (fun sum value => round (sum + value)) newStart := by
  induction hvalues generalizing oldStart newStart with
  | nil => exact hstart
  | cons hvalue htail ih =>
      exact ih _ _ (hround (add_le_add hstart hvalue))

end Allium.Support
