import Allium.SuffixScan
import Allium.Saturating

/-!
# Numeric Power global bounds

The accepted aggregate domain is computed from the five largest card maxima
plus the honor bonus. Selected summaries, global repetitions and honors use
the same saturating u32 operations as the numeric solver. Bounds are proved
from decoded card tables and the computed acceptance guard, not supplied as
an abstract soundness premise.
-/
namespace Allium.NumericPower
open PowerModel FiniteBounds MixedPower

variable {Id : Type*} [DecidableEq Id]

def globalMaximum (pool : Finset Id) (data : Id → Card) : Nat :=
  pool.sup (fun card => maximum (data card))

noncomputable def globalMinimum (mode : Mode) (pool : Finset Id) (data : Id → Card) : Nat :=
  if h : pool.Nonempty then pool.inf' h (fun card => minimumFor mode (data card)) else 0

def domainUpper (pool : Finset Id) (data : Id → Card) (honor : Nat) : Nat :=
  maxSum 5 pool (fun card => maximum (data card)) + honor

def accepted (pool : Finset Id) (data : Id → Card) (honor : Nat) : Bool :=
  decide (domainUpper pool data honor ≤ 2 ^ 24)

def LegalPoolDeck (pool : Finset Id) (deck : List Id) : Prop :=
  deck.length = 5 ∧ deck.Nodup ∧ ∀ card ∈ deck, card ∈ pool

omit [DecidableEq Id] in
theorem maximum_le_global (pool : Finset Id) (data : Id → Card) (card : Id) (member : card ∈ pool) :
    maximum (data card) ≤ globalMaximum pool data :=
  Finset.le_sup (f := fun card => maximum (data card)) member

omit [DecidableEq Id] in
theorem global_le_minimum (mode : Mode) (pool : Finset Id) (data : Id → Card) (card : Id) (member : card ∈ pool) :
    globalMinimum mode pool data ≤ minimumFor mode (data card) := by
  have hn : pool.Nonempty := ⟨card, member⟩
  rw [globalMinimum, dif_pos hn]
  exact Finset.inf'_le _ member

/-- Equivalence with the validator's sorted top-five scan. -/
theorem domain_sorted (pool : Finset Id) (data : Id → Card) (honor : Nat) :
    domainUpper pool data honor =
      SuffixDelta.prefixSum (fun card => maximum (data card))
        (SuffixDelta.ordered (fun card => maximum (data card)) pool.toList) 5 + honor := by
  have ordered := SuffixDelta.ordered_pairwise (fun card => maximum (data card)) pool.toList
  have unique := (List.mergeSort_perm pool.toList
    (fun a b => decide (maximum (data b) ≤ maximum (data a)))).nodup_iff.mpr pool.nodup_toList
  have he := SuffixScan.prefix_eq_maxSum (fun card => maximum (data card)) _ ordered unique 5
  have hp : (SuffixDelta.ordered (fun card => maximum (data card)) pool.toList).toFinset = pool := by
    ext card
    simp only [SuffixDelta.ordered, List.mem_toFinset]
    rw [(List.mergeSort_perm pool.toList
      (fun a b => decide (maximum (data b) ≤ maximum (data a)))).mem_iff]
    exact Finset.mem_toList
  rw [hp] at he
  exact congrArg (fun power => power + honor) he.symm

theorem legal_domain_bound (mode : Mode) (pool : Finset Id) (data : Id → Card)
    (honor : Nat) (deck : List Id) (legal : LegalPoolDeck pool deck) :
    MixedPower.total mode data deck + honor ≤ domainUpper pool data honor := by
  have hs := SuffixScan.sum_toFinset (fun card => maximum (data card)) deck legal.2.1
  have hpool : deck.toFinset ⊆ pool := by
    intro card hc
    exact legal.2.2 card (List.mem_toFinset.mp hc)
  have hcount : deck.toFinset.card ≤ 5 := by
    exact (List.toFinset_card_le deck).trans (Nat.le_of_eq legal.1)
  have hb := sum_le_maxSum 5 pool deck.toFinset (fun card => maximum (data card)) hpool hcount
  rw [hs] at hb
  exact Nat.add_le_add_right ((MixedPower.total_upper mode data deck).trans hb) honor

theorem accepted_power_fits (mode : Mode) (pool : Finset Id) (data : Id → Card)
    (honor : Nat) (deck : List Id) (legal : LegalPoolDeck pool deck)
    (domain : accepted pool data honor = true) :
    MixedPower.total mode data deck + honor ≤ Saturating.u32Max := by
  have hd : domainUpper pool data honor ≤ 2 ^ 24 := by simpa [accepted] using domain
  exact (legal_domain_bound mode pool data honor deck legal).trans (hd.trans Saturating.power_domain_fits)

omit [DecidableEq Id] in
/-- Independent maxima allow every free slot to reuse the global maximum. -/
theorem global_raw_upper (mode : Mode) (pool : Finset Id) (data : Id → Card)
    (selected free : List Id) (members : ∀ card ∈ free, card ∈ pool) :
    MixedPower.total mode data (selected ++ free) ≤
      (selected.map (fun card => maximum (data card))).sum + free.length * globalMaximum pool data := by
  have hs : (selected.map (MixedPower.cardPower mode data (selected ++ free))).sum ≤
      (selected.map (fun card => maximum (data card))).sum :=
    List.sum_le_sum (fun card _ => effective_upper mode _ _ _ (data card))
  have hf : (free.map (MixedPower.cardPower mode data (selected ++ free))).sum ≤
      free.length * globalMaximum pool data := by
    calc
      _ ≤ (free.map (fun _ => globalMaximum pool data)).sum :=
        List.sum_le_sum (fun card hc => (effective_upper mode _ _ _ (data card)).trans
          (maximum_le_global pool data card (members card hc)))
      _ = _ := by simp
  simpa only [MixedPower.total, List.map_append, List.sum_append] using Nat.add_le_add hs hf

omit [DecidableEq Id] in
theorem global_raw_lower (mode : Mode) (pool : Finset Id) (data : Id → Card)
    (selected free : List Id) (members : ∀ card ∈ free, card ∈ pool) :
    (selected.map (fun card => minimumFor mode (data card))).sum + free.length * globalMinimum mode pool data ≤
      MixedPower.total mode data (selected ++ free) := by
  have hs : (selected.map (fun card => minimumFor mode (data card))).sum ≤
      (selected.map (MixedPower.cardPower mode data (selected ++ free))).sum :=
    List.sum_le_sum (fun card _ => effective_lower_for mode _ _ _ (data card))
  have hf : free.length * globalMinimum mode pool data ≤
      (free.map (MixedPower.cardPower mode data (selected ++ free))).sum := by
    calc
      _ = (free.map (fun _ => globalMinimum mode pool data)).sum := by simp
      _ ≤ _ := List.sum_le_sum (fun card hc =>
        (global_le_minimum mode pool data card (members card hc)).trans (effective_lower_for mode _ _ _ (data card)))
  simpa only [MixedPower.total, List.map_append, List.sum_append] using Nat.add_le_add hs hf

def relaxed (selectedValues : List Nat) (global slots honor : Nat) : Nat :=
  Saturating.add Saturating.u32Max
    (Saturating.add Saturating.u32Max (Saturating.sum Saturating.u32Max selectedValues)
      (Saturating.mul Saturating.u32Max global slots)) honor

theorem relaxed_eq (selectedValues : List Nat) (global slots honor : Nat) :
    relaxed selectedValues global slots honor =
      Saturating.clip Saturating.u32Max (selectedValues.sum + slots * global + honor) := by
  unfold relaxed Saturating.mul
  rw [Saturating.sum_eq, Saturating.clip_add_both]
  unfold Saturating.add
  rw [Saturating.clip_add_left]
  rw [Nat.mul_comm global slots]

/-- Actual maximizing global prune, including u32 saturation and the cap
applied after the honor bonus. -/
theorem maximizing_prune (mode : Mode) (pool : Finset Id) (data : Id → Card)
    (selected free : List Id) (honor threshold : Nat) (cap : Option Nat)
    (legal : LegalPoolDeck pool (selected ++ free)) (domain : accepted pool data honor = true)
    (cut : clamp cap (relaxed (selected.map (fun card => maximum (data card)))
      (globalMaximum pool data) (5 - selected.length) honor) < threshold) :
    clamp cap (MixedPower.total mode data (selected ++ free) + honor) < threshold := by
  have members : ∀ card ∈ free, card ∈ pool := fun card hc => legal.2.2 card (List.mem_append_right _ hc)
  have lengths : free.length = 5 - selected.length := by
    have hl := legal.1
    simp only [List.length_append] at hl
    omega
  have hb := Nat.add_le_add_right (global_raw_upper mode pool data selected free members) honor
  have hw := accepted_power_fits mode pool data honor (selected ++ free) legal domain
  apply lt_of_le_of_lt _ cut
  apply clamp_monotone cap
  rw [relaxed_eq, ← lengths]
  exact Saturating.upper_preserved _ _ _ hb hw

omit [DecidableEq Id] in
/-- Clipping a lower bound remains safe without an overflow assumption. -/
theorem minimizing_prune (mode : Mode) (pool : Finset Id) (data : Id → Card)
    (selected free : List Id) (honor threshold : Nat) (cap : Option Nat)
    (legal : LegalPoolDeck pool (selected ++ free))
    (cut : threshold < clamp cap (relaxed (selected.map (fun card => minimumFor mode (data card)))
      (globalMinimum mode pool data) (5 - selected.length) honor)) :
    threshold < clamp cap (MixedPower.total mode data (selected ++ free) + honor) := by
  have members : ∀ card ∈ free, card ∈ pool := fun card hc => legal.2.2 card (List.mem_append_right _ hc)
  have lengths : free.length = 5 - selected.length := by
    have hl := legal.1
    simp only [List.length_append] at hl
    omega
  have hb := Nat.add_le_add_right (global_raw_lower mode pool data selected free members) honor
  apply lt_of_lt_of_le cut
  apply clamp_monotone cap
  rw [relaxed_eq, ← lengths]
  exact Saturating.lower_preserved _ _ _ hb

end Allium.NumericPower
