import Allium.Admission

/-!
# Skill-result production

The slot/Option projection of handler/skill.rs retains every return branch,
the ordered effect scan, scalar capacity failures and the final branch
priority. Lookup tables supply skill levels and effect rows, never prebuilt
SkillResult values. Unrelated full-precision presentation fields are omitted.
-/
namespace Allium.Admission.SkillProducer
open SkillGather

inductive Produced : ConcreteInput → Prop where
  | ordinary (value : Fin 256) : Produced ⟨none, none, none, (0, value)⟩
  | unitCount (value : UnitCountPayload) : Produced ⟨some value, none, none, (1, 0)⟩
  | differentUnit (value : PairPayload) : Produced ⟨none, some value, none, (2, 0)⟩
  | reference (value : PairPayload) : Produced ⟨none, none, some value, (3, 0)⟩

inductive Training where
  | before | after
  deriving DecidableEq

/-- Six accepted source units and the enum values rejected by unit_to_pool_index. -/
inductive SourceUnit where
  | real (index : Fin 6)
  | other
  deriving DecidableEq

def unitIndex : SourceUnit → Option (Fin 256)
  | .real index => some ⟨index.val, by have bound := index.isLt; omega⟩
  | .other => none

inductive EffectKind where
  | scoreUp | lifeRecovery | characterRank | unitCount | differentUnit | reference | other
  deriving DecidableEq

structure Effect where
  kind : EffectKind
  value : Int
  additional : Option Int := none
  rank : Option Int := none
  unit : Option SourceUnit := none
  count : Option Int := none

structure StaticMaximum where
  rank : Option (Int × Int) := none
  unit : Option Int := none
  differentIncrement : Int := 0
  referenceMaximum : Int := 0

structure Scan where
  base : Int := 0
  rankBonus : Int := 0
  unit : Option SourceUnit := none
  unitValues : Fin 5 → Fin 256 := fun _ => 0
  different : Option PairPayload := none
  referenceRate : Int := 0
  referenceMaximum : Int := 0
  staticMaximum : StaticMaximum := {}

/-- Checked conversion, with the caller's actual error field retained. -/
def score8 (field : Field) (value : Int) (limit : Option Nat) : Except Error (Fin 256) :=
  let bounded := scoreValue value limit
  if fits : bounded < 256 then .ok ⟨bounded, fits⟩ else .error (.capacity field bounded 255)

theorem score8_value (field : Field) (value : Int) (limit : Option Nat) (output : Fin 256)
    (accepted : score8 field value limit = .ok output) : output.val = scoreValue value limit := by
  unfold score8 at accepted
  dsimp only at accepted
  split at accepted
  · cases Except.ok.inj accepted
    rfl
  · cases accepted

def staticValue (base : Int) (parts : StaticMaximum) : Nat :=
  base.toNat + ((parts.rank.map Prod.snd).getD 0).toNat +
    ((parts.unit.map (fun value => value - max base 0)).getD 0).toNat +
    2 * parts.differentIncrement.toNat + parts.referenceMaximum.toNat

def reference16 (base : Int) (parts : StaticMaximum) : Except Error (Fin 65536) :=
  let value := staticValue base parts
  if fits : value < 65536 then .ok ⟨value, fits⟩ else .error (.capacity .skillReferenceValue value 65535)

def rankRow (parts : StaticMaximum) (rank value : Int) : StaticMaximum :=
  { parts with rank := if parts.rank.isNone ∨ (parts.rank.getD (0, 0)).1 < rank then some (rank, value) else parts.rank }

def unitRow (parts : StaticMaximum) (value : Int) : StaticMaximum :=
  { parts with unit := some (match parts.unit with | none => value | some previous => max previous value) }

/-- Only the five counted unit rows are narrowed; out-of-range counts do not
write the array. A later unit row still replaces the optional target unit. -/
def scanStep (rank : Int) (limit : Option Nat) (state : Scan) (effect : Effect) : Except Error Scan :=
  match effect.kind with
  | .scoreUp => .ok { state with base := max state.base effect.value }
  | .characterRank =>
      match effect.rank with
      | none => .ok state
      | some threshold => .ok { state with
          rankBonus := if threshold ≤ rank then max state.rankBonus effect.value else state.rankBonus
          staticMaximum := rankRow state.staticMaximum threshold effect.value }
  | .unitCount =>
      let next := { state with unit := effect.unit, staticMaximum := unitRow state.staticMaximum effect.value }
      match effect.count with
      | none => .ok next
      | some count =>
          if valid : 1 ≤ count ∧ count ≤ 5 then
            match score8 .skillScore effect.value limit with
            | .error error => .error error
            | .ok value => .ok { next with unitValues := Function.update next.unitValues ⟨(count - 1).toNat, by omega⟩ value }
          else .ok next
  | .differentUnit =>
      match score8 .skillScore effect.value limit with
      | .error error => .error error
      | .ok base => match score8 .differentUnitIncrement (effect.additional.getD 0) limit with
          | .error error => .error error
          | .ok increment =>
              .ok { state with
                different := some (base, increment)
                staticMaximum := { state.staticMaximum with differentIncrement := effect.additional.getD 0 } }
  | .reference =>
      .ok { state with
        referenceRate := effect.value
        referenceMaximum := effect.additional.getD 0
        staticMaximum := { state.staticMaximum with referenceMaximum := effect.additional.getD 0 } }
  | .lifeRecovery | .other => .ok state

def scan (rank : Int) (limit : Option Nat) : Scan → List Effect → Except Error Scan
  | state, [] => .ok state
  | state, effect :: rest => match scanStep rank limit state effect with
      | .error error => .error error
      | .ok next => scan rank limit next rest

structure Output where
  skill : ConcreteInput
  minimum : Fin 256
  maximum : Fin 256
  referenceValue : Fin 65536
  skillId : Int
  hasReference : Bool

def empty : Output := ⟨⟨none, none, none, (0, 0)⟩, 0, 0, 0, 0, false⟩

def ordinary (id : Int) (hasReference : Bool) (base : Fin 256) (referenceValue : Fin 65536) : Output :=
  ⟨⟨none, none, none, (0, base)⟩, base, base, referenceValue, id, hasReference⟩

def unitResult (id : Int) (hasReference : Bool) (base unit : Fin 256)
    (values : Fin 5 → Fin 256) (referenceValue : Fin 65536) : Output :=
  let filled := fun index => if values index = 0 then base else values index
  ⟨⟨some (unit, filled), none, none, (1, 0)⟩,
    (List.ofFn filled).foldl min 255, (List.ofFn filled).foldl max 0, referenceValue, id, hasReference⟩

def differentResult (id : Int) (hasReference : Bool) (value : PairPayload)
    (maximum : Fin 256) (referenceValue : Fin 65536) : Output :=
  ⟨⟨none, some value, none, (2, 0)⟩, value.1, maximum, referenceValue, id, hasReference⟩

def referenceResult (id : Int) (base rate addition maximum : Fin 256) (referenceValue : Fin 65536) : Output :=
  ⟨⟨none, none, some (rate, addition), (3, 0)⟩, base, maximum, referenceValue, id, true⟩

/-- The return branches start from a fresh default SkillResult. Accumulator
Options from a lower-priority branch do not leak into the returned record. -/
def finish (id : Int) (limit : Option Nat) (state : Scan) (base : Fin 256)
    (referenceValue : Fin 65536) : Except Error Output :=
  let hasReference := decide (0 < state.referenceRate ∧ 0 < state.referenceMaximum)
  match state.unit.bind unitIndex with
  | some unit => .ok (unitResult id hasReference base unit state.unitValues referenceValue)
  | none => match state.different with
      | some value =>
          match score8 .differentUnitUpper (value.1.val + value.2.val * 2 : Nat) limit with
          | .error error => .error error
          | .ok maximum => .ok (differentResult id hasReference value maximum referenceValue)
      | none =>
          if 0 < state.referenceRate ∧ 0 < state.referenceMaximum then
            match score8 .referenceAddition state.referenceMaximum (limit.map (fun cap => cap - base.val)) with
            | .error error => .error error
            | .ok addition => match score8 .referenceRate state.referenceRate none with
                | .error error => .error error
                | .ok rate => match score8 .referenceUpper (base.val + addition.val : Nat) none with
                    | .error error => .error error
                    | .ok maximum => .ok (referenceResult id base rate addition maximum referenceValue)
          else .ok (ordinary id hasReference base referenceValue)

structure Tables where
  skillLevel : Int → Int → Option Int
  effects : Int → Int → List Effect

structure Request where
  originalSkill : Int
  trainedSkill : Option Int
  level : Int
  characterRank : Int
  limit : Option Nat

def chosenId (request : Request) : Training → Int
  | .before => request.originalSkill
  | .after => request.trainedSkill.getD request.originalSkill

/-- Skill and effect lookup results are data; no lookup can inject a raw slot. -/
def buildSkill (tables : Tables) (request : Request) (training : Training) : Except Error Output :=
  let id := chosenId request training
  match tables.skillLevel id request.level with
  | none => .ok empty
  | some level => match scan request.characterRank request.limit {} (tables.effects id level) with
      | .error error => .error error
      | .ok state => match reference16 state.base state.staticMaximum with
          | .error error => .error error
          | .ok referenceValue => match score8 .baseSkillScore (state.base + state.rankBonus) request.limit with
              | .error error => .error error
              | .ok base => finish id request.limit state base referenceValue

theorem finish_produced (id : Int) (limit : Option Nat) (state : Scan) (base : Fin 256)
    (referenceValue : Fin 65536) (output : Output)
    (accepted : finish id limit state base referenceValue = .ok output) : Produced output.skill := by
  cases unitChoice : state.unit.bind unitIndex with
  | some unit =>
      have same := accepted
      simp only [finish, unitChoice, Except.ok.injEq] at same
      subst output
      exact Produced.unitCount _
  | none =>
      cases differentChoice : state.different with
      | some value =>
          cases checked : score8 .differentUnitUpper (value.1.val + value.2.val * 2 : Nat) limit with
          | error error =>
              simp only [finish, unitChoice, differentChoice, checked] at accepted
              cases accepted
          | ok maximum =>
              have same := accepted
              simp only [finish, unitChoice, differentChoice, checked, Except.ok.injEq] at same
              subst output
              exact Produced.differentUnit value
      | none =>
          by_cases reference : 0 < state.referenceRate ∧ 0 < state.referenceMaximum
          · cases added : score8 .referenceAddition state.referenceMaximum (limit.map (fun cap => cap - base.val)) with
            | error error => simp [finish, unitChoice, differentChoice, reference, added] at accepted
            | ok addition =>
                cases rated : score8 .referenceRate state.referenceRate none with
                | error error => simp [finish, unitChoice, differentChoice, reference, added, rated] at accepted
                | ok rate =>
                    cases bounded : score8 .referenceUpper (base.val + addition.val : Nat) none with
                    | error error =>
                        simp only [finish, unitChoice, differentChoice, if_pos reference, added, rated, bounded] at accepted
                        cases accepted
                    | ok maximum =>
                        have same := accepted
                        simp only [finish, unitChoice, differentChoice, if_pos reference, added, rated, bounded,
                          Except.ok.injEq] at same
                        subst output
                        exact Produced.reference (rate, addition)
          · have same := accepted
            simp only [finish, unitChoice, differentChoice, if_neg reference, Except.ok.injEq] at same
            subst output
            exact Produced.ordinary base

theorem build_skill_produced (tables : Tables) (request : Request) (training : Training) (output : Output)
    (accepted : buildSkill tables request training = .ok output) : Produced output.skill := by
  cases lookup : tables.skillLevel (chosenId request training) request.level with
  | none =>
      have same : empty = output := by simpa only [buildSkill, lookup, Except.ok.injEq] using accepted
      subst output
      exact Produced.ordinary 0
  | some level =>
      cases scanned : scan request.characterRank request.limit {} (tables.effects (chosenId request training) level) with
      | error error => simp [buildSkill, lookup, scanned] at accepted
      | ok state =>
          cases reference : reference16 state.base state.staticMaximum with
          | error error => simp [buildSkill, lookup, scanned, reference] at accepted
          | ok referenceValue =>
              cases base : score8 .baseSkillScore (state.base + state.rankBonus) request.limit with
              | error error => simp [buildSkill, lookup, scanned, reference, base] at accepted
              | ok value =>
                  apply finish_produced _ _ _ _ _ output
                  simpa only [buildSkill, lookup, scanned, reference, base] using accepted

end Allium.Admission.SkillProducer
