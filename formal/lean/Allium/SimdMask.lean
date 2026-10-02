import Allium.Arithmetic

/-!
# Sixteen-lane unsigned threshold masks

The scalar loop sets bit i for unsigned upper[i] >= threshold. The vector
model compares two eight-lane halves and concatenates their masks. Tail,
used-character, monotone-break and refreshed-threshold masks are intersections.
-/
namespace Allium.SimdMask

/-- Little-endian predicate bitmap, with no bits outside the given width. -/
def encode : (width : Nat) → (Nat → Bool) → BitVec width
  | 0, _ => 0
  | width + 1, predicate => BitVec.cons (predicate width) (encode width predicate)

@[simp] theorem encode_bit (width : Nat) (predicate : Nat → Bool) (lane : Nat) :
    (encode width predicate).getLsbD lane = (decide (lane < width) && predicate lane) := by
  induction width with
  | zero => simp [encode]
  | succ width ih =>
      rw [encode, BitVec.getLsbD_cons]
      by_cases he : lane = width
      · subst lane
        simp
      · rw [if_neg he, ih]
        have hl : lane < width + 1 ↔ lane < width := by omega
        simp [hl]

def scalarMask (upper : Nat → BitVec 64) (threshold : BitVec 64) : BitVec 16 :=
  encode 16 (fun lane => decide (threshold.toNat ≤ (upper lane).toNat))

def vectorMask (upper : Nat → BitVec 64) (threshold : BitVec 64) : BitVec 16 :=
  encode 8 (fun lane => threshold.ule (upper (lane + 8))) ++
    encode 8 (fun lane => threshold.ule (upper lane))

theorem unsigned_halves_eq_scalar (upper : Nat → BitVec 64) (threshold : BitVec 64) :
    vectorMask upper threshold = scalarMask upper threshold := by
  apply BitVec.eq_of_getLsbD_eq
  intro lane hl
  simp only [vectorMask, scalarMask, BitVec.getLsbD_append, encode_bit, BitVec.ule_eq_decide]
  by_cases hhalf : lane < 8
  · simp [hhalf, hl]
  · have hsub : lane - 8 < 8 := by omega
    have hback : lane - 8 + 8 = lane := by omega
    simp [hhalf, hsub, hback, hl]

def legalMask (length : Nat) (character : Nat → Fin 27) (used : Finset (Fin 27)) : BitVec 16 :=
  encode 16 (fun lane => decide (lane < length ∧ character lane ∉ used))

def surviving (length : Nat) (character : Nat → Fin 27) (used : Finset (Fin 27))
    (upper : Nat → BitVec 64) (threshold : BitVec 64) : BitVec 16 :=
  legalMask length character used &&& vectorMask upper threshold

/-- Exact lane/tail semantics, including equality with the threshold. -/
theorem surviving_iff (length : Nat) (character : Nat → Fin 27) (used : Finset (Fin 27))
    (upper : Nat → BitVec 64) (threshold : BitVec 64) (lane : Nat) :
    (surviving length character used upper threshold).getLsbD lane = true ↔
      lane < 16 ∧ lane < length ∧ character lane ∉ used ∧
        threshold.toNat ≤ (upper lane).toNat := by
  rw [surviving, unsigned_halves_eq_scalar]
  simp only [BitVec.getLsbD_and, legalMask, scalarMask, encode_bit,
    Bool.and_eq_true, decide_eq_true_eq]
  tauto

theorem equal_threshold_retained (length : Nat) (character : Nat → Fin 27)
    (used : Finset (Fin 27)) (upper : Nat → BitVec 64) (threshold : BitVec 64)
    (lane : Nat) (hw : lane < 16) (hl : lane < length)
    (hc : character lane ∉ used) (he : upper lane = threshold) :
    (surviving length character used upper threshold).getLsbD lane = true := by
  apply (surviving_iff length character used upper threshold lane).mpr
  exact ⟨hw, hl, hc, by rw [he]⟩

/-- Increasing the incumbent cannot introduce a lane that was previously cut. -/
theorem threshold_refresh (upper : Nat → BitVec 64) (old fresh : BitVec 64)
    (h : old.toNat ≤ fresh.toNat) :
    scalarMask upper old &&& scalarMask upper fresh = scalarMask upper fresh := by
  apply BitVec.eq_of_getLsbD_eq
  intro lane hl
  simp only [BitVec.getLsbD_and, scalarMask, encode_bit, hl, decide_true, Bool.true_and]
  by_cases hf : fresh.toNat ≤ (upper lane).toNat
  · have ho := h.trans hf
    simp [ho, hf]
  · simp [hf]

/-- A block shortened by a dense break contains exactly its earlier lanes. -/
theorem break_mask_iff (mask : BitVec 16) (stop lane : Nat) :
    (mask &&& encode 16 (fun i => decide (i < stop))).getLsbD lane = true ↔
      mask.getLsbD lane = true ∧ lane < 16 ∧ lane < stop := by
  simp [BitVec.getLsbD_and]

end Allium.SimdMask
