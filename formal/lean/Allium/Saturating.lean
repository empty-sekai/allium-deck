import Allium.PowerModel

/-!
# Saturating unsigned arithmetic

Saturation is a minimum with the storage limit. Nested saturated additions
collapse to one final clipping operation for non-negative operands. An upper
bound may be clipped only after proving the legal value fits that limit;
a clipped lower bound needs no such assumption.
-/
namespace Allium.Saturating

abbrev u32Max : Nat := 2 ^ 32 - 1

def clip (limit value : Nat) : Nat := min value limit

def add (limit left right : Nat) : Nat := clip limit (left + right)

def mul (limit left right : Nat) : Nat := clip limit (left * right)

theorem clip_add_left (limit left right : Nat) :
    clip limit (clip limit left + right) = clip limit (left + right) := by
  unfold clip
  omega

theorem clip_add_right (limit left right : Nat) :
    clip limit (left + clip limit right) = clip limit (left + right) := by
  unfold clip
  omega

theorem clip_add_both (limit left right : Nat) :
    add limit (clip limit left) (clip limit right) = clip limit (left + right) := by
  unfold add
  rw [clip_add_left, clip_add_right]

theorem add_assoc (limit a b c : Nat) : add limit (add limit a b) c = add limit a (add limit b c) := by
  unfold add
  rw [clip_add_left, clip_add_right]
  congr 1
  omega

theorem fold_sum (limit : Nat) (values : List Nat) (initial : Nat) (fits : initial ≤ limit) :
    values.foldl (add limit) initial = clip limit (initial + values.sum) := by
  induction values generalizing initial with
  | nil => simp [clip, min_eq_left fits]
  | cons first rest ih =>
      simp only [List.foldl_cons, List.sum_cons]
      rw [ih (add limit initial first) (min_le_right _ _)]
      unfold add
      rw [clip_add_left]
      congr 1
      omega

def sum (limit : Nat) (values : List Nat) : Nat := values.foldl (add limit) 0

theorem sum_eq (limit : Nat) (values : List Nat) : sum limit values = clip limit values.sum := by
  simpa only [sum, Nat.zero_add] using fold_sum limit values 0 (Nat.zero_le _)

theorem sum_fits (limit : Nat) (values : List Nat) : sum limit values ≤ limit := by
  rw [sum_eq]
  exact min_le_right _ _

theorem upper_preserved (limit value bound : Nat) (upper : value ≤ bound) (fits : value ≤ limit) :
    value ≤ clip limit bound := le_min upper fits

theorem lower_preserved (limit value bound : Nat) (lower : bound ≤ value) :
    clip limit bound ≤ value := (min_le_left _ _).trans lower

/-- The accepted aggregate-power domain lies strictly inside u32 storage. -/
theorem power_domain_fits : (2 ^ 24 : Nat) ≤ u32Max := by decide

end Allium.Saturating
