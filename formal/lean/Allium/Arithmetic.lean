import Mathlib.Algebra.Order.Floor.Ring
import Mathlib.Data.Real.Basic
import Mathlib.Tactic

/-!
# Exact arithmetic used by pruning

These lemmas separate exact integer/rational reasoning from floating-point
error estimates. `grid_floor` requires a strict error premise; it does not
claim arbitrary binary64 expressions meet that premise.

Source: objective.rs, correlated.rs, bonus_tiers.rs; pruning-proof §§1,10,17,29.
-/
namespace Allium.Arithmetic

theorem upper_min {x a b : ℕ} (ha : x ≤ a) (hb : x ≤ b) : x ≤ min a b :=
  le_min ha hb

theorem lower_max {x a b : ℕ} (ha : a ≤ x) (hb : b ≤ x) : max a b ≤ x :=
  max_le ha hb

theorem clamp_mono (cap : ℕ) : Monotone (fun x : ℕ => min x cap) := by
  intro x y h
  exact min_le_min_right cap h

/-- Exact replacement of a hot division by a threshold multiplication. -/
theorem numerator_prune (n d t : ℕ) (hd : 0 < d) :
    n / d < t ↔ n < t * d := by
  exact Nat.div_lt_iff_lt_mul hd

def ceilDiv (n d : ℕ) : ℕ := (n + d - 1) / d

theorem ceilDiv_upper (n d : ℕ) (hd : 0 < d) : n ≤ ceilDiv n d * d := by
  have hmod := Nat.mod_lt (n + d - 1) hd
  have hdivision := Nat.mod_add_div (n + d - 1) d
  have hsub : n + d - 1 + 1 = n + d := by omega
  unfold ceilDiv
  nlinarith

theorem ceilDiv_minimal (n d q : ℕ) (hd : 0 < d) (hq : n ≤ q * d) :
    ceilDiv n d ≤ q := by
  unfold ceilDiv
  apply Nat.lt_succ_iff.mp
  rw [Nat.div_lt_iff_lt_mul hd]
  have hsub : n + d - 1 + 1 = n + d := by omega
  change n + d - 1 < (q + 1) * d
  nlinarith

theorem ceilDiv_mono (d : ℕ) {a b : ℕ} (h : a ≤ b) :
    ceilDiv a d ≤ ceilDiv b d := by
  unfold ceilDiv
  exact Nat.div_le_div_right (by omega)

/-- Mathematical packing. The width hypotheses prevent field overlap. -/
def pack (base high low : ℕ) : ℕ := high * base + low

theorem pack_mono (base : ℕ) {hi hi' lo lo' : ℕ}
    (hh : hi ≤ hi') (hl : lo ≤ lo') : pack base hi lo ≤ pack base hi' lo' := by
  unfold pack
  exact Nat.add_le_add (Nat.mul_le_mul_right base hh) hl

theorem pack_lt_iff (base hi hi' lo lo' : ℕ)
    (hlo : lo < base) (hlo' : lo' < base) :
    pack base hi lo < pack base hi' lo' ↔
      hi < hi' ∨ (hi = hi' ∧ lo < lo') := by
  unfold pack
  constructor
  · intro h
    rcases lt_trichotomy hi hi' with hh | hh | hh
    · exact Or.inl hh
    · subst hi'
      exact Or.inr ⟨rfl, by omega⟩
    · have hmul := Nat.mul_le_mul_right base (Nat.succ_le_of_lt hh)
      nlinarith
  · rintro (hh | ⟨rfl, hl⟩)
    · have hmul := Nat.mul_le_mul_right base (Nat.succ_le_of_lt hh)
      nlinarith
    · omega

theorem noevent_order (base a b : ℕ) :
    pack base a a < pack base b b ↔ a < b := by
  unfold pack
  constructor
  · intro h
    by_contra hnot
    have hmul := Nat.mul_le_mul_right base (Nat.le_of_not_gt hnot)
    omega
  · intro h
    have hmul := Nat.mul_le_mul_right base h.le
    omega

/-- Oversized exact bounds may be clipped if legal outputs fit below the
clip. Wrapping arithmetic has no corresponding theorem. -/
theorem safe_clip {value bound limit : ℕ}
    (hb : value ≤ bound) (hl : value ≤ limit) : value ≤ min bound limit :=
  le_min hb hl

theorem optional_bound {value fallback infinity : ℕ}
    (hf : value ≤ fallback) (hi : value ≤ infinity)
    (tight : Option ℕ) (ht : ∀ t, tight = some t → value ≤ t) :
    value ≤ min fallback (tight.getD infinity) := by
  cases tight with
  | none => exact le_min hf hi
  | some t => exact le_min hf (ht t rfl)

/-- The integer grid supplies a full 1/d gap to the next integer. -/
theorem grid_floor (n : ℤ) (d : ℕ) (hd : 0 < d) (x : ℝ)
    (hx : x < ((n : ℝ) + 1) / (d : ℝ)) :
    Int.floor x ≤ n / (d : ℤ) := by
  have hdZ : (0 : ℤ) < d := by exact_mod_cast hd
  have hdR : (0 : ℝ) < d := by exact_mod_cast hd
  have hmod := Int.emod_lt_of_pos n hdZ
  have heq := Int.mul_ediv_add_emod n (d : ℤ)
  have hgridZ : n + 1 ≤ (n / (d : ℤ) + 1) * (d : ℤ) := by nlinarith
  have hgridR : (n : ℝ) + 1 ≤ ((n / (d : ℤ) : ℤ) + 1 : ℝ) * (d : ℝ) := by
    exact_mod_cast hgridZ
  have hxnext : x < ((n / (d : ℤ) : ℤ) : ℝ) + 1 := by
    apply lt_of_lt_of_le hx
    exact (div_le_iff₀ hdR).mpr hgridR
  have hfloor := Int.floor_le x
  by_contra hnot
  have hi : n / (d : ℤ) + 1 ≤ Int.floor x := by omega
  have hiR : ((n / (d : ℤ) : ℤ) : ℝ) + 1 ≤ (Int.floor x : ℝ) := by
    exact_mod_cast hi
  linarith

theorem grid_floor_of_error (n : ℤ) (d : ℕ) (hd : 0 < d)
    (x error : ℝ) (hx : x ≤ (n : ℝ) / d + error) (he : error < 1 / (d : ℝ)) :
    Int.floor x ≤ n / (d : ℤ) := by
  apply grid_floor n d hd x
  calc
    x ≤ (n : ℝ) / d + error := hx
    _ < (n : ℝ) / d + 1 / (d : ℝ) := add_lt_add_left he _
    _ = ((n : ℝ) + 1) / (d : ℝ) := by ring

/-- Strict support compensation must pay for both decks' rounding errors. -/
theorem support_compensation (oldExact newExact oldEval newEval loss surplus eOld eNew : ℝ)
    (hchange : oldExact + surplus - loss ≤ newExact)
    (hold : oldEval ≤ oldExact + eOld)
    (hnew : newExact - eNew ≤ newEval)
    (hmargin : eOld + eNew < surplus - loss) : oldEval < newEval := by
  linarith

/-- The suffix interval implied by key/slack/excess bounds for an exact tier. -/
theorem tier_suffix_interval (target pre suffix extra slack extraLo extraHi
    slackMax excess excessMax : ℤ)
    (hlower : pre + suffix + extra - excess ≤ target)
    (hupper : target ≤ pre + suffix + slack + extra)
    (heLo : extraLo ≤ extra) (heHi : extra ≤ extraHi)
    (hs : slack ≤ slackMax) (hx : excess ≤ excessMax) :
    target - extraHi - slackMax - pre ≤ suffix ∧
      suffix ≤ target - extraLo + excessMax - pre := by
  constructor <;> omega

theorem unavoidable_bonus_exclusion (unavoidable total maxTier requested : ℕ)
    (hcontribution : unavoidable ≤ total) (hrequest : requested ≤ maxTier)
    (hexceeds : maxTier < unavoidable) : total ≠ requested := by
  omega

end Allium.Arithmetic
