import Allium.Arithmetic
import Mathlib.Data.Prod.Lex
import Mathlib.Data.Finset.Max

/-!
# Binary64 values for numeric admission

Finite encodings retain their sign, biased exponent and fraction. Arithmetic
rounds exact rational results to the nearest finite encoding, ties to even,
with the binary64 overflow midpoint handled separately. NaNs and infinities
remain distinct from finite numbers. Zero signs may be canonicalized by
arithmetic: the admission expressions use no division by a floating zero and
observe zeros only through arithmetic values and range comparisons.

This is a numeric semantics, not a floating-error bound for a search ceiling.
-/
namespace Allium.Binary64

abbrev PositiveBits := Fin (2047 * 2 ^ 52)

def exponent (bits : PositiveBits) : Nat := bits.val / 2 ^ 52
def fraction (bits : PositiveBits) : Nat := bits.val % 2 ^ 52

def magnitude (bits : PositiveBits) : ℚ :=
  if exponent bits = 0 then (fraction bits : ℚ) * (2 : ℚ) ^ (-1074 : Int)
  else ((2 ^ 52 + fraction bits : Nat) : ℚ) * (2 : ℚ) ^ ((exponent bits : Int) - 1075)

@[simp] theorem zero_magnitude : magnitude 0 = 0 := by
  simp [magnitude, exponent, fraction]

theorem magnitude_nonnegative (bits : PositiveBits) : 0 ≤ magnitude bits := by
  unfold magnitude
  split <;> positivity

inductive Value where
  | finite (negative : Bool) (bits : PositiveBits)
  | infinity (negative : Bool)
  | nan
  deriving DecidableEq

def toRat : Value → Option ℚ
  | .finite negative bits => some (if negative then -magnitude bits else magnitude bits)
  | _ => none

def sign : Value → Bool
  | .finite negative _ => negative
  | .infinity negative => negative
  | .nan => false

def isNaN : Value → Bool
  | .nan => true
  | _ => false

def isFinite : Value → Bool
  | .finite _ _ => true
  | _ => false

def isZero (value : Value) : Bool :=
  match toRat value with
  | some rational => decide (rational = 0)
  | none => false

def zero : Value := .finite false 0

def overflowMidpoint : ℚ := (2 : ℚ) ^ (1024 : Nat) - (2 : ℚ) ^ (970 : Nat)

abbrev RoundingKey := ℚ ×ₗ (Nat ×ₗ Nat)

def roundingKey (rational : ℚ) (bits : PositiveBits) : RoundingKey :=
  toLex (abs (magnitude bits - |rational|), toLex (bits.val % 2, bits.val))

private theorem closest_exists (rational : ℚ) :
    ∃ bits : PositiveBits, ∀ other : PositiveBits, roundingKey rational bits ≤ roundingKey rational other := by
  obtain ⟨bits, _, least⟩ := Finset.exists_min_image (Finset.univ : Finset PositiveBits)
    (roundingKey rational) Finset.univ_nonempty
  exact ⟨bits, fun other => least other (Finset.mem_univ _)⟩

noncomputable def closest (rational : ℚ) : PositiveBits := Classical.choose (closest_exists rational)

theorem closest_minimizes (rational : ℚ) (other : PositiveBits) :
    roundingKey rational (closest rational) ≤ roundingKey rational other :=
  Classical.choose_spec (closest_exists rational) other

/-- The distance component wins before parity and the deterministic bit tie. -/
theorem nearest_distance (rational : ℚ) (other : PositiveBits) :
    abs (magnitude (closest rational) - |rational|) ≤ abs (magnitude other - |rational|) := by
  by_contra rejected
  have smaller : abs (magnitude other - |rational|) < abs (magnitude (closest rational) - |rational|) :=
    lt_of_not_ge rejected
  have keySmaller : roundingKey rational other < roundingKey rational (closest rational) :=
    Prod.Lex.lt_iff.mpr (Or.inl smaller)
  exact (not_lt_of_ge (closest_minimizes rational other)) keySmaller

noncomputable def round (rational : ℚ) : Value :=
  if overflowMidpoint ≤ |rational| then .infinity (decide (rational < 0))
  else .finite (decide (rational < 0)) (closest rational)

noncomputable def ofNat (value : Nat) : Value := round value
noncomputable def ofInt (value : Int) : Value := round value

noncomputable def add (left right : Value) : Value :=
  match toRat left, toRat right with
  | some a, some b => round (a + b)
  | some _, none => right
  | none, some _ => left
  | none, none =>
      if isNaN left || isNaN right || decide (sign left ≠ sign right) then .nan else left

noncomputable def mul (left right : Value) : Value :=
  match toRat left, toRat right with
  | some a, some b => round (a * b)
  | _, _ =>
      if isNaN left || isNaN right || isZero left || isZero right then .nan
      else .infinity (decide (sign left ≠ sign right))

noncomputable def div (left right : Value) : Value :=
  match toRat left, toRat right with
  | some a, some b =>
      if b = 0 then
        if a = 0 then .nan else .infinity (decide (sign left ≠ sign right))
      else round (a / b)
  | some _, none => if isNaN right then .nan else zero
  | none, some _ => if isNaN left then .nan else .infinity (decide (sign left ≠ sign right))
  | none, none => .nan

/-- Integer-spelled floating literals undergo the same conversion as every
other floating operand before division. -/
noncomputable def divNat (value : Value) (divisor : Nat) : Value := div value (ofNat divisor)

/-- Numerical comparison. NaN makes a comparison false. -/
def le : Value → Value → Bool
  | .nan, _ => false
  | _, .nan => false
  | .infinity true, _ => true
  | _, .infinity false => true
  | .infinity false, _ => false
  | _, .infinity true => false
  | .finite an a, .finite bn b =>
      decide ((if an then -magnitude a else magnitude a) ≤ (if bn then -magnitude b else magnitude b))

/-- Rust min/max ignore a sole NaN. Equal zero signs are immaterial here. -/
def minimum (left right : Value) : Value :=
  if isNaN left then right else if isNaN right then left else if le left right then left else right

def maximum (left right : Value) : Value :=
  if isNaN left then right else if isNaN right then left else if le left right then right else left

noncomputable def ceil (value : Value) : Value :=
  match toRat value with
  | some rational => ofInt (Int.ceil rational)
  | none => value

def toU64 (value : Value) : Nat :=
  match value with
  | .infinity false => 2 ^ 64 - 1
  | .infinity true | .nan => 0
  | .finite negative bits =>
      min ((Int.floor (if negative then -magnitude bits else magnitude bits)).toNat) (2 ^ 64 - 1)

/-- Iteration order is retained; no real-valued sum replaces the rounded fold. -/
noncomputable def sum (values : List Value) : Value := values.foldl add zero

def nonnegative (value : Value) : Bool := isFinite value && le zero value

def atMost (value : Value) (maximum : Nat) : Bool :=
  match value with
  | .nan | .infinity false => false
  | .infinity true => true
  | .finite negative bits => decide ((if negative then -magnitude bits else magnitude bits) ≤ (maximum : ℚ))

/-- Sorting is used only after finite/nonnegative checks. The extended key
makes the total function defined on rejected inputs as well. -/
abbrev SortKey := Nat ×ₗ ℚ

def sortKey : Value → SortKey
  | .infinity true => toLex (0, 0)
  | .finite negative bits => toLex (1, if negative then -magnitude bits else magnitude bits)
  | .infinity false => toLex (2, 0)
  | .nan => toLex (3, 0)

noncomputable def sortedSum (values : List Value) (count : Nat) : Value :=
  sum ((values.mergeSort (fun a b => decide (sortKey b ≤ sortKey a))).take count)

theorem finite_nonnegative_iff (negative : Bool) (bits : PositiveBits) :
    nonnegative (.finite negative bits) = true ↔ 0 ≤ (if negative then -magnitude bits else magnitude bits) := by
  simp [nonnegative, isFinite, le, zero]

theorem finite_at_most_iff (negative : Bool) (bits : PositiveBits) (maximum : Nat) :
    atMost (.finite negative bits) maximum = true ↔
      (if negative then -magnitude bits else magnitude bits) ≤ (maximum : ℚ) := by simp [atMost]

theorem rejects_nonfinite : nonnegative .nan = false ∧ nonnegative (.infinity false) = false ∧
    nonnegative (.infinity true) = false ∧ atMost .nan 0 = false ∧ atMost (.infinity false) 0 = false := by decide

theorem exponent_bound (bits : PositiveBits) : exponent bits < 2047 := by
  exact (Nat.div_lt_iff_lt_mul (by positivity)).mpr bits.isLt

theorem fraction_bound (bits : PositiveBits) : fraction bits < 2 ^ 52 := Nat.mod_lt _ (by positivity)

theorem decoded_fields (bits : PositiveBits) : exponent bits * 2 ^ 52 + fraction bits = bits.val := by
  simpa [exponent, fraction, Nat.mul_comm] using Nat.div_add_mod bits.val (2 ^ 52)

theorem ties_even (rational : ℚ) (other : PositiveBits)
    (tied : abs (magnitude (closest rational) - |rational|) = abs (magnitude other - |rational|))
    (even : other.val % 2 = 0) : (closest rational).val % 2 = 0 := by
  by_contra odd
  have parity : other.val % 2 < (closest rational).val % 2 := by omega
  have keySmaller : roundingKey rational other < roundingKey rational (closest rational) :=
    Prod.Lex.lt_iff.mpr (Or.inr ⟨tied.symm, Prod.Lex.lt_iff.mpr (Or.inl parity)⟩)
  exact (not_lt_of_ge (closest_minimizes rational other)) keySmaller

end Allium.Binary64
