import Mathlib.Data.Finset.Sort
import Mathlib.Data.Finset.Image
import Mathlib.Tactic

/-!
# Canonical Top-K with public-set identity

The order on `Candidate` is the complete better-first order, NOT just the
numeric objective. `identity` identifies the public card set; cultivation
variants and placements remain distinct candidates until collection.

`topK` is specified by best representatives and a count of DISTINCT preceding
identities. The coverage theorem is the reusable exactness argument: a search
must either retain a no-worse representative of the same identity, or exhibit
K distinct retained identities strictly better than the discarded candidate.
No bound soundness or search exactness is assumed by this theorem.

Source correspondence: search/tracker.rs; pruning-proof sections 1, 6, 13, 22.
-/

namespace Allium

variable {Candidate Identity : Type*} [LinearOrder Candidate] [DecidableEq Identity]

/-- Public identities with at least one strictly better concrete candidate. -/
def earlierIds (identity : Candidate → Identity) (pool : Finset Candidate)
    (candidate : Candidate) : Finset Identity :=
  (pool.filter (fun other => other < candidate)).image identity

/-- The canonical representatives whose distinct-public-set rank is below K. -/
noncomputable def topK (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) : Finset Candidate := by
  classical
  exact pool.filter (fun candidate =>
    (∀ other ∈ pool, identity other = identity candidate → candidate ≤ other) ∧
    (earlierIds identity pool candidate).card < k)

@[simp] theorem mem_topK (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) (candidate : Candidate) :
    candidate ∈ topK identity k pool ↔
      candidate ∈ pool ∧
      (∀ other ∈ pool, identity other = identity candidate → candidate ≤ other) ∧
      (earlierIds identity pool candidate).card < k := by
  classical
  simp [topK]

@[simp] theorem mem_earlierIds (identity : Candidate → Identity)
    (pool : Finset Candidate) (candidate : Candidate) (key : Identity) :
    key ∈ earlierIds identity pool candidate ↔
      ∃ other ∈ pool, other < candidate ∧ identity other = key := by
  simp only [earlierIds, Finset.mem_image, Finset.mem_filter]
  aesop

theorem earlierIds_mono_pool (identity : Candidate → Identity)
    {small large : Finset Candidate} (h : small ⊆ large) (candidate : Candidate) :
    earlierIds identity small candidate ⊆ earlierIds identity large candidate := by
  intro key hk
  rcases (mem_earlierIds _ _ _ _).mp hk with ⟨other, ho, hc, rfl⟩
  exact (mem_earlierIds _ _ _ _).mpr ⟨other, h ho, hc, rfl⟩

theorem earlierIds_mono_key (identity : Candidate → Identity)
    (pool : Finset Candidate) {a b : Candidate} (h : a ≤ b) :
    earlierIds identity pool a ⊆ earlierIds identity pool b := by
  intro key hk
  rcases (mem_earlierIds _ _ _ _).mp hk with ⟨other, ho, hc, rfl⟩
  exact (mem_earlierIds _ _ _ _).mpr ⟨other, ho, lt_of_lt_of_le hc h, rfl⟩

/-- A certificate for omitting one concrete candidate. -/
def Covered (identity : Candidate → Identity) (k : ℕ)
    (retained : Finset Candidate) (candidate : Candidate) : Prop :=
  (∃ other ∈ retained, identity other = identity candidate ∧ other ≤ candidate) ∨
    k ≤ (earlierIds identity retained candidate).card

theorem covered_of_mem (identity : Candidate → Identity) (k : ℕ)
    {retained : Finset Candidate} {candidate : Candidate} (h : candidate ∈ retained) :
    Covered identity k retained candidate :=
  Or.inl ⟨candidate, h, rfl, le_rfl⟩

theorem covered_mono (identity : Candidate → Identity) (k : ℕ)
    {small large : Finset Candidate} (h : small ⊆ large) {candidate : Candidate}
    (hc : Covered identity k small candidate) : Covered identity k large candidate := by
  rcases hc with ⟨other, ho, hi, hv⟩ | hr
  · exact Or.inl ⟨other, h ho, hi, hv⟩
  · exact Or.inr (hr.trans (Finset.card_le_card (earlierIds_mono_pool identity h candidate)))

/-- Replacing a pool by a valid coverage certificate preserves the full Top-K,
including placement/variant representatives and all equal-score ties. -/
theorem coverage_exactness (identity : Candidate → Identity) (k : ℕ)
    {full retained : Finset Candidate} (hsub : retained ⊆ full)
    (hcover : ∀ candidate ∈ full, Covered identity k retained candidate) :
    topK identity k retained = topK identity k full := by
  classical
  ext candidate
  rw [mem_topK, mem_topK]
  constructor
  · rintro ⟨hmem, hbest, hrank⟩
    have lift (other : Candidate) (ho : other ∈ full) (hord : other ≤ candidate) :
        ∃ replacement ∈ retained,
          identity replacement = identity other ∧ replacement ≤ other := by
      rcases hcover other ho with h | h
      · exact h
      · have hle := Finset.card_le_card (earlierIds_mono_key identity retained hord)
        exact False.elim (Nat.not_le_of_lt hrank (h.trans hle))
    have hbestFull : ∀ other ∈ full,
        identity other = identity candidate → candidate ≤ other := by
      intro other ho hi
      by_contra hnot
      have hlt : other < candidate := lt_of_not_ge hnot
      rcases lift other ho hlt.le with ⟨replacement, hm, hid, hv⟩
      have hc := hbest replacement hm (hid.trans hi)
      exact (not_lt_of_ge (hc.trans hv)) hlt
    have hbefore : earlierIds identity full candidate ⊆
        earlierIds identity retained candidate := by
      intro key hk
      rcases (mem_earlierIds _ _ _ _).mp hk with ⟨other, ho, hv, hid⟩
      rcases lift other ho hv.le with ⟨replacement, hm, hi, hr⟩
      exact (mem_earlierIds _ _ _ _).mpr
        ⟨replacement, hm, lt_of_le_of_lt hr hv, hi.trans hid⟩
    exact ⟨hsub hmem, hbestFull,
      lt_of_le_of_lt (Finset.card_le_card hbefore) hrank⟩
  · rintro ⟨hmem, hbest, hrank⟩
    have hretained : candidate ∈ retained := by
      rcases hcover candidate hmem with ⟨other, ho, hi, hv⟩ | h
      · have heq : other = candidate := le_antisymm hv (hbest other (hsub ho) hi)
        simpa only [heq] using ho
      · have hle := Finset.card_le_card (earlierIds_mono_pool identity hsub candidate)
        exact False.elim (Nat.not_le_of_lt hrank (h.trans hle))
    exact ⟨hretained, fun other ho hi => hbest other (hsub ho) hi,
      lt_of_le_of_lt
        (Finset.card_le_card (earlierIds_mono_pool identity hsub candidate)) hrank⟩

theorem topK_subset (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) : topK identity k pool ⊆ pool := by
  intro candidate hc
  exact ((mem_topK _ _ _ _).mp hc).1

/-- Deduplication is by identity, not by objective or concrete variant. -/
theorem topK_identity_injective (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) : Set.InjOn identity (topK identity k pool) := by
  intro a ha b hb hab
  rcases (mem_topK _ _ _ _).mp ha with ⟨ham, habest, _⟩
  rcases (mem_topK _ _ _ _).mp hb with ⟨hbm, hbbest, _⟩
  exact le_antisymm (habest b hbm hab.symm) (hbbest a ham hab)

/-- Sorting the proven set gives exactly the same canonical result sequence. -/
theorem coverage_canonical_sequence (identity : Candidate → Identity) (k : ℕ)
    {full retained : Finset Candidate} (hsub : retained ⊆ full)
    (hcover : ∀ candidate ∈ full, Covered identity k retained candidate) :
    (topK identity k retained).sort (· ≤ ·) =
      (topK identity k full).sort (· ≤ ·) := by
  rw [coverage_exactness identity k hsub hcover]

/-- Distinct already evaluated identities whose scores strictly exceed U. -/
def strictWitnesses (identity : Candidate → Identity) (score : Candidate → ℕ)
    (retained : Finset Candidate) (upper : ℕ) : Finset Identity :=
  (retained.filter (fun other => upper < score other)).image identity

/-- The numeric prune uses STRICT comparison. The later key fields need no
assumption because a strict primary improvement decides the complete order. -/
theorem strict_upper_prune (identity : Candidate → Identity) (score : Candidate → ℕ)
    (horder : ∀ a b, score b < score a → a < b) (k : ℕ)
    (retained : Finset Candidate) (candidate : Candidate) (upper : ℕ)
    (hsound : score candidate ≤ upper)
    (hwitness : k ≤ (strictWitnesses identity score retained upper).card) :
    Covered identity k retained candidate := by
  apply Or.inr
  apply hwitness.trans
  apply Finset.card_le_card
  intro key hk
  rcases Finset.mem_image.mp hk with ⟨other, ho, hi⟩
  rcases Finset.mem_filter.mp ho with ⟨hm, hv⟩
  exact (mem_earlierIds _ _ _ _).mpr
    ⟨other, hm, horder other candidate (lt_of_le_of_lt hsound hv), hi⟩

end Allium
