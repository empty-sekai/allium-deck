import Mathlib.Data.Real.Basic
import Mathlib.Tactic

/-!
# Correlated and joint power/skill bounds

All inequalities are proved over exact reals. Positivity guards are explicit;
conversion to integer/rational machine bounds uses Arithmetic.lean separately.
No optimization oracle or numerical solver is trusted.

Source: correlated.rs; solver/bonus_tiers.rs JointCeiling::live.
-/
namespace Allium.Quadratic

/-- The product maximum under a non-negative sum constraint. -/
theorem product_under_sum (u v c : ℝ) (hu : 0 ≤ u) (hv : 0 ≤ v)
    (h : u + v ≤ c) : 4 * u * v ≤ c ^ 2 := by
  have hc : 0 ≤ c := le_trans (add_nonneg hu hv) h
  have hp := mul_nonneg (sub_nonneg.mpr h) (add_nonneg hc (add_nonneg hu hv))
  nlinarith [sq_nonneg (u - v)]

/-- A clipped vertex: if U is before the parabola's peak, U is optimal. -/
theorem product_left_cap (u v cap c : ℝ) (hu : 0 ≤ u)
    (hcap : u ≤ cap) (hsum : u + v ≤ c) (hpeak : 2 * cap ≤ c) :
    u * v ≤ cap * (c - cap) := by
  have hp := mul_nonneg (sub_nonneg.mpr hcap)
    (show 0 ≤ c - cap - u by linarith)
  have hq := mul_nonneg hu (show 0 ≤ c - u - v by linarith)
  nlinarith

/-- The monotone boundary case used by the correlated live-score bound. -/
theorem product_boundary (u v intercept total : ℝ)
    (hu : 0 ≤ u) (hv : 0 ≤ v) (hsum : u + v ≤ total)
    (hintercept : total ≤ intercept) :
    u * (intercept + v) ≤ intercept * total := by
  have huT : u ≤ total := by linarith
  have huI : u ≤ intercept := huT.trans hintercept
  have hp := mul_nonneg (sub_nonneg.mpr huI) (sub_nonneg.mpr huT)
  have hq := mul_nonneg hu (show 0 ≤ total - u - v by linarith)
  nlinarith

/-- A correlated linear plane implies the full quadratic envelope. -/
theorem correlated_vertex (power skill intercept weight rate scale total : ℝ)
    (hp : 0 ≤ power) (hs : 0 ≤ skill) (hc : 0 ≤ intercept)
    (hw : 0 < weight) (hr : 0 < rate) (hq : 0 < scale)
    (hplane : weight * power + rate * skill ≤ total) :
    4 * power * (intercept + skill) / scale ≤
      (rate * intercept + total) ^ 2 / (rate * weight * scale) := by
  have hsum : weight * power + rate * (intercept + skill) ≤
      rate * intercept + total := by nlinarith
  have hproduct := product_under_sum (weight * power) (rate * (intercept + skill))
    (rate * intercept + total) (mul_nonneg hw.le hp)
    (mul_nonneg hr.le (add_nonneg hc hs)) hsum
  calc
    4 * power * (intercept + skill) / scale =
        (4 * (weight * power) * (rate * (intercept + skill))) /
          (rate * weight * scale) := by field_simp
    _ ≤ (rate * intercept + total) ^ 2 / (rate * weight * scale) :=
      div_le_div_of_nonneg_right hproduct (by positivity)

/-- The second branch of correlated.rs's quadratic envelope. -/
theorem correlated_boundary (power skill intercept weight rate scale total : ℝ)
    (hp : 0 ≤ power) (hs : 0 ≤ skill)
    (hw : 0 < weight) (hr : 0 < rate) (hq : 0 < scale)
    (hplane : weight * power + rate * skill ≤ total)
    (hbranch : total ≤ rate * intercept) :
    4 * power * (intercept + skill) / scale ≤
      4 * intercept * total / (weight * scale) := by
  have hproduct := product_boundary (weight * power) (rate * skill)
    (rate * intercept) total (mul_nonneg hw.le hp) (mul_nonneg hr.le hs) hplane hbranch
  have hscaled : 4 * (weight * power) * (rate * intercept + rate * skill) ≤
      4 * (rate * intercept) * total := by nlinarith
  calc
    4 * power * (intercept + skill) / scale =
        (4 * (weight * power) * (rate * intercept + rate * skill)) /
          (rate * weight * scale) := by field_simp
    _ ≤ (4 * (rate * intercept) * total) / (rate * weight * scale) :=
      div_le_div_of_nonneg_right hscaled (by positivity)
    _ = 4 * intercept * total / (weight * scale) := by field_simp

/-- The same branch selection as the correlated mathematical algorithm.
Its heuristic plane weights influence tightness, never validity. -/
noncomputable def correlatedBound (intercept weight rate scale total : ℝ) : ℝ :=
  if rate * intercept + total < 2 * total then
    (rate * intercept + total) ^ 2 / (rate * weight * scale)
  else 4 * intercept * total / (weight * scale)

theorem correlated_bound_sound (power skill intercept weight rate scale total : ℝ)
    (hp : 0 ≤ power) (hs : 0 ≤ skill) (hc : 0 ≤ intercept)
    (hw : 0 < weight) (hr : 0 < rate) (hq : 0 < scale)
    (hplane : weight * power + rate * skill ≤ total) :
    4 * power * (intercept + skill) / scale ≤
      correlatedBound intercept weight rate scale total := by
  unfold correlatedBound
  split_ifs with h
  · exact correlated_vertex power skill intercept weight rate scale total hp hs hc hw hr hq hplane
  · exact correlated_boundary power skill intercept weight rate scale total hp hs hw hr hq hplane
      (by linarith)

/-- Product ceiling in a box intersected with a half-plane. -/
noncomputable def jointPeak (upperPower upperRate alpha beta total : ℝ) : ℝ :=
  if alpha * upperPower + beta * upperRate ≤ total then upperPower * upperRate
  else if 2 * alpha * upperPower ≤ total then
    upperPower * (total - alpha * upperPower) / beta
  else if 2 * beta * upperRate ≤ total then
    upperRate * (total - beta * upperRate) / alpha
  else total ^ 2 / (4 * alpha * beta)

/-- All four branches of the joint ceiling dominate every feasible product. -/
theorem joint_peak_sound (power rate upperPower upperRate alpha beta total : ℝ)
    (hp : 0 ≤ power) (hr : 0 ≤ rate)
    (hP : power ≤ upperPower) (hR : rate ≤ upperRate)
    (ha : 0 < alpha) (hb : 0 < beta)
    (hplane : alpha * power + beta * rate ≤ total) :
    power * rate ≤ jointPeak upperPower upperRate alpha beta total := by
  unfold jointPeak
  split_ifs with hcorner hleft hright
  · exact mul_le_mul hP hR hr (hp.trans hP)
  · have hu : alpha * power ≤ alpha * upperPower := mul_le_mul_of_nonneg_left hP ha.le
    have hprod := product_left_cap (alpha * power) (beta * rate)
      (alpha * upperPower) total (mul_nonneg ha.le hp) hu hplane (by nlinarith)
    calc
      power * rate = (alpha * power * (beta * rate)) / (alpha * beta) := by field_simp
      _ ≤ (alpha * upperPower * (total - alpha * upperPower)) / (alpha * beta) :=
        div_le_div_of_nonneg_right hprod (by positivity)
      _ = upperPower * (total - alpha * upperPower) / beta := by field_simp
  · have hv : beta * rate ≤ beta * upperRate := mul_le_mul_of_nonneg_left hR hb.le
    have hprod := product_left_cap (beta * rate) (alpha * power)
      (beta * upperRate) total (mul_nonneg hb.le hr) hv (by linarith) (by nlinarith)
    calc
      power * rate = (beta * rate * (alpha * power)) / (alpha * beta) := by field_simp
      _ ≤ (beta * upperRate * (total - beta * upperRate)) / (alpha * beta) :=
        div_le_div_of_nonneg_right hprod (by positivity)
      _ = upperRate * (total - beta * upperRate) / alpha := by field_simp
  · have hprod := product_under_sum (alpha * power) (beta * rate) total
      (mul_nonneg ha.le hp) (mul_nonneg hb.le hr) hplane
    calc
      power * rate = (4 * (alpha * power) * (beta * rate)) / (4 * alpha * beta) := by
        field_simp
      _ ≤ total ^ 2 / (4 * alpha * beta) :=
        div_le_div_of_nonneg_right hprod (by positivity)

/-- From selected features and a joint suffix maximum to the plane used above. -/
theorem joint_plane (prePower preRate power skill powerWeight skillWeight skillRate joint : ℝ)
    (hw : 0 ≤ skillRate)
    (hjoint : powerWeight * power + skillWeight * skill ≤ joint) :
    (powerWeight * skillRate) * (prePower + power) +
      skillWeight * (preRate + skillRate * skill) ≤
      skillRate * joint + (powerWeight * skillRate) * prePower + skillWeight * preRate := by
  have h := mul_le_mul_of_nonneg_left hjoint hw
  nlinarith

end Allium.Quadratic
