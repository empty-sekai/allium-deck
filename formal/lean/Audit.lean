import Allium
import Lean.Util.CollectAxioms

/-!
Audit the TRANSITIVE axiom dependencies of every declaration in the Allium
namespace, not merely a hand-picked list of headline theorems. This also checks
definitions, so hiding a new axiom in an opaque helper does not evade the gate.
The three permitted axioms are Lean's standard propositional extensionality,
classical choice, and quotient soundness. Model premises remain visible in
theorem types: passing this audit does NOT discharge an unproved premise.
-/

open Lean Elab Command in
run_cmd do
  let env ← getEnv
  let allowed : List Name := [`propext, `Classical.choice, `Quot.sound]
  let mut declarations := 0
  let mut theorems := 0
  for (name, info) in env.constants.toList do
    if (`Allium).isPrefixOf name then
      declarations := declarations + 1
      match info with
      | .thmInfo _ => theorems := theorems + 1
      | _ => pure ()
      let axioms ← collectAxioms name
      for axiomName in axioms do
        unless allowed.contains axiomName do
          throwError "AXIOM AUDIT FAILED: {name} depends on forbidden axiom {axiomName}"
  if declarations == 0 || theorems == 0 then
    throwError "AXIOM AUDIT FAILED: no Allium declarations/theorems were imported"
  logInfo m!"AXIOM AUDIT PASSED: {declarations} declarations; {theorems} theorems; allowlist={allowed}"

#print axioms Allium.coverage_exactness
#print axioms Allium.topK_card
#print axioms Allium.collect_exact
#print axioms Allium.group_topK_merge
#print axioms Allium.search_exact
#print axioms Allium.complete_exact
#print axioms Allium.FiniteBounds.character_bound_sound
#print axioms Allium.DP.compressed_reachable_covers
#print axioms Allium.Skill.reference_relax_sound
#print axioms Allium.Quadratic.joint_peak_sound
#print axioms Allium.Arithmetic.grid_floor_of_error
