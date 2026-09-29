import Allium.TopK
import Mathlib.Data.Finset.Max

/-!
# Bounded canonical collection and group merging

This proves that the rank specification really returns min(K, distinct IDs)
candidates, and that truncating after each insertion or inside each group is
exact. Merely proving a property of an arbitrarily empty result would not do.

Source: tracker.rs; Challenge-all merge; composition regimes.
-/
namespace Allium

variable {Candidate Identity : Type*} [LinearOrder Candidate] [DecidableEq Identity]

/-- Every public set has a least concrete representative in a finite pool. -/
theorem best_representative (identity : Candidate → Identity)
    (pool : Finset Candidate) (candidate : Candidate) (hmem : candidate ∈ pool) :
    ∃ best ∈ pool,
      (∀ other ∈ pool, identity other = identity best → best ≤ other) ∧
      identity best = identity candidate ∧ best ≤ candidate := by
  classical
  let fiber := pool.filter (fun other => identity other = identity candidate)
  have hx : candidate ∈ fiber := by simp [fiber, hmem]
  have hnonempty : fiber.Nonempty := ⟨candidate, hx⟩
  let best := fiber.min' hnonempty
  have hb : best ∈ fiber := Finset.min'_mem fiber hnonempty
  rcases Finset.mem_filter.mp hb with ⟨hpool, hid⟩
  refine ⟨best, hpool, ?_, hid, Finset.min'_le fiber candidate hx⟩
  intro other ho hi
  exact Finset.min'_le fiber other (Finset.mem_filter.mpr ⟨ho, hi.trans hid⟩)

/-- Top-K itself provides the certificates required to safely discard the
rest of a finite pool. Witnesses are counted by distinct public identity. -/
theorem topK_covers (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) :
    ∀ candidate ∈ pool, Covered identity k (topK identity k pool) candidate := by
  classical
  intro candidate hc
  rcases best_representative identity pool candidate hc with
    ⟨best, hbestMem, hbest, hbestId, hbestLe⟩
  by_cases hkept : best ∈ topK identity k pool
  · exact Or.inl ⟨best, hkept, hbestId, hbestLe⟩
  · let bad := pool.filter (fun a =>
      (∀ b ∈ pool, identity b = identity a → a ≤ b) ∧
      a ≤ candidate ∧ a ∉ topK identity k pool)
    have hbadBest : best ∈ bad := Finset.mem_filter.mpr
      ⟨hbestMem, hbest, hbestLe, hkept⟩
    have hnonempty : bad.Nonempty := ⟨best, hbadBest⟩
    let pivot := bad.min' hnonempty
    have hpivot : pivot ∈ bad := Finset.min'_mem bad hnonempty
    rcases Finset.mem_filter.mp hpivot with ⟨hpMem, hpBest, hpLe, hpNot⟩
    have hrank : k ≤ (earlierIds identity pool pivot).card := by
      by_contra hnot
      exact hpNot ((mem_topK identity k pool pivot).mpr
        ⟨hpMem, hpBest, Nat.lt_of_not_ge hnot⟩)
    have hbefore : earlierIds identity pool pivot ⊆
        earlierIds identity (topK identity k pool) candidate := by
      intro key hk
      rcases (mem_earlierIds _ _ _ _).mp hk with ⟨other, ho, hv, hid⟩
      rcases best_representative identity pool other ho with
        ⟨replacement, hm, hb, hi, hl⟩
      have hrp : replacement < pivot := lt_of_le_of_lt hl hv
      have hrTop : replacement ∈ topK identity k pool := by
        by_contra hn
        have hrBad : replacement ∈ bad := Finset.mem_filter.mpr
          ⟨hm, hb, hrp.le.trans hpLe, hn⟩
        exact (not_lt_of_ge (Finset.min'_le bad replacement hrBad)) hrp
      exact (mem_earlierIds _ _ _ _).mpr
        ⟨replacement, hrTop, lt_of_lt_of_le hrp hpLe, hi.trans hid⟩
    exact Or.inr (hrank.trans (Finset.card_le_card hbefore))

/-- The result cannot exceed its requested capacity. -/
theorem topK_card_le (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) : (topK identity k pool).card ≤ k := by
  classical
  let kept := topK identity k pool
  change kept.card ≤ k
  by_cases hn : kept.Nonempty
  · let last := kept.max' hn
    have hlast : last ∈ kept := Finset.max'_mem kept hn
    have hrank := ((mem_topK identity k pool last).mp hlast).2.2
    have hsubset : (kept.erase last).image identity ⊆ earlierIds identity pool last := by
      intro key hk
      rcases Finset.mem_image.mp hk with ⟨other, ho, hid⟩
      rcases Finset.mem_erase.mp ho with ⟨hne, hm⟩
      have hlt : other < last := lt_iff_le_and_ne.mpr
        ⟨Finset.le_max' kept other hm, hne⟩
      exact (mem_earlierIds _ _ _ _).mpr
        ⟨other, topK_subset identity k pool hm, hlt, hid⟩
    have hinj : Set.InjOn identity (kept.erase last) := by
      intro a ha b hb hid
      exact topK_identity_injective identity k pool
        (Finset.mem_erase.mp ha).2 (Finset.mem_erase.mp hb).2 hid
    have hcard := Finset.card_le_card hsubset
    rw [Finset.card_image_of_injOn hinj] at hcard
    have herase := Finset.card_erase_add_one hlast
    omega
  · have he : kept = ∅ := Finset.not_nonempty_iff_eq_empty.mp hn
    simp [he]

/-- Exactly min(K, the number of public sets) representatives are returned. -/
theorem topK_card (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) :
    (topK identity k pool).card = min k (pool.image identity).card := by
  classical
  let kept := topK identity k pool
  have hinj : Set.InjOn identity kept := topK_identity_injective identity k pool
  have hleK : kept.card ≤ k := topK_card_le identity k pool
  have hleId : kept.card ≤ (pool.image identity).card := by
    rw [← Finset.card_image_of_injOn hinj]
    exact Finset.card_le_card (Finset.image_subset_image (topK_subset identity k pool))
  change kept.card = min k (pool.image identity).card
  by_cases heq : kept.card = k
  · rw [heq] at hleId ⊢
    exact (min_eq_left hleId).symm
  · have hlt : kept.card < k := by omega
    have hfull : pool.image identity ⊆ kept.image identity := by
      intro key hk
      rcases Finset.mem_image.mp hk with ⟨candidate, hc, hid⟩
      rcases topK_covers identity k pool candidate hc with ⟨other, ho, hi, _⟩ | hr
      · exact Finset.mem_image.mpr ⟨other, ho, hi.trans hid⟩
      · have hs : earlierIds identity kept candidate ⊆ kept.image identity :=
          Finset.image_subset_image (Finset.filter_subset _ _)
        have hbound := Finset.card_le_card hs
        rw [Finset.card_image_of_injOn hinj] at hbound
        exact False.elim (Nat.not_le_of_lt hlt (hr.trans hbound))
    have hidEq : kept.image identity = pool.image identity :=
      Finset.Subset.antisymm
        (Finset.image_subset_image (topK_subset identity k pool)) hfull
    have hcount : (pool.image identity).card = kept.card := by
      rw [← hidEq, Finset.card_image_of_injOn hinj]
    rw [hcount]
    exact (min_eq_right hleK).symm

theorem topK_idempotent (identity : Candidate → Identity) (k : ℕ)
    (pool : Finset Candidate) :
    topK identity k (topK identity k pool) = topK identity k pool :=
  coverage_exactness identity k (topK_subset identity k pool) (topK_covers identity k pool)

/-- Local truncation followed by insertion/union preserves global Top-K. -/
theorem topK_union_left (identity : Candidate → Identity) (k : ℕ)
    (left right : Finset Candidate) :
    topK identity k (topK identity k left ∪ right) = topK identity k (left ∪ right) := by
  apply coverage_exactness identity k
  · exact Finset.union_subset_union (topK_subset identity k left) (fun _ h => h)
  · intro candidate hc
    rcases Finset.mem_union.mp hc with hl | hr
    · exact covered_mono identity k Finset.subset_union_left (topK_covers identity k left candidate hl)
    · exact covered_of_mem identity k (Finset.mem_union_right _ hr)

/-- Challenge-all and scenario merging are exact even with duplicate public
sets across groups, unequal group sizes, ties, variants, and K=0. -/
theorem group_topK_merge {Group : Type*} [DecidableEq Group]
    (identity : Candidate → Identity) (k : ℕ) (groups : Finset Group)
    (pool : Group → Finset Candidate) :
    topK identity k (groups.biUnion (fun g => topK identity k (pool g))) =
      topK identity k (groups.biUnion pool) := by
  apply coverage_exactness identity k
  · intro candidate hc
    rcases Finset.mem_biUnion.mp hc with ⟨g, hg, hm⟩
    exact Finset.mem_biUnion.mpr ⟨g, hg, topK_subset identity k (pool g) hm⟩
  · intro candidate hc
    rcases Finset.mem_biUnion.mp hc with ⟨g, hg, hm⟩
    apply covered_mono identity k _ (topK_covers identity k (pool g) candidate hm)
    intro other ho
    exact Finset.mem_biUnion.mpr ⟨g, hg, ho⟩

/-- A bounded mathematical tracker, truncating after every insertion. -/
noncomputable def collect (identity : Candidate → Identity) (k : ℕ) :
    List Candidate → Finset Candidate
  | [] => ∅
  | candidate :: rest => topK identity k (insert candidate (collect identity k rest))

theorem collect_exact (identity : Candidate → Identity) (k : ℕ)
    (candidates : List Candidate) :
    collect identity k candidates = topK identity k candidates.toFinset := by
  induction candidates with
  | nil => simp [collect, topK]
  | cons candidate rest ih =>
      simp only [collect, ih, List.toFinset_cons]
      simpa only [Finset.union_singleton] using
        topK_union_left identity k rest.toFinset {candidate}

theorem collect_order_independent (identity : Candidate → Identity) (k : ℕ)
    (left right : List Candidate) (h : left.toFinset = right.toFinset) :
    collect identity k left = collect identity k right := by
  rw [collect_exact, collect_exact, h]

end Allium
