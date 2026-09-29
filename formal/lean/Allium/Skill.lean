import Allium.Arithmetic
import Allium.FiniteBounds

/-!
# Composition-sensitive skill ceilings

Unit-count tables are deliberately NOT assumed monotone. Reference shares are
modelled in exact hundredths, with the floating-point bridge kept separate.
The four reference positions are the other four deck members, not the holder.

Source: skill_ceiling.rs; evaluate.rs; pruning-proof §20.
-/
namespace Allium.Skill

/-- Encoded Skill objective, before the evaluator's floating-point bridge. -/
def key (sum leader : ℕ) : ℕ := 2 * sum + 8 * leader

theorem key_mono {s s' l l' : ℕ} (hs : s ≤ s') (hl : l ≤ l') :
    key s l ≤ key s' l' := by unfold key; omega

theorem effective_skill_identity (sum leader : ℝ) :
    10 * (leader + (sum - leader) / 5) = 2 * sum + 8 * leader := by ring

/-- Global remaining-member ceiling, including the leader's extra weight. -/
theorem global_bound (sum leader extraSum extraLeader free maximum : ℕ)
    (hs : extraSum ≤ free * maximum) (hl : extraLeader ≤ maximum) :
    key (sum + extraSum) (max leader extraLeader) ≤
      key (sum + free * maximum) (max leader maximum) := by
  exact key_mono (Nat.add_le_add_left hs sum) (max_le_max_left leader hl)

/-- Prefix maxima, not the last element of a potentially nonmonotone table. -/
def unitCeiling (table : ℕ → ℕ) (staticMax possibleCount : ℕ) : ℕ :=
  min staticMax ((Finset.range (min 5 (max 1 possibleCount))).sup table)

theorem unit_count_sound (table : ℕ → ℕ) (staticMax actualCount possibleCount : ℕ)
    (hdeck : actualCount ≤ 5) (hcount : actualCount ≤ possibleCount)
    (hstatic : table (actualCount - 1) ≤ staticMax) :
    table (actualCount - 1) ≤ unitCeiling table staticMax possibleCount := by
  apply le_min hstatic
  apply Finset.le_sup (f := table)
  simp only [Finset.mem_range]
  omega

/-- Each additional card increases a selected unit's population by at most one. -/
theorem counted_union_bound {Card : Type*} [DecidableEq Card]
    (selected remaining : Finset Card) (p : Card → Prop) [DecidablePred p] :
    ((selected ∪ remaining).filter p).card ≤
      (selected.filter p).card + remaining.card := by
  rw [Finset.filter_union]
  exact (Finset.card_union_le _ _).trans
    (Nat.add_le_add_left (Finset.card_filter_le _ _) _)

/-- Empty and own units are removed before measuring different units. -/
def otherUnits {Card Unit : Type*} [DecidableEq Unit]
    (cards : Finset Card) (memberUnit : Card → Option Unit) (own : Option Unit) :
    Finset (Option Unit) :=
  (cards.image memberUnit).filter (fun unit => unit ≠ none ∧ unit ≠ own)

theorem other_units_union_bound {Card Unit : Type*} [DecidableEq Card] [DecidableEq Unit]
    (selected remaining : Finset Card) (memberUnit : Card → Option Unit) (own : Option Unit) :
    (otherUnits (selected ∪ remaining) memberUnit own).card ≤
      (otherUnits selected memberUnit own).card + remaining.card := by
  unfold otherUnits
  rw [Finset.image_union, Finset.filter_union]
  exact (Finset.card_union_le _ _).trans
    (Nat.add_le_add_left ((Finset.card_filter_le _ _).trans Finset.card_image_le) _)

def differentUnitValue (base increment staticMax counted : ℕ) : ℕ :=
  min staticMax (base + increment * min 2 counted)

theorem different_unit_mono (base increment staticMax : ℕ) :
    Monotone (differentUnitValue base increment staticMax) := by
  intro a b h
  apply min_le_min_left
  apply Nat.add_le_add_left
  exact Nat.mul_le_mul_left increment (min_le_min_left 2 h)

theorem different_unit_sound {Card Unit : Type*} [DecidableEq Card] [DecidableEq Unit]
    (selected remaining : Finset Card) (memberUnit : Card → Option Unit)
    (own : Option Unit) (base increment staticMax free : ℕ) (hfree : remaining.card ≤ free) :
    differentUnitValue base increment staticMax
      (otherUnits (selected ∪ remaining) memberUnit own).card ≤
    differentUnitValue base increment staticMax
      ((otherUnits selected memberUnit own).card + free) := by
  apply different_unit_mono
  exact (other_units_union_bound selected remaining memberUnit own).trans
    (Nat.add_le_add_left hfree _)

/-- Share in hundredths of one percent; inputs are integer static references. -/
def referenceShare (reference rate cap : ℕ) : ℕ := min (reference * rate) (100 * cap)

theorem reference_share_cap (reference rate cap : ℕ) :
    referenceShare reference rate cap ≤ 100 * cap := min_le_right _ _

theorem reference_share_mono {reference reference' rate rate' cap cap' : ℕ}
    (hr : reference ≤ reference') (hw : rate ≤ rate') (hc : cap ≤ cap') :
    referenceShare reference rate cap ≤ referenceShare reference' rate' cap' := by
  exact min_le_min (Nat.mul_le_mul hr hw) (Nat.mul_le_mul_left 100 hc)

def max4 (values : Fin 4 → ℕ) : ℕ := max (values 0) (max (values 1) (max (values 2) (values 3)))
def min4 (values : Fin 4 → ℕ) : ℕ := min (values 0) (min (values 1) (min (values 2) (values 3)))
def sum4 (values : Fin 4 → ℕ) : ℕ := values 0 + values 1 + values 2 + values 3

theorem max4_mono {a b : Fin 4 → ℕ} (h : ∀ i, a i ≤ b i) : max4 a ≤ max4 b :=
  max_le_max (h 0) (max_le_max (h 1) (max_le_max (h 2) (h 3)))

theorem min4_mono {a b : Fin 4 → ℕ} (h : ∀ i, a i ≤ b i) : min4 a ≤ min4 b :=
  min_le_min (h 0) (min_le_min (h 1) (min_le_min (h 2) (h 3)))

theorem sum4_mono {a b : Fin 4 → ℕ} (h : ∀ i, a i ≤ b i) : sum4 a ≤ sum4 b := by
  unfold sum4
  exact Nat.add_le_add (Nat.add_le_add (Nat.add_le_add (h 0) (h 1)) (h 2)) (h 3)

inductive ReferenceStrategy where
  | maximum | minimum | average

def referenceUpper (strategy : ReferenceStrategy) (shares : Fin 4 → ℕ) : ℕ :=
  match strategy with
  | .maximum => Arithmetic.ceilDiv (max4 shares) 100
  | .minimum => Arithmetic.ceilDiv (min4 shares) 100
  | .average => sum4 shares / 400 + 1

noncomputable def referenceExact (strategy : ReferenceStrategy) (shares : Fin 4 → ℕ) : ℝ :=
  match strategy with
  | .maximum => (max4 shares : ℝ) / 100
  | .minimum => (min4 shares : ℝ) / 100
  | .average => (sum4 shares : ℝ) / 400

theorem reference_upper_mono (strategy : ReferenceStrategy) {a b : Fin 4 → ℕ}
    (h : ∀ i, a i ≤ b i) : referenceUpper strategy a ≤ referenceUpper strategy b := by
  cases strategy with
  | maximum => exact Arithmetic.ceilDiv_mono 100 (max4_mono h)
  | minimum => exact Arithmetic.ceilDiv_mono 100 (min4_mono h)
  | average => exact Nat.add_le_add_right (Nat.div_le_div_right (sum4_mono h)) 1

/-- Rational reference values are below the implemented integer ceiling.
The Average branch leaves at least 1/400 for its floating-point bridge. -/
theorem reference_exact_bound (strategy : ReferenceStrategy) (shares : Fin 4 → ℕ) :
    referenceExact strategy shares ≤ (referenceUpper strategy shares : ℝ) := by
  cases strategy with
  | maximum =>
      have h := Arithmetic.ceilDiv_upper (max4 shares) 100 (by decide)
      change (max4 shares : ℝ) / 100 ≤ (Arithmetic.ceilDiv (max4 shares) 100 : ℝ)
      rw [div_le_iff₀ (by norm_num : (0 : ℝ) < 100)]
      exact_mod_cast h
  | minimum =>
      have h := Arithmetic.ceilDiv_upper (min4 shares) 100 (by decide)
      change (min4 shares : ℝ) / 100 ≤ (Arithmetic.ceilDiv (min4 shares) 100 : ℝ)
      rw [div_le_iff₀ (by norm_num : (0 : ℝ) < 100)]
      exact_mod_cast h
  | average =>
      have hm := Nat.mod_lt (sum4 shares) (by decide : 0 < 400)
      have hd := Nat.mod_add_div (sum4 shares) 400
      have h : sum4 shares ≤ (sum4 shares / 400 + 1) * 400 := by omega
      change (sum4 shares : ℝ) / 400 ≤ ((sum4 shares / 400 + 1 : ℕ) : ℝ)
      rw [div_le_iff₀ (by norm_num : (0 : ℝ) < 400)]
      exact_mod_cast h

def relaxedShares (known : Finset (Fin 4)) (references : Fin 4 → ℕ) (rate cap : ℕ) :
    Fin 4 → ℕ := fun i =>
  if i ∈ known then referenceShare (references i) rate cap else 100 * cap

/-- Each unknown member can be safely replaced by the cap. -/
theorem reference_relax_sound (strategy : ReferenceStrategy) (known : Finset (Fin 4))
    (references : Fin 4 → ℕ) (rate cap : ℕ) :
    referenceUpper strategy (fun i => referenceShare (references i) rate cap) ≤
      referenceUpper strategy (relaxedShares known references rate cap) := by
  apply reference_upper_mono
  intro i
  unfold relaxedShares
  split_ifs
  · exact le_rfl
  · exact reference_share_cap _ _ _

/-- Merging candidate rules componentwise may lose correlation, but never
underestimates: candidate ceilings never read the holder's own reference. -/
theorem merged_reference_rule_sound (strategy : ReferenceStrategy) (known : Finset (Fin 4))
    (references : Fin 4 → ℕ) (base base' rate rate' cap cap' : ℕ)
    (hb : base ≤ base') (hr : rate ≤ rate') (hc : cap ≤ cap') :
    base + referenceUpper strategy (relaxedShares known references rate cap) ≤
      base' + referenceUpper strategy (relaxedShares known references rate' cap') := by
  apply Nat.add_le_add hb
  apply reference_upper_mono
  intro i
  unfold relaxedShares
  split_ifs
  · exact reference_share_mono le_rfl hr hc
  · exact Nat.mul_le_mul_left 100 hc

/-- The encoded floating result is safe once its independently established
error, including the explicit offset, is strictly less than one integer unit. -/
theorem encoded_key_upper (sum leader : ℕ) (encoded error : ℝ)
    (hencoded : encoded ≤ (key sum leader : ℝ) + error) (herror : error < 1) :
    Int.floor encoded ≤ (key sum leader : ℤ) := by
  have h := Arithmetic.grid_floor_of_error (key sum leader : ℤ) 1 (by decide)
    encoded error (by simpa using hencoded) (by simpa using herror)
  simpa using h

end Allium.Skill
