import Allium.Arithmetic
import Allium.Binary64
import Mathlib.Data.List.Lex
import Mathlib.Data.List.Sort

/-!
# Checked scalar representation boundaries

Representable IDs are ordinary values, including zero and 65535. Range errors
are returned before narrowing; a rejected value is not clipped into a search
candidate. Score components alone apply the documented non-negative clamp and
optional cap before the checked eight-bit conversion.
-/
namespace Allium.Admission

inductive Field where
  | publicId
  | characterId
  | skill
  | unitCountSkills
  | differentUnitSkills
  | referenceSkills
  | skillScore
  | baseSkillScore
  | skillReferenceValue
  | differentUnitIncrement
  | differentUnitUpper
  | referenceAddition
  | referenceRate
  | attributeId
  | unitMask
  | unitProfiles
  | power
  | eventBonus
  | referenceUpper
  | limitedValues
  | baseScore
  | autoBaseScore
  | feverScore
  | skillScoreRate
  | supportBonus
  | deckPower
  | bonusCeiling
  | liveRate
  | liveScore
  | eventInner
  | eventPoint
  deriving DecidableEq, Repr

inductive Error where
  | negative (field : Field)
  | capacity (field : Field) (value maximum : Nat)
  | tooManyCards (count : Nat)
  | invalidNumber (field : Field)
  deriving DecidableEq, Repr

def checked (field : Field) (value maximum : Nat) : Except Error Nat :=
  if value > maximum then .error (.capacity field value maximum) else .ok value

@[simp] theorem checked_ok_iff (field : Field) (value maximum result : Nat) :
    checked field value maximum = .ok result ↔ value ≤ maximum ∧ result = value := by
  unfold checked
  by_cases h : value > maximum
  · simp [h]
  · simp [h]; omega

theorem checked_rejects (field : Field) (value maximum : Nat) (h : maximum < value) :
    checked field value maximum = .error (.capacity field value maximum) := by simp [checked, h]

def identity (field : Field) (maximum : Nat) (value : Int) : Except Error Nat :=
  if value < 0 then .error (.negative field) else checked field value.toNat maximum

@[simp] theorem identity_ok_iff (field : Field) (maximum result : Nat) (value : Int) :
    identity field maximum value = .ok result ↔
      0 ≤ value ∧ value ≤ (maximum : Int) ∧ (result : Int) = value := by
  unfold identity
  by_cases h : value < 0
  · simp [h]
  · rw [if_neg h, checked_ok_iff]
    omega

def publicId (value : Int) : Except Error Nat := identity .publicId 65535 value

def characterId (value : Int) : Except Error Nat := identity .characterId 26 value

theorem public_id_range (value : Int) (result : Nat) (h : publicId value = .ok result) :
    result ≤ 65535 ∧ (result : Int) = value := by
  rcases (identity_ok_iff .publicId 65535 result value).mp h with ⟨_, hmax, he⟩
  constructor
  · omega
  · exact he

theorem character_id_range (value : Int) (result : Nat) (h : characterId value = .ok result) :
    result ≤ 26 ∧ (result : Int) = value := by
  rcases (identity_ok_iff .characterId 26 result value).mp h with ⟨_, hmax, he⟩
  constructor
  · omega
  · exact he

/-- Narrowing is lossless on every accepted identity. -/
theorem accepted_public_id_fin (value : Int) (result : Nat) (h : publicId value = .ok result) :
    ∃ encoded : Fin 65536, encoded.val = result ∧ (encoded.val : Int) = value := by
  rcases public_id_range value result h with ⟨hr, he⟩
  exact ⟨⟨result, by omega⟩, rfl, he⟩

def scoreValue (value : Int) (limit : Option Nat) : Nat :=
  match limit with
  | none => value.toNat
  | some cap => min value.toNat cap

def score (value : Int) (limit : Option Nat) : Except Error Nat :=
  checked .skill (scoreValue value limit) 255

@[simp] theorem score_ok_iff (value : Int) (limit : Option Nat) (result : Nat) :
    score value limit = .ok result ↔ scoreValue value limit ≤ 255 ∧ result = scoreValue value limit :=
  checked_ok_iff _ _ _ _

theorem score_never_saturates (value : Int) (limit : Option Nat)
    (overflow : 255 < scoreValue value limit) :
    score value limit = .error (.capacity .skill (scoreValue value limit) 255) :=
  checked_rejects _ _ _ overflow

theorem negative_score_zero (value : Int) (limit : Option Nat) (negative : value ≤ 0) :
    score value limit = .ok 0 := by
  have hz : value.toNat = 0 := by omega
  cases limit <;> simp [score, scoreValue, hz, checked]

/-- IDs at the endpoints do not acquire sentinel meanings. -/
theorem identity_boundaries : publicId 0 = .ok 0 ∧ publicId 65535 = .ok 65535 ∧
    publicId 65536 = .error (.capacity .publicId 65536 65535) ∧
    publicId (-1) = .error (.negative .publicId) ∧ characterId 26 = .ok 26 ∧
    characterId 27 = .error (.capacity .characterId 27 26) := by
  norm_num [publicId, characterId, identity, checked]
  decide

namespace Intern

variable {Value : Type*} [DecidableEq Value]

/-- Stable first-content match, mirroring the table's position scan. -/
def firstIndex (value : Value) : List Value → Option Nat
  | [] => none
  | first :: rest =>
      if first = value then some 0 else (firstIndex value rest).map (· + 1)

@[simp] theorem first_index_none (value : Value) (values : List Value) :
    firstIndex value values = none ↔ value ∉ values := by
  induction values with
  | nil => simp [firstIndex]
  | cons first rest ih =>
      by_cases same : first = value
      · subst first
        simp [firstIndex]
      · have different : value ≠ first := Ne.symm same
        simp [firstIndex, same, different, ih]

theorem first_index_lookup (value : Value) (values : List Value) (index : Nat)
    (found : firstIndex value values = some index) :
    index < values.length ∧ values[index]? = some value := by
  induction values generalizing index with
  | nil => simp [firstIndex] at found
  | cons first rest ih =>
      by_cases same : first = value
      · have zero : index = 0 := by simpa [firstIndex, same] using found.symm
        subst index
        simp [same]
      · cases tail : firstIndex value rest with
        | none => simp [firstIndex, same, tail] at found
        | some previous =>
            have next : previous + 1 = index := by simpa [firstIndex, same, tail] using found
            subst index
            obtain ⟨bound, lookup⟩ := ih previous tail
            exact ⟨Nat.succ_lt_succ bound, by simpa using lookup⟩

structure Result (Value : Type*) where
  values : List Value
  index : Nat
  fresh : Bool

/-- Zero remains the absent-entry sentinel. Existing content does not consume
a slot, including when all 255 nonzero indices are already occupied. -/
def intern (values : List Value) (value : Value) : Except Error (Result Value) :=
  match firstIndex value values with
  | some index => .ok ⟨values, index + 1, false⟩
  | none =>
      if 255 < values.length + 1 then .error (.capacity .skill (values.length + 1) 255)
      else .ok ⟨values ++ [value], values.length + 1, true⟩

theorem duplicate_reuses_index (values : List Value) (value : Value) (index : Nat)
    (found : firstIndex value values = some index) :
    intern values value = .ok ⟨values, index + 1, false⟩ := by
  simp only [intern, found]

theorem fresh_overflow_rejected (values : List Value) (value : Value)
    (fresh : value ∉ values) (full : 255 ≤ values.length) :
    intern values value = .error (.capacity .skill (values.length + 1) 255) := by
  have missing := (first_index_none value values).mpr fresh
  have overflow : 255 < values.length + 1 := by omega
  simp only [intern, missing, if_pos overflow]

/-- Every successful call retains a bounded, distinct table, returns a
nonzero representable index, and that exact index looks up the requested
content. These are loop invariants of repeated interning from the empty list. -/
theorem intern_invariants (values : List Value) (value : Value) (result : Result Value)
    (capacity : values.length ≤ 255) (unique : values.Nodup)
    (accepted : intern values value = .ok result) :
    0 < result.index ∧ result.index ≤ 255 ∧ result.values[result.index - 1]? = some value ∧
      result.values.length ≤ 255 ∧ result.values.Nodup := by
  cases found : firstIndex value values with
  | some index =>
      have same : Result.mk values (index + 1) false = result := by
        simpa only [intern, found, Except.ok.injEq] using accepted
      subst result
      dsimp only
      obtain ⟨bound, lookup⟩ := first_index_lookup value values index found
      exact ⟨by omega, by omega, by simpa using lookup, capacity, unique⟩
  | none =>
      have fresh := (first_index_none value values).mp found
      by_cases overflow : 255 < values.length + 1
      · simp [intern, found, overflow] at accepted
      · have same : Result.mk (values ++ [value]) (values.length + 1) true = result := by
          simpa only [intern, found, if_neg overflow, Except.ok.injEq] using accepted
        subst result
        dsimp only
        refine ⟨by omega, by omega, ?_, ?_, ?_⟩
        · simp
        · simp only [List.length_append, List.length_singleton]
          omega
        · apply List.nodup_append.mpr
          refine ⟨unique, by simp, ?_⟩
          intro old member other singleton same
          have last : other = value := List.mem_singleton.mp singleton
          exact fresh (same.trans last ▸ member)

/-- A sequence is accepted only through the same checked table update; there
is no unchecked prepopulation or narrowing path in this construction. -/
def run : List Value → List Value → Except Error (List Value)
  | values, [] => .ok values
  | values, value :: rest =>
      match intern values value with
      | .error error => .error error
      | .ok result => run result.values rest

theorem run_invariants (values inputs output : List Value) (capacity : values.length ≤ 255)
    (unique : values.Nodup) (accepted : run values inputs = .ok output) :
    output.length ≤ 255 ∧ output.Nodup := by
  induction inputs generalizing values with
  | nil =>
      have same : values = output := by simpa only [run, Except.ok.injEq] using accepted
      subst output
      exact ⟨capacity, unique⟩
  | cons value rest ih =>
      cases call : intern values value with
      | error error => simp [run, call] at accepted
      | ok result =>
          have post := intern_invariants values value result capacity unique call
          apply ih result.values post.2.2.2.1 post.2.2.2.2
          simpa only [run, call] using accepted

theorem from_empty_invariants (inputs output : List Value) (accepted : run [] inputs = .ok output) :
    output.length ≤ 255 ∧ output.Nodup :=
  run_invariants [] inputs output (by simp) (by simp) accepted

/-- The builder appends exactly when the interner reports a new entry. -/
theorem intern_shape (values : List Value) (value : Value) (result : Result Value)
    (accepted : intern values value = .ok result) :
    result.values = if result.fresh then values ++ [value] else values := by
  cases found : firstIndex value values with
  | some index =>
      have same : Result.mk values (index + 1) false = result := by
        simpa only [intern, found, Except.ok.injEq] using accepted
      subst result
      rfl
  | none =>
      by_cases overflow : 255 < values.length + 1
      · simp [intern, found, overflow] at accepted
      · have same : Result.mk (values ++ [value]) (values.length + 1) true = result := by
          simpa only [intern, found, if_neg overflow, Except.ok.injEq] using accepted
        subst result
        rfl

theorem intern_lookup_stable (values : List Value) (value : Value) (result : Result Value)
    (accepted : intern values value = .ok result) (index : Nat) (bound : index < values.length) :
    result.values[index]? = values[index]? := by
  rw [intern_shape values value result accepted]
  split
  next => exact List.getElem?_append_left bound
  next => rfl

/-- Earlier one-based skill references remain valid after all later inserts,
including arbitrary repeated content in the rest of the card stream. -/
theorem run_lookup_stable (values inputs output : List Value)
    (accepted : run values inputs = .ok output) (index : Nat) (bound : index < values.length) :
    output[index]? = values[index]? := by
  induction inputs generalizing values with
  | nil =>
      have same : values = output := by simpa only [run, Except.ok.injEq] using accepted
      subst output
      rfl
  | cons value rest ih =>
      cases call : intern values value with
      | error error => simp [run, call] at accepted
      | ok result =>
          have size : values.length ≤ result.values.length := by
            rw [intern_shape values value result call]
            split <;> simp
          have remaining : run result.values rest = .ok output := by simpa only [run, call] using accepted
          exact (ih result.values remaining (bound.trans_le size)).trans
            (intern_lookup_stable values value result call index bound)

end Intern

namespace CardTable

/-- Fields read by validate_cards after card evaluation and before compact
pool construction. Detail powers still have their signed source values. -/
structure Core where
  publicId : Int
  character : Nat
  attr : Nat
  unitMask : Nat
  power : Fin 6 → Fin 4 → Int
  baseBonus : Nat
  limitedBonus : Nat
  skillMin : Fin 256
  skillMax : Fin 256

structure Input extends Core where
  referenceMax : Option (Fin 256)

def population (mask : Nat) : Nat :=
  ((Finset.range 8).filter (fun bit => mask.testBit bit = true)).card

/-- The exact finite checks performed for one card. In particular, zero unit
masks are accepted here; nonempty membership cannot be inferred from capacity
validation. -/
def Domain (card : Input) : Prop :=
  0 ≤ card.publicId ∧ card.publicId ≤ 65535 ∧ card.character ≤ 26 ∧ card.attr ≤ 4 ∧
  card.unitMask ≤ 63 ∧ population card.unitMask ≤ 2 ∧
  (∀ unit member, (card.power unit member).toNat ≤ 2 ^ 18 - 1) ∧
  card.baseBonus + card.limitedBonus ≤ 4095 ∧
  (match card.referenceMax with
    | none => True
    | some maximum => card.skillMin.val + maximum.val ≤ card.skillMax.val)

instance decidableDomain (card : Input) : Decidable (Domain card) := by
  unfold Domain
  cases card.referenceMax <;> infer_instance

def validCard (card : Input) : Bool := decide (Domain card)

@[simp] theorem valid_card_iff (card : Input) : validCard card = true ↔ Domain card := by
  simp [validCard]

/-- Only a new, nonzero limited-bonus value consumes a four-bit code. -/
def remember (values : List Nat) (value : Nat) : List Nat :=
  if value = 0 ∨ value ∈ values then values else values ++ [value]

theorem remember_unique (values : List Nat) (value : Nat) (unique : values.Nodup) :
    (remember values value).Nodup := by
  unfold remember
  split
  next => exact unique
  next fresh =>
    apply List.nodup_append.mpr
    refine ⟨unique, by simp, ?_⟩
    intro old member other singleton same
    have last : other = value := List.mem_singleton.mp singleton
    exact fresh (Or.inr (same.trans last ▸ member))

@[simp] theorem remember_set (values : List Nat) (value : Nat) :
    (remember values value).toFinset =
      if value = 0 then values.toFinset else insert value values.toFinset := by
  by_cases zero : value = 0
  · simp [remember, zero]
  · by_cases member : value ∈ values
    · simp [remember, zero, member]
    · ext item
      simp [remember, zero, member]

def limitedSet (cards : List Input) : Finset Nat :=
  (cards.map (fun card => card.limitedBonus)).toFinset.erase 0

def collected : List Nat → List Input → List Nat
  | values, [] => values
  | values, card :: rest => collected (remember values card.limitedBonus) rest

theorem collected_unique (values : List Nat) (cards : List Input) (unique : values.Nodup) :
    (collected values cards).Nodup := by
  induction cards generalizing values with
  | nil => exact unique
  | cons card rest ih => exact ih _ (remember_unique values card.limitedBonus unique)

theorem collected_set (values : List Nat) (cards : List Input) :
    (collected values cards).toFinset = values.toFinset ∪ limitedSet cards := by
  induction cards generalizing values with
  | nil => simp [collected, limitedSet]
  | cons card rest ih =>
      rw [collected, ih, remember_set]
      by_cases zero : card.limitedBonus = 0
      · simp [zero, limitedSet]
      · ext item
        by_cases itemZero : item = 0
        · subst item
          have distinct : (0 : Nat) ≠ card.limitedBonus := Ne.symm zero
          simp [zero, limitedSet, distinct]
        · simp [zero, limitedSet, itemZero, or_left_comm]

/-- A failed card check aborts before accepting a completed table. The local
limited-code vector has no observable result on that failure path. -/
def scan : List Nat → List Input → Option (List Nat)
  | values, [] => some values
  | values, card :: rest =>
      if validCard card then scan (remember values card.limitedBonus) rest else none

theorem scan_iff (values : List Nat) (cards : List Input) (output : List Nat) :
    scan values cards = some output ↔
      (∀ card ∈ cards, Domain card) ∧ output = collected values cards := by
  classical
  induction cards generalizing values with
  | nil => simp [scan, collected, eq_comm]
  | cons card rest ih =>
      by_cases valid : Domain card
      · simp [scan, valid, validCard, ih, collected]
      · simp [scan, valid, validCard]

def validate (cards : List Input) : Bool :=
  if 65535 < cards.length then false
  else match scan [] cards with
    | none => false
    | some limited => decide (limited.length ≤ 15)

/-- Acceptance characterizes every row, the full dense capacity, and exactly
the number of distinct nonzero limited bonuses, rather than a clipped count. -/
theorem validate_iff (cards : List Input) :
    validate cards = true ↔ cards.length ≤ 65535 ∧
      (∀ card ∈ cards, Domain card) ∧ (limitedSet cards).card ≤ 15 := by
  classical
  have unique := collected_unique [] cards (by simp)
  have set_eq : (collected [] cards).toFinset = limitedSet cards := by
    simpa using collected_set [] cards
  have length_eq : (collected [] cards).length = (limitedSet cards).card := by
    rw [← set_eq, List.toFinset_card_of_nodup unique]
  by_cases size : 65535 < cards.length
  · simp [validate, size, Nat.not_le.mpr size]
  · have fits : cards.length ≤ 65535 := by omega
    by_cases rows : ∀ card ∈ cards, Domain card
    · have scanned : scan [] cards = some (collected [] cards) :=
        (scan_iff [] cards _).mpr ⟨rows, rfl⟩
      rw [validate, if_neg size, scanned]
      simp only [decide_eq_true_eq, length_eq]
      constructor
      · intro bounded
        exact ⟨fits, rows, bounded⟩
      · intro accepted
        exact accepted.2.2
    · have failed : scan [] cards = none := by
        cases result : scan [] cards with
        | none => rfl
        | some output => exact False.elim (rows ((scan_iff [] cards output).mp result).1)
      simp [validate, size, fits, rows, failed]

/-- Every dense offset of an accepted full table has a lossless u16 address;
no 512-card metadata shortcut limits the full arena. -/
theorem dense_address (cards : List Input) (accepted : validate cards = true) (index : Fin cards.length) :
    ∃ encoded : Fin 65536, encoded.val = index.val := by
  have bound := ((validate_iff cards).mp accepted).1
  exact ⟨⟨index.val, by omega⟩, rfl⟩

theorem accepted_row (cards : List Input) (accepted : validate cards = true) (card : Input)
    (member : card ∈ cards) : Domain card := ((validate_iff cards).mp accepted).2.1 card member

theorem accepted_public_identity (cards : List Input) (accepted : validate cards = true) (card : Input)
    (member : card ∈ cards) : Admission.publicId card.publicId = .ok card.publicId.toNat := by
  have domain := accepted_row cards accepted card member
  have nonnegative : 0 ≤ card.publicId := domain.1
  apply (identity_ok_iff .publicId 65535 card.publicId.toNat card.publicId).mpr
  exact ⟨nonnegative, domain.2.1, by omega⟩

/-- All identity casts used by the builder and the optional full-precision
row are lossless after the complete card-table guard. -/
theorem accepted_narrowing (cards : List Input) (accepted : validate cards = true) :
    cards.length % 65536 = cards.length ∧
      (∀ index : Fin cards.length, index.val % 65536 = index.val) ∧
      (∀ card ∈ cards, card.publicId.toNat % 65536 = card.publicId.toNat ∧
        card.character % 256 = card.character ∧ card.attr % 256 = card.attr ∧
        card.unitMask % 256 = card.unitMask) := by
  have bound := ((validate_iff cards).mp accepted).1
  refine ⟨Nat.mod_eq_of_lt (by omega), ?_, ?_⟩
  · intro index
    have address := index.isLt
    exact Nat.mod_eq_of_lt (by omega)
  · intro card member
    have domain := accepted_row cards accepted card member
    unfold Domain at domain
    rcases domain with ⟨nonnegative, publicBound, charBound, attrBound, maskBound, _⟩
    exact ⟨Nat.mod_eq_of_lt (by omega), Nat.mod_eq_of_lt (by omega),
      Nat.mod_eq_of_lt (by omega), Nat.mod_eq_of_lt (by omega)⟩

theorem metadata_addresses (card : Input) (valid : Domain card) : card.character < 27 ∧ card.attr < 5 := by
  have characterBound := valid.2.2.1
  have attrBound := valid.2.2.2.1
  omega

theorem attribute_overflow_rejected (card : Input) (overflow : 5 ≤ card.attr) : validCard card = false := by
  have invalid : ¬Domain card := by
    intro domain
    have bound := domain.2.2.2.1
    omega
  simp [validCard, invalid]

end CardTable

namespace LimitedCode
open CardTable

/-- Zero is implicit; nonzero values reuse the first stored position or append. -/
def code (values : List Nat) (value : Nat) : Nat :=
  if value = 0 then 0 else
    match Intern.firstIndex value values with
    | some index => index + 1
    | none => values.length + 1

def decode (values : List Nat) (index : Nat) : Nat :=
  if index = 0 then 0 else (values[index - 1]?).getD 0

theorem step_reference (values : List Nat) (value : Nat) :
    code values value ≤ (remember values value).length ∧
      (value = 0 ∧ code values value = 0 ∨
        0 < code values value ∧ (remember values value)[code values value - 1]? = some value) := by
  by_cases zero : value = 0
  · simp [code, zero, remember]
  · cases found : Intern.firstIndex value values with
    | none =>
        have missing := (Intern.first_index_none value values).mp found
        simp [code, zero, found, remember, missing]
    | some index =>
        obtain ⟨bound, lookup⟩ := Intern.first_index_lookup value values index found
        have present : value ∈ values := by
          by_contra missing
          have absent := (Intern.first_index_none value values).mpr missing
          rw [absent] at found
          contradiction
        simp only [code, if_neg zero, found, remember, present, or_true, ↓reduceIte,
          Nat.add_sub_cancel]
        exact ⟨by omega, Or.inr ⟨by omega, lookup⟩⟩

theorem collected_append (values : List Nat) (before suffix : List Input) :
    collected values (before ++ suffix) = collected (collected values before) suffix := by
  induction before generalizing values with
  | nil => rfl
  | cons card rest ih => exact ih (remember values card.limitedBonus)

theorem collected_length (values : List Nat) (cards : List Input) :
    values.length ≤ (collected values cards).length := by
  induction cards generalizing values with
  | nil => exact Nat.le_refl _
  | cons card rest ih =>
      apply le_trans _ (ih (remember values card.limitedBonus))
      unfold remember
      split <;> simp

theorem collected_reference (values : List Nat) (cards : List Input)
    (index : Nat) (bound : index < values.length) :
    (collected values cards)[index]? = values[index]? := by
  induction cards generalizing values with
  | nil => rfl
  | cons card rest ih =>
      have grows : values.length ≤ (remember values card.limitedBonus).length := by
        unfold remember
        split <;> simp
      rw [collected, ih (remember values card.limitedBonus) (bound.trans_le grows)]
      unfold remember
      split
      · rfl
      · exact List.getElem?_append_left bound

theorem accepted_length (cards : List Input) (accepted : validate cards = true) :
    (collected [] cards).length ≤ 15 := by
  have unique := collected_unique [] cards (by simp)
  have setEq : (collected [] cards).toFinset = limitedSet cards := by
    simpa using collected_set [] cards
  rw [← List.toFinset_card_of_nodup unique, setEq]
  exact ((validate_iff cards).mp accepted).2.2

/-- The row reference is valid in the final table, not only immediately after
its insertion. Its four-bit bound follows from the full admission guard. -/
theorem accepted_reference (before suffix : List Input) (card : Input)
    (accepted : validate (before ++ card :: suffix) = true) :
    let index := code (collected [] before) card.limitedBonus
    index ≤ 15 ∧ decode (collected [] (before ++ card :: suffix)) index = card.limitedBonus := by
  dsimp only
  have step := step_reference (collected [] before) card.limitedBonus
  have finalBound := accepted_length (before ++ card :: suffix) accepted
  rw [collected_append, collected] at finalBound ⊢
  have grows := collected_length (remember (collected [] before) card.limitedBonus) suffix
  refine ⟨step.1.trans (grows.trans finalBound), ?_⟩
  rcases step.2 with ⟨zero, absent⟩ | ⟨positive, lookup⟩
  · rw [decode, absent, if_pos rfl]
    exact zero.symm
  · have address : code (collected [] before) card.limitedBonus - 1 <
        (remember (collected [] before) card.limitedBonus).length := by omega
    rw [decode, if_neg (by omega), collected_reference _ suffix _ address, lookup]
    rfl

/-- The packed field has twelve total-bonus bits and four side-table bits. -/
def pack (total index : Nat) : Nat := (total <<< 4) ||| index

theorem pack_eq (total index : Nat) (fits : index < 16) :
    pack total index = total * 16 + index := by
  rw [pack, ← Nat.shiftLeft_add_eq_or_of_lt (i := 4) fits total]
  simp [Nat.shiftLeft_eq]

theorem packing_exact (total index : Nat) (totalBound : total ≤ 4095) (indexBound : index ≤ 15) :
    pack total index < 65536 ∧ pack total index >>> 4 = total ∧ pack total index &&& 15 = index := by
  have fits : index < 16 := by omega
  rw [pack_eq total index fits]
  have mask : (15 : Nat) = 2 ^ 4 - 1 := by decide
  rw [mask, Nat.and_two_pow_sub_one_eq_mod, Nat.shiftRight_eq_div_pow]
  norm_num only [Nat.reducePow]
  omega

/-- Both casts performed while writing a checked bonus row preserve value. -/
theorem accepted_packing (before suffix : List Input) (card : Input)
    (accepted : validate (before ++ card :: suffix) = true) :
    let index := code (collected [] before) card.limitedBonus
    let total := card.baseBonus + card.limitedBonus
    (pack total index) % 65536 = pack total index ∧
      index % 256 = index ∧ pack total index >>> 4 = total ∧
      decode (collected [] (before ++ card :: suffix)) (pack total index &&& 15) = card.limitedBonus := by
  have reference := accepted_reference before suffix card accepted
  have member : card ∈ before ++ card :: suffix := List.mem_append_right _ (by simp)
  have domain := accepted_row _ accepted card member
  have totalBound := domain.2.2.2.2.2.2.2.1
  have packed := packing_exact (card.baseBonus + card.limitedBonus)
    (code (collected [] before) card.limitedBonus) totalBound reference.1
  dsimp only
  exact ⟨Nat.mod_eq_of_lt packed.1, Nat.mod_eq_of_lt (by omega), packed.2.1,
    by rw [packed.2.2]; exact reference.2⟩

/-- The builder's fresh-value assertion is implied by prior validation of
the full stream, even before any remaining rows have been written. -/
theorem accepted_append_space (before suffix : List Input) (card : Input)
    (accepted : validate (before ++ card :: suffix) = true)
    (nonzero : card.limitedBonus ≠ 0) (fresh : card.limitedBonus ∉ collected [] before) :
    (collected [] before).length < 15 := by
  have reference := (accepted_reference before suffix card accepted).1
  have missing := (Intern.first_index_none card.limitedBonus (collected [] before)).mpr fresh
  simp only [code, if_neg nonzero, missing] at reference
  omega

theorem accepted_components (before after : List Input) (card : Input)
    (accepted : validate (before ++ card :: after) = true) :
    let index := code (collected [] before) card.limitedBonus
    let word := pack (card.baseBonus + card.limitedBonus) index
    (word >>> 4) - decode (collected [] (before ++ card :: after)) (word &&& 15) = card.baseBonus := by
  have packed := accepted_packing before after card accepted
  dsimp only
  rw [packed.2.2.1, packed.2.2.2]
  omega

end LimitedCode

namespace PowerEncoding

/-- Little-endian two-bit fields, with disjoint positions joined by bitwise OR. -/
def fields : List (Fin 4) → Nat
  | [] => 0
  | digit :: rest => digit.val ||| (fields rest <<< 2)

theorem fields_cons (digit : Fin 4) (rest : List (Fin 4)) :
    fields (digit :: rest) = digit.val + 4 * fields rest := by
  rw [fields, Nat.or_comm, ← Nat.shiftLeft_add_eq_or_of_lt (i := 2) digit.isLt (fields rest)]
  simp [Nat.shiftLeft_eq, Nat.add_comm, Nat.mul_comm]

theorem fields_bound (digits : List (Fin 4)) : fields digits < 4 ^ digits.length := by
  induction digits with
  | nil => decide
  | cons digit rest ih =>
      rw [fields_cons, List.length_cons, pow_succ]
      have fits := digit.isLt
      omega

theorem fields_read (digits : List (Fin 4)) (index : Nat) (valid : index < digits.length) :
    (fields digits / 4 ^ index) % 4 = (digits[index]).val := by
  induction digits generalizing index with
  | nil => simp at valid
  | cons digit rest ih =>
      cases index with
      | zero =>
          rw [fields_cons]
          have fits := digit.isLt
          simp only [pow_zero, Nat.div_one, List.getElem_cons_zero]
          omega
      | succ index =>
          have fits := digit.isLt
          have first : (digit.val + 4 * fields rest) / 4 = fields rest := by omega
          rw [fields_cons, pow_succ', ← Nat.div_div_eq_div_mul, first]
          exact ih index (by simpa using valid)

def high (value : Fin 262144) : Fin 4 := ⟨value.val / 65536, by have bound := value.isLt; omega⟩
def low (value : Fin 262144) : Nat := value.val % 65536

def digits (values : Fin 8 → Fin 262144) : List (Fin 4) :=
  [high (values 0), high (values 1), high (values 2), high (values 3),
   high (values 4), high (values 5), high (values 6), high (values 7)]

def highWord (values : Fin 8 → Fin 262144) : Nat := fields (digits values)

/-- Interleaved primary/secondary writes in the four-iteration source loop. -/
def sourceHighWord (values : Fin 8 → Fin 262144) : Nat :=
  (high (values 0)).val ||| ((high (values 4)).val <<< 8) |||
  ((high (values 1)).val <<< 2) ||| ((high (values 5)).val <<< 10) |||
  ((high (values 2)).val <<< 4) ||| ((high (values 6)).val <<< 12) |||
  ((high (values 3)).val <<< 6) ||| ((high (values 7)).val <<< 14)

theorem source_high_word (values : Fin 8 → Fin 262144) : sourceHighWord values = highWord values := by
  simp only [sourceHighWord, highWord, digits, fields, Nat.shiftLeft_or_distrib,
    Nat.zero_shiftLeft, Nat.or_zero]
  ac_rfl

theorem high_word_bound (values : Fin 8 → Fin 262144) : highWord values < 65536 := by
  simpa only [highWord, digits, List.length_cons, List.length_nil, Nat.reduceAdd, Nat.reducePow] using
    fields_bound (digits values)

theorem high_word_read (values : Fin 8 → Fin 262144) (index : Fin 8) :
    (highWord values / 4 ^ index.val) % 4 = (values index).val / 65536 := by
  have read := fields_read (digits values) index.val index.isLt
  have slot : (digits values)[index.val] = high (values index) := by
    fin_cases index <;> rfl
  simpa only [highWord, slot, high] using read

/-- The unit-profile bits occupy bits sixteen through twenty-one, above all
eight high-two-bit fields. The optional eight-state sidecar is not encoded here. -/
def lut (values : Fin 8 → Fin 262144) (profiles : Fin 64) : Nat :=
  sourceHighWord values ||| (profiles.val <<< 16)

theorem lut_eq (values : Fin 8 → Fin 262144) (profiles : Fin 64) :
    lut values profiles = profiles.val * 65536 + highWord values := by
  rw [lut, source_high_word, Nat.or_comm,
    ← Nat.shiftLeft_add_eq_or_of_lt (i := 16) (high_word_bound values) profiles.val]
  simp [Nat.shiftLeft_eq]

theorem lut_bound (values : Fin 8 → Fin 262144) (profiles : Fin 64) :
    lut values profiles < 4294967296 := by
  rw [lut_eq]
  have profileBound := profiles.isLt
  have fieldBound := high_word_bound values
  omega

def decode (values : Fin 8 → Fin 262144) (profiles : Fin 64) (index : Fin 8) : Nat :=
  low (values index) ||| (((lut values profiles >>> (index.val * 2)) &&& 3) <<< 16)

/-- Extracting any high field is unaffected by the higher profile bits. -/
theorem lut_read (values : Fin 8 → Fin 262144) (profiles : Fin 64) (index : Fin 8) :
    ((lut values profiles >>> (index.val * 2)) &&& 3) = (values index).val / 65536 := by
  have read := high_word_read values index
  rw [lut_eq, show (3 : Nat) = 2 ^ 2 - 1 by decide, Nat.and_two_pow_sub_one_eq_mod,
    Nat.shiftRight_eq_div_pow]
  fin_cases index <;> norm_num only [Fin.val_zero, Fin.val_one, Fin.val_ofNat, Nat.reduceMul,
    Nat.reducePow] at read ⊢ <;> omega

theorem decode_exact (values : Fin 8 → Fin 262144) (profiles : Fin 64) (index : Fin 8) :
    decode values profiles index = (values index).val := by
  rw [decode, lut_read, low, Nat.or_comm,
    ← Nat.shiftLeft_add_eq_or_of_lt (i := 16) (Nat.mod_lt _ (by decide))]
  simp only [Nat.shiftLeft_eq, Nat.reducePow]
  omega

/-- The legacy encoding chooses the lowest set bit, not the original unit's
piapro-priority rule used by mixed composition. Empty membership uses slot zero. -/
def primary (mask : Fin 64) : Fin 6 :=
  ((List.finRange 6).find? (fun unit => mask.val.testBit unit.val)).getD 0

def secondary (mask : Fin 64) : Fin 6 :=
  ((List.finRange 6).find? (fun unit => mask.val.testBit unit.val && decide (unit ≠ primary mask))).getD
    (primary mask)

def trailingZeros8 (value : Nat) : Nat :=
  ((List.range 8).takeWhile (fun bit => !(value.testBit bit))).length

theorem source_unit_selection : ∀ mask : Fin 64,
    (primary mask).val = (if mask.val = 0 then 0 else min (trailingZeros8 mask.val) 5) ∧
    (secondary mask).val =
      let remaining := mask.val &&& (255 - 2 ^ (primary mask).val)
      if remaining = 0 then (primary mask).val else min (trailingZeros8 remaining) 5 := by decide

def profile (mask : Fin 64) (unit : Fin 6) : Bool :=
  decide (secondary mask ≠ primary mask ∧ unit = secondary mask)

def selected (mask : Fin 64) (slotProfile : Bool) : Fin 6 :=
  if slotProfile then secondary mask else primary mask

theorem carried_unit_selected : ∀ mask : Fin 64, CardTable.population mask.val ≤ 2 →
    ∀ unit : Fin 6, mask.val.testBit unit.val = true → selected mask (profile mask unit) = unit := by decide

def profileMask (mask : Fin 64) : Nat :=
  if secondary mask = primary mask then 0 else 1 <<< (secondary mask).val

theorem profile_mask_bound : ∀ mask : Fin 64, profileMask mask < 64 := by decide

def profileWord (mask : Fin 64) : Fin 64 := ⟨profileMask mask, profile_mask_bound mask⟩

theorem profile_mask_bit : ∀ (mask : Fin 64) (unit : Fin 6),
    ((profileWord mask).val >>> unit.val) &&& 1 = (profile mask unit).toNat := by decide

theorem lut_profile (values : Fin 8 → Fin 262144) (profiles : Fin 64) (unit : Fin 6) :
    (lut values profiles >>> (16 + unit.val)) &&& 1 = (profiles.val >>> unit.val) &&& 1 := by
  have fieldBound := high_word_bound values
  rw [lut_eq, show (1 : Nat) = 2 ^ 1 - 1 by decide]
  simp only [Nat.and_two_pow_sub_one_eq_mod, Nat.shiftRight_eq_div_pow]
  fin_cases unit <;> norm_num only [Fin.val_zero, Fin.val_one, Fin.val_ofNat, Nat.reduceAdd,
    Nat.reducePow] at fieldBound ⊢ <;> omega

def slot (slotProfile : Bool) (member : Fin 4) : Fin 8 :=
  ⟨slotProfile.toNat * 4 + member.val, by cases slotProfile <;> have bound := member.isLt <;> simp_all <;> omega⟩

def rowMask (card : CardTable.Input) (valid : CardTable.Domain card) : Fin 64 :=
  ⟨card.unitMask, by have bound := valid.2.2.2.2.1; omega⟩

def rowValues (card : CardTable.Input) (valid : CardTable.Domain card) (index : Fin 8) : Fin 262144 :=
  let unit := if index.val < 4 then primary (rowMask card valid) else secondary (rowMask card valid)
  let member : Fin 4 := ⟨index.val % 4, Nat.mod_lt _ (by decide)⟩
  ⟨(card.power unit member).toNat, by
    have bound := valid.2.2.2.2.2.2.1 unit member
    norm_num only [Nat.reducePow, Nat.reduceSub] at bound
    omega⟩

theorem row_slot (card : CardTable.Input) (valid : CardTable.Domain card)
    (slotProfile : Bool) (member : Fin 4) :
    (rowValues card valid (slot slotProfile member)).val =
      (card.power (selected (rowMask card valid) slotProfile) member).toNat := by
  cases slotProfile <;> fin_cases member <;> rfl

/-- The same LUT supplies the unit-profile bit and the high value field.
Every carried unit recovers its own admitted power entry, including two-bit
masks; zero masks issue no carried-unit read. -/
theorem row_decode (card : CardTable.Input) (valid : CardTable.Domain card) (unit : Fin 6) (member : Fin 4)
    (carried : card.unitMask.testBit unit.val = true) :
    decode (rowValues card valid) (profileWord (rowMask card valid))
      (slot (profile (rowMask card valid) unit) member) = (card.power unit member).toNat := by
  rw [decode_exact, row_slot]
  have capacity := valid.2.2.2.2.2.1
  have same := carried_unit_selected (rowMask card valid) capacity unit carried
  rw [same]

theorem row_profile (card : CardTable.Input) (valid : CardTable.Domain card) (unit : Fin 6) :
    (lut (rowValues card valid) (profileWord (rowMask card valid)) >>> (16 + unit.val)) &&& 1 =
      (profile (rowMask card valid) unit).toNat := by
  rw [lut_profile, profile_mask_bit]

theorem source_high_field (value : Fin 262144) :
    ((value.val >>> 16) &&& 3) = (high value).val := by
  rw [show (3 : Nat) = 2 ^ 2 - 1 by decide, Nat.and_two_pow_sub_one_eq_mod,
    Nat.shiftRight_eq_div_pow]
  have bound := (high value).isLt
  simpa only [high, Nat.reducePow] using Nat.mod_eq_of_lt bound

theorem stored_widths (values : Fin 8 → Fin 262144) (profiles : Fin 64) (index : Fin 8) :
    low (values index) < 65536 ∧ lut values profiles % 4294967296 = lut values profiles :=
  ⟨Nat.mod_lt _ (by decide), Nat.mod_eq_of_lt (lut_bound values profiles)⟩

end PowerEncoding

namespace SkillGather

abbrev Kind := Fin 3
abbrev Slot := Nat × Nat

/-- A common payload domain can embed the three concrete skill record types;
content equality is tested only inside the selected table. -/
structure Input (Value : Type*) where
  unitCount : Option Value
  differentUnit : Option Value
  reference : Option Value
  plain : Fin 256 × Fin 256

def selected {Value : Type*} (input : Input Value) : Option (Kind × Value) :=
  match input.unitCount with
  | some value => some (0, value)
  | none => match input.differentUnit with
    | some value => some (1, value)
    | none => input.reference.map (fun value => (2, value))

structure State (Value : Type*) where
  seen : Kind → List Value
  stored : Kind → List Value

def initial {Value : Type*} : State Value := ⟨fun _ => [], fun _ => []⟩

def Healthy {Value : Type*} (state : State Value) : Prop :=
  ∀ kind, state.seen kind = state.stored kind ∧ (state.seen kind).length ≤ 255 ∧ (state.seen kind).Nodup

def Extends {Value : Type*} (before after : State Value) : Prop :=
  ∀ kind, (before.stored kind).length ≤ (after.stored kind).length ∧
    ∀ index, index < (before.stored kind).length → (after.stored kind)[index]? = (before.stored kind)[index]?

def Reference {Value : Type*} (state : State Value) (input : Input Value) (slot : Slot) : Prop :=
  match selected input with
  | none => slot = (input.plain.1.val, input.plain.2.val)
  | some (kind, value) => slot.1 = kind.val + 1 ∧ 0 < slot.2 ∧ slot.2 ≤ 255 ∧
      slot.2 ≤ (state.stored kind).length ∧ (state.stored kind)[slot.2 - 1]? = some value

def Fits (slot : Slot) : Prop := slot.1 < 256 ∧ slot.2 < 256

variable {Value : Type*} [DecidableEq Value]

def write (state : State Value) (kind : Kind) (value : Value) (result : Intern.Result Value) : State Value :=
  { seen := Function.update state.seen kind result.values
    stored := Function.update state.stored kind
      (if result.fresh then state.stored kind ++ [value] else state.stored kind) }

def tableField (kind : Kind) : Field :=
  if kind = 0 then .unitCountSkills else if kind = 1 then .differentUnitSkills else .referenceSkills

def tableError (kind : Kind) : Error → Error
  | .capacity _ value maximum => .capacity (tableField kind) value maximum
  | error => error

def step (state : State Value) (input : Input Value) : Except Error (State Value × Slot) :=
  match selected input with
  | none => .ok (state, (input.plain.1.val, input.plain.2.val))
  | some (kind, value) =>
      match Intern.intern (state.seen kind) value with
      | .error error => .error (tableError kind error)
      | .ok result => .ok (write state kind value result, (kind.val + 1, result.index))

theorem fresh_failure (state : State Value) (input : Input Value) (kind : Kind) (value : Value)
    (choice : selected input = some (kind, value)) (fresh : value ∉ state.seen kind)
    (full : 255 ≤ (state.seen kind).length) :
    step state input = .error (.capacity (tableField kind) ((state.seen kind).length + 1) 255) := by
  simp only [step, choice, Intern.fresh_overflow_rejected _ _ fresh full, tableError]

omit [DecidableEq Value] in
theorem initial_healthy : Healthy (initial : State Value) := by simp [Healthy, initial]

omit [DecidableEq Value] in
theorem extends_refl (state : State Value) : Extends state state :=
  fun _ => ⟨Nat.le_refl _, fun _ _ => rfl⟩

omit [DecidableEq Value] in
theorem extends_trans (a b c : State Value) (ab : Extends a b) (bc : Extends b c) : Extends a c := by
  intro kind
  refine ⟨(ab kind).1.trans (bc kind).1, ?_⟩
  intro index valid
  exact ((bc kind).2 index (valid.trans_le (ab kind).1)).trans ((ab kind).2 index valid)

omit [DecidableEq Value] in
theorem reference_extends (before after : State Value) (extension : Extends before after)
    (input : Input Value) (slot : Slot) (reference : Reference before input slot) : Reference after input slot := by
  cases choice : selected input with
  | none => simpa only [Reference, choice] using reference
  | some pair =>
      rcases pair with ⟨kind, value⟩
      simp only [Reference, choice] at reference ⊢
      refine ⟨reference.1, reference.2.1, reference.2.2.1,
        reference.2.2.2.1.trans (extension kind).1, ?_⟩
      rw [(extension kind).2 (slot.2 - 1) (by omega)]
      exact reference.2.2.2.2

theorem write_healthy (state : State Value) (healthy : Healthy state) (kind : Kind) (value : Value)
    (result : Intern.Result Value) (accepted : Intern.intern (state.seen kind) value = .ok result) :
    Healthy (write state kind value result) := by
  have invariants := Intern.intern_invariants _ value result (healthy kind).2.1 (healthy kind).2.2 accepted
  have shape := Intern.intern_shape _ value result accepted
  intro other
  by_cases same : other = kind
  · subst other
    simp only [write, Function.update_self]
    exact ⟨by simpa only [(healthy kind).1] using shape, invariants.2.2.2.1, invariants.2.2.2.2⟩
  · simpa only [write, Function.update_of_ne same] using healthy other

theorem write_stored (state : State Value) (healthy : Healthy state) (kind : Kind) (value : Value)
    (result : Intern.Result Value) (accepted : Intern.intern (state.seen kind) value = .ok result) :
    (write state kind value result).stored kind = result.values := by
  have same := (write_healthy state healthy kind value result accepted kind).1
  simpa only [write, Function.update_self] using same.symm

theorem write_extends (state : State Value) (healthy : Healthy state) (kind : Kind) (value : Value)
    (result : Intern.Result Value) (accepted : Intern.intern (state.seen kind) value = .ok result) :
    Extends state (write state kind value result) := by
  intro other
  by_cases same : other = kind
  · subst other
    rw [write_stored state healthy kind value result accepted]
    have shape := Intern.intern_shape _ value result accepted
    rw [← (healthy kind).1]
    refine ⟨?_, Intern.intern_lookup_stable _ value result accepted⟩
    rw [shape]
    split <;> simp
  · simp [write, same]

/-- A successful fresh intern leaves room for the builder's checked append. -/
theorem fresh_builder_space (state : State Value) (healthy : Healthy state) (kind : Kind) (value : Value)
    (result : Intern.Result Value) (accepted : Intern.intern (state.seen kind) value = .ok result)
    (fresh : result.fresh = true) : (state.stored kind).length < 255 := by
  have invariants := Intern.intern_invariants _ value result (healthy kind).2.1 (healthy kind).2.2 accepted
  have capacity := invariants.2.2.2.1
  rw [Intern.intern_shape _ value result accepted, fresh, if_pos rfl,
    List.length_append, List.length_singleton, (healthy kind).1] at capacity
  omega

theorem step_spec (state : State Value) (healthy : Healthy state) (input : Input Value)
    (output : State Value × Slot) (accepted : step state input = .ok output) :
    Healthy output.1 ∧ Fits output.2 ∧ Reference output.1 input output.2 ∧ Extends state output.1 := by
  cases choice : selected input with
  | none =>
      have same : (state, (input.plain.1.val, input.plain.2.val)) = output := by
        simpa only [step, choice, Except.ok.injEq] using accepted
      subst output
      exact ⟨healthy, ⟨input.plain.1.isLt, input.plain.2.isLt⟩,
        by simp only [Reference, choice], extends_refl state⟩
  | some pair =>
      rcases pair with ⟨kind, value⟩
      cases call : Intern.intern (state.seen kind) value with
      | error error => simp [step, choice, call] at accepted
      | ok result =>
          have same : (write state kind value result, (kind.val + 1, result.index)) = output := by
            simpa only [step, choice, call, Except.ok.injEq] using accepted
          subst output
          have invariants := Intern.intern_invariants _ value result (healthy kind).2.1 (healthy kind).2.2 call
          have address : result.index - 1 < result.values.length := by
            by_contra missing
            have absent : result.values[result.index - 1]? = none := List.getElem?_eq_none (by omega)
            have lookup := invariants.2.2.1
            rw [absent] at lookup
            cases lookup
          refine ⟨write_healthy state healthy kind value result call, ?_, ?_,
            write_extends state healthy kind value result call⟩
          · change kind.val + 1 < 256 ∧ result.index < 256
            have kindBound := kind.isLt
            have indexBound := invariants.2.1
            exact ⟨by omega, by omega⟩
          · have relation : kind.val + 1 = kind.val + 1 ∧ 0 < result.index ∧ result.index ≤ 255 ∧
                result.index ≤ result.values.length ∧ result.values[result.index - 1]? = some value :=
              ⟨rfl, invariants.1, invariants.2.1, by omega, invariants.2.2.1⟩
            simpa only [Reference, choice, write_stored state healthy kind value result call] using relation

def run (state : State Value) : List (Input Value) → Except Error (State Value × List Slot)
  | [] => .ok (state, [])
  | input :: rest =>
      match step state input with
      | .error error => .error error
      | .ok (next, slot) =>
          match run next rest with
          | .error error => .error error
          | .ok (finalState, slots) => .ok (finalState, slot :: slots)

/-- The actual interleaved stream emits one slot per row in order. Every
reference is checked against the final builder tables, not a transient copy. -/
theorem run_spec (state : State Value) (healthy : Healthy state) (inputs : List (Input Value))
    (output : State Value × List Slot) (accepted : run state inputs = .ok output) :
    Healthy output.1 ∧ List.Forall₂ (Reference output.1) inputs output.2 ∧
      (∀ slot ∈ output.2, Fits slot) ∧ Extends state output.1 := by
  induction inputs generalizing state output with
  | nil =>
      have same : (state, []) = output := by simpa only [run, Except.ok.injEq] using accepted
      subst output
      exact ⟨healthy, .nil, by simp, extends_refl state⟩
  | cons input rest ih =>
      cases call : step state input with
      | error error => simp [run, call] at accepted
      | ok first =>
          rcases first with ⟨next, slot⟩
          have firstSpec := step_spec state healthy input (next, slot) call
          cases following : run next rest with
          | error error => simp [run, call, following] at accepted
          | ok last =>
              rcases last with ⟨finalState, slots⟩
              have same : (finalState, slot :: slots) = output := by
                simpa only [run, call, following, Except.ok.injEq] using accepted
              subst output
              have restSpec := ih next firstSpec.1 (finalState, slots) following
              refine ⟨restSpec.1, .cons (reference_extends next finalState restSpec.2.2.2 input slot
                firstSpec.2.2.1) restSpec.2.1, ?_, extends_trans state next finalState firstSpec.2.2.2 restSpec.2.2.2⟩
              intro other member
              rcases List.mem_cons.mp member with same | member
              · simpa only [same] using firstSpec.2.1
              · exact restSpec.2.2.1 other member

theorem from_empty (inputs : List (Input Value)) (output : State Value × List Slot)
    (accepted : run initial inputs = .ok output) :
    Healthy output.1 ∧ List.Forall₂ (Reference output.1) inputs output.2 ∧
      (∀ slot ∈ output.2, Fits slot) ∧ output.2.length = inputs.length := by
  have result := run_spec initial initial_healthy inputs output accepted
  exact ⟨result.1, result.2.1, result.2.2.1, result.2.1.length_eq.symm⟩

/-- Exact byte-valued contents of the three production records. -/
abbrev UnitCountPayload := Fin 256 × (Fin 5 → Fin 256)
abbrev PairPayload := Fin 256 × Fin 256

inductive Payload where
  | unitCount (data : UnitCountPayload)
  | differentUnit (data : PairPayload)
  | reference (data : PairPayload)
  deriving DecidableEq

def unitEqual (left right : UnitCountPayload) : Bool :=
  decide (left.1 = right.1) && (List.finRange 5).all (fun index => decide (left.2 index = right.2 index))

def pairEqual (left right : PairPayload) : Bool :=
  decide (left.1 = right.1) && decide (left.2 = right.2)

def sourceEqual : Payload → Payload → Bool
  | .unitCount left, .unitCount right => unitEqual left right
  | .differentUnit left, .differentUnit right => pairEqual left right
  | .reference left, .reference right => pairEqual left right
  | _, _ => false

theorem unit_equal_iff (left right : UnitCountPayload) : unitEqual left right = true ↔ left = right := by
  simp [unitEqual, List.all_eq_true, Prod.ext_iff, funext_iff]

theorem pair_equal_iff (left right : PairPayload) : pairEqual left right = true ↔ left = right := by
  simp [pairEqual, Prod.ext_iff]

/-- Rust's derived record/array equality agrees with equality of the bounded
payload model. No floating-point equality or validity premise is involved. -/
theorem source_equal_iff (left right : Payload) : sourceEqual left right = true ↔ left = right := by
  cases left <;> cases right <;> simp [sourceEqual, unit_equal_iff, pair_equal_iff]

structure ConcreteInput where
  unitCount : Option UnitCountPayload
  differentUnit : Option PairPayload
  reference : Option PairPayload
  rawSlot : Fin 256 × Fin 256

def encodeInput (input : ConcreteInput) : Input Payload :=
  { unitCount := input.unitCount.map Payload.unitCount
    differentUnit := input.differentUnit.map Payload.differentUnit
    reference := input.reference.map Payload.reference
    plain := input.rawSlot }

theorem concrete_priority (input : ConcreteInput) :
    selected (encodeInput input) =
      match input.unitCount with
      | some value => some (0, Payload.unitCount value)
      | none => match input.differentUnit with
        | some value => some (1, Payload.differentUnit value)
        | none => input.reference.map (fun value => (2, Payload.reference value)) := by
  cases input with
  | mk unitCount differentUnit reference plain =>
      cases unitCount <;> cases differentUnit <;> cases reference <;> rfl

theorem concrete_from_empty (inputs : List ConcreteInput) (output : State Payload × List Slot)
    (accepted : run initial (inputs.map encodeInput) = .ok output) :
    Healthy output.1 ∧ List.Forall₂ (fun input slot => Reference output.1 (encodeInput input) slot) inputs output.2 ∧
      (∀ slot ∈ output.2, Fits slot) ∧ output.2.length = inputs.length := by
  have result := from_empty (inputs.map encodeInput) output accepted
  refine ⟨result.1, ?_, result.2.2.1, by simpa only [List.length_map] using result.2.2.2⟩
  simpa only [List.forall₂_map_left_iff] using result.2.1

/-- The internal gather boundary preserves any byte-valued slot when no
side-table payload is present; it does not validate the slot's interpretation. -/
theorem no_side_table_preserves_raw (state : State Payload) (input : ConcreteInput)
    (absent : selected (encodeInput input) = none) :
    step state (encodeInput input) = .ok (state, (input.rawSlot.1.val, input.rawSlot.2.val)) := by
  rw [step, absent]
  rfl

theorem concrete_reference_or_raw (state : State Payload) (input : ConcreteInput) (slot : Slot)
    (valid : Reference state (encodeInput input) slot) :
    (selected (encodeInput input) = none ∧ slot = (input.rawSlot.1.val, input.rawSlot.2.val)) ∨
      ∃ (kind : Kind) (value : Payload), selected (encodeInput input) = some (kind, value) ∧
        slot.1 = kind.val + 1 ∧ 0 < slot.2 ∧ slot.2 ≤ 255 ∧ slot.2 ≤ (state.stored kind).length ∧
        (state.stored kind)[slot.2 - 1]? = some value := by
  cases choice : selected (encodeInput input) with
  | none =>
      exact Or.inl ⟨rfl, by simpa only [Reference, choice] using valid⟩
  | some pair =>
      rcases pair with ⟨kind, value⟩
      exact Or.inr ⟨kind, value, rfl, by simpa only [Reference, choice] using valid⟩

/-- In particular, an internally supplied tagged slot is retained even when
the corresponding table is empty. This is not a producer reachability claim. -/
theorem tagged_raw_slot_passthrough :
    run (initial : State Payload) [encodeInput ⟨none, none, none, (1, 7)⟩] =
      .ok (initial, [(1, 7)]) ∧ (initial : State Payload).stored 0 = [] := by
  constructor <;> rfl

end SkillGather

namespace PoolAttribute

/-- Public enum values are Null=0 followed by the five attributes. The pool
stores only the zero-based real attributes; null has no pool index. -/
def toIndex (raw : Fin 6) : Option (Fin 5) :=
  if raw.val = 0 then none else some ⟨raw.val - 1, by have bound := raw.isLt; omega⟩

theorem all_public_attributes : toIndex 0 = none ∧ toIndex 1 = some 0 ∧ toIndex 2 = some 1 ∧
    toIndex 3 = some 2 ∧ toIndex 4 = some 3 ∧ toIndex 5 = some 4 := by decide

theorem all_pool_indices : ∀ index : Fin 5, ∃ raw : Fin 6, toIndex raw = some index := by decide

end PoolAttribute

namespace GatherOrder

inductive Target where
  | score | power | skill | bonus | mysekai
  deriving DecidableEq

structure Signed32 where
  value : Int
  lower : -2147483648 ≤ value
  upper : value ≤ 2147483647

structure Card where
  core : CardTable.Core
  powerMax : Signed32
  powerMin : Signed32
  multiPower : Option (Fin 8 → Signed32) := none
  trained : Bool
  skills : SkillGather.ConcreteInput
  leaderHonor : Fin 65536 := 0
  leaderLimit : Fin 65536 := 0

/-- Validation and interning read the same optional reference record. -/
def Card.row (card : Card) : CardTable.Input :=
  { card.core with referenceMax := card.skills.reference.map Prod.snd }

@[simp] theorem reference_field (card : Card) :
    card.row.referenceMax = card.skills.reference.map Prod.snd := rfl

/-- Changing only the raw slot leaves the capacity validation input unchanged. -/
theorem raw_slot_validation_row (card : Card) (raw : Fin 256 × Fin 256) :
    ({ card with skills := { card.skills with rawSlot := raw } }).row = card.row := rfl

theorem nonnegative_u32_cast (value : Signed32) :
    value.value.toNat % 4294967296 = value.value.toNat := by
  have bound := value.upper
  exact Nat.mod_eq_of_lt (by omega)

theorem multi_row_u32_cast (card : Card) (values : Fin 8 → Signed32)
    (present : card.multiPower = some values) :
    ∀ index : Fin 8, (card.multiPower.map (fun row => (row index).value.toNat % 4294967296)) =
      some (values index).value.toNat := by
  intro index
  simp only [present, Option.map_some, nonnegative_u32_cast]

def scoreKey (card : Card) : Nat := card.powerMax.value.toNat * (256 + card.row.skillMax.val)

theorem score_key_fits (card : Card) : scoreKey card < 18446744073709551616 := by
  have powerBound : card.powerMax.value.toNat ≤ 2147483647 := by have bound := card.powerMax.upper; omega
  have skillBound := card.row.skillMax.isLt
  unfold scoreKey
  nlinarith

/-- Both event-aware Score branches have this same key, including Solo/Auto.
The signed extrema and public ID retain the source's descending tie order. -/
def ordinaryKey (target : Target) (hasEvent : Bool) (card : Card) : List Int :=
  let power := [-card.powerMax.value, -card.powerMin.value, -card.row.publicId]
  match target with
  | .skill => [-Int.ofNat card.row.skillMax.val, -Int.ofNat card.row.skillMin.val, -card.row.publicId]
  | .power => power
  | _ =>
      if hasEvent then
        [-Int.ofNat (card.row.baseBonus + card.row.limitedBonus), -Int.ofNat (scoreKey card),
          -card.powerMax.value, -Int.ofNat card.row.skillMax.val, -card.row.publicId]
      else match target with
        | .score => [-Int.ofNat (scoreKey card), -card.powerMax.value, -Int.ofNat card.row.skillMax.val,
            -card.powerMin.value, -Int.ofNat card.row.skillMin.val, -card.row.publicId]
        | _ => power

def fixedRank (fixedCards fixedCharacters : List Nat) (card : Card) : Option Nat :=
  match Intern.firstIndex (card.row.publicId % 65536).toNat fixedCards with
  | some rank => some rank
  | none => (Intern.firstIndex card.row.character fixedCharacters).map (fixedCards.length + ·)

/-- Rank precedes trained state; trained state participates only inside a
fixed public-card slot. Character-fixed and free slots use the ordinary key. -/
def key (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat) (card : Card) : List Int :=
  match fixedRank fixedCards fixedCharacters card with
  | none => [1, 0, 0] ++ ordinaryKey target hasEvent card
  | some rank =>
      [0, Int.ofNat rank, if rank < fixedCards.length ∧ card.trained = false then 1 else 0] ++
        ordinaryKey target hasEvent card

def sort (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat) (cards : List Card) : List Card :=
  cards.mergeSort (fun left right => decide (key target hasEvent fixedCards fixedCharacters left ≤
    key target hasEvent fixedCards fixedCharacters right))

/-- The concrete key comparator is a total preorder, including equal keys.
No sorting precondition is imposed on the input stream. -/
theorem comparator_total (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (left right : Card) :
    key target hasEvent fixedCards fixedCharacters left ≤ key target hasEvent fixedCards fixedCharacters right ∨
      key target hasEvent fixedCards fixedCharacters right ≤ key target hasEvent fixedCards fixedCharacters left := le_total _ _

theorem comparator_trans (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (a b c : Card) :
    key target hasEvent fixedCards fixedCharacters a ≤ key target hasEvent fixedCards fixedCharacters b →
    key target hasEvent fixedCards fixedCharacters b ≤ key target hasEvent fixedCards fixedCharacters c →
    key target hasEvent fixedCards fixedCharacters a ≤ key target hasEvent fixedCards fixedCharacters c := le_trans

theorem sorted (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat) (cards : List Card) :
    (sort target hasEvent fixedCards fixedCharacters cards).Sorted
      (fun left right => key target hasEvent fixedCards fixedCharacters left ≤
        key target hasEvent fixedCards fixedCharacters right) := by
  let relation := fun left right => key target hasEvent fixedCards fixedCharacters left ≤
    key target hasEvent fixedCards fixedCharacters right
  letI : IsTrans Card relation := ⟨fun a b c => comparator_trans target hasEvent fixedCards fixedCharacters a b c⟩
  letI : IsTotal Card relation := ⟨comparator_total target hasEvent fixedCards fixedCharacters⟩
  exact List.sorted_mergeSort' relation cards

theorem sort_permutation (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat) (cards : List Card) :
    (sort target hasEvent fixedCards fixedCharacters cards).Perm cards := List.mergeSort_perm _ _

theorem validate_permutation (first second : List CardTable.Input) (perm : first.Perm second)
    (accepted : CardTable.validate first = true) : CardTable.validate second = true := by
  have domain := (CardTable.validate_iff first).mp accepted
  apply (CardTable.validate_iff second).mpr
  refine ⟨by simpa only [← perm.length_eq] using domain.1,
    fun card member => domain.2.1 card (perm.mem_iff.mpr member), ?_⟩
  have limited : CardTable.limitedSet first = CardTable.limitedSet second := by
    unfold CardTable.limitedSet
    congr 1
    ext item
    simpa only [List.mem_toFinset] using (perm.map (fun card => card.limitedBonus)).mem_iff (a := item)
  simpa only [← limited] using domain.2.2

theorem sort_preserves_validation (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List Card) (accepted : CardTable.validate (cards.map Card.row) = true) :
    CardTable.validate ((sort target hasEvent fixedCards fixedCharacters cards).map Card.row) = true :=
  validate_permutation _ _ ((sort_permutation target hasEvent fixedCards fixedCharacters cards).symm.map Card.row) accepted

end GatherOrder

namespace CardValidation
open CardTable

structure Check where
  field : Field
  value : Nat
  maximum : Nat

def checkAll : List Check → Except Error Unit
  | [] => .ok ()
  | check :: rest =>
      if check.maximum < check.value then .error (.capacity check.field check.value check.maximum)
      else checkAll rest

theorem check_all_ok (checks : List Check) :
    checkAll checks = .ok () ↔ ∀ check ∈ checks, check.value ≤ check.maximum := by
  induction checks with
  | nil => simp [checkAll]
  | cons first rest ih =>
      by_cases tooLarge : first.maximum < first.value
      · simp [checkAll, tooLarge, Nat.not_le.mpr tooLarge]
      · simp [checkAll, tooLarge, ih, Nat.le_of_not_gt tooLarge]

/-- Scalar and detail checks are listed in their source order. -/
def checks (card : Input) : List Check :=
  [⟨.publicId, card.publicId.toNat, 65535⟩,
   ⟨.characterId, card.character, 26⟩,
   ⟨.attributeId, card.attr, 4⟩,
   ⟨.unitMask, card.unitMask, 63⟩,
   ⟨.unitProfiles, population card.unitMask, 2⟩] ++
  (List.finRange 6).flatMap (fun unit => (List.finRange 4).map
    (fun member => ⟨.power, (card.power unit member).toNat, 2 ^ 18 - 1⟩)) ++
  [⟨.eventBonus, card.baseBonus + card.limitedBonus, 4095⟩] ++
  match card.referenceMax with
  | none => []
  | some maximum => [⟨.referenceUpper, card.skillMin.val + maximum.val, card.skillMax.val⟩]

def row (card : Input) : Except Error Unit :=
  if card.publicId < 0 then .error (.negative .publicId) else checkAll (checks card)

theorem row_ok (card : Input) : row card = .ok () ↔ Domain card := by
  by_cases negative : card.publicId < 0
  · have notValid : ¬Domain card := fun valid => (not_lt_of_ge valid.1) negative
    simp [row, negative, notValid]
  · have nonnegative : 0 ≤ card.publicId := by omega
    have identityBound : card.publicId.toNat ≤ 65535 ↔ card.publicId ≤ 65535 := by omega
    simp only [row, if_neg negative, check_all_ok, checks, List.forall_mem_append,
      List.forall_mem_cons, List.forall_mem_flatMap, List.forall_mem_map]
    cases reference : card.referenceMax <;>
      simp [Domain, reference, nonnegative, identityBound, and_assoc]

def rows : List Nat → List Input → Except Error (List Nat)
  | values, [] => .ok values
  | values, card :: rest =>
      match row card with
      | .error error => .error error
      | .ok () => rows (remember values card.limitedBonus) rest

theorem rows_ok (values : List Nat) (cards : List Input) (output : List Nat) :
    rows values cards = .ok output ↔ (∀ card ∈ cards, Domain card) ∧ output = collected values cards := by
  induction cards generalizing values with
  | nil => simp [rows, collected, eq_comm]
  | cons card rest ih =>
      by_cases valid : Domain card
      · have accepted := (row_ok card).mpr valid
        simp [rows, accepted, ih, valid, collected]
      · have rejected : ∃ error, row card = .error error := by
          cases outcome : row card with
          | error error => exact ⟨error, rfl⟩
          | ok token => cases token; exact False.elim (valid ((row_ok card).mp outcome))
        obtain ⟨error, rejected⟩ := rejected
        simp [rows, rejected, valid]

/-- TooManyCards and per-field failures retain typed errors; successful
validation checks the final distinct limited-value count after the row stream. -/
def validate (cards : List Input) : Except Error Unit :=
  if 65535 < cards.length then .error (.tooManyCards cards.length)
  else match rows [] cards with
    | .error error => .error error
    | .ok values => checkAll [⟨.limitedValues, values.length, 15⟩]

theorem validate_ok (cards : List Input) : validate cards = .ok () ↔ CardTable.validate cards = true := by
  rw [CardTable.validate_iff]
  by_cases tooMany : 65535 < cards.length
  · simp [validate, tooMany, Nat.not_le.mpr tooMany]
  · have fits : cards.length ≤ 65535 := by omega
    by_cases valid : ∀ card ∈ cards, Domain card
    · have accepted := (rows_ok [] cards (collected [] cards)).mpr ⟨valid, rfl⟩
      have tableLength : (collected [] cards).length = (limitedSet cards).card := by
        have unique := collected_unique [] cards (by simp)
        have same : (collected [] cards).toFinset = limitedSet cards := by simpa using collected_set [] cards
        rw [← List.toFinset_card_of_nodup unique, same]
      rw [validate, if_neg tooMany, accepted, check_all_ok]
      simp only [List.forall_mem_cons, tableLength]
      constructor
      · intro bound
        exact ⟨fits, valid, bound.1⟩
      · intro admitted
        exact ⟨admitted.2.2, by simp⟩
    · have rejected : ∃ error, rows [] cards = .error error := by
        cases outcome : rows [] cards with
        | error error => exact ⟨error, rfl⟩
        | ok values => exact False.elim (valid ((rows_ok [] cards values).mp outcome).1)
      obtain ⟨error, rejected⟩ := rejected
      simp [validate, tooMany, rejected, valid]

end CardValidation

namespace DenseWrites

/-- Consecutive dense writes into one builder column. Only the initialized
range is observable; the initial contents outside it are immaterial. -/
def fill {Value : Type*} : List Value → Nat → (Nat → Option Value) → Nat → Option Value
  | [], _, column => column
  | value :: rest, start, column => fill rest (start + 1) (Function.update column start (some value))

theorem preserves_before {Value : Type*} (values : List Value) (start : Nat) (column : Nat → Option Value)
    (index : Nat) (before : index < start) : fill values start column index = column index := by
  induction values generalizing start column with
  | nil => rfl
  | cons value rest ih =>
      rw [fill, ih (start + 1) _ (by omega), Function.update_of_ne (by omega : index ≠ start)]

theorem reads_row {Value : Type*} (values : List Value) (start : Nat) (column : Nat → Option Value)
    (index : Nat) (valid : index < values.length) : fill values start column (start + index) = some values[index] := by
  induction values generalizing start column index with
  | nil => simp at valid
  | cons value rest ih =>
      cases index with
      | zero =>
          simp only [Nat.add_zero, List.getElem_cons_zero, fill]
          rw [preserves_before rest (start + 1) _ start (by omega), Function.update_self]
      | succ index =>
          rw [fill]
          have same : start + (index + 1) = (start + 1) + index := by omega
          rw [same]
          exact ih (start + 1) _ index (by simpa using valid)

theorem initialized_column {Value : Type*} (values : List Value) (index : Fin values.length) :
    fill values 0 (fun _ => none) index.val = some values[index] := by
  simpa only [Nat.zero_add] using reads_row values 0 (fun _ => none) index.val index.isLt

end DenseWrites

namespace GatherPipeline
open GatherOrder

structure Pool where
  cards : List GatherOrder.Card
  tables : SkillGather.State SkillGather.Payload
  slots : List SkillGather.Slot

/-- Validation precedes sorting and every builder write. A failed interner is
propagated without returning a partially built pool. -/
def gather (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List GatherOrder.Card) : Except Error Pool :=
  match CardValidation.validate (cards.map Card.row) with
  | .error error => .error error
  | .ok () =>
      let ordered := sort target hasEvent fixedCards fixedCharacters cards
      match SkillGather.run SkillGather.initial (ordered.map (fun card => SkillGather.encodeInput card.skills)) with
      | .error error => .error error
      | .ok (tables, slots) => .ok ⟨ordered, tables, slots⟩

def Valid (pool : Pool) : Prop :=
  CardTable.validate (pool.cards.map Card.row) = true ∧ SkillGather.Healthy pool.tables ∧
    List.Forall₂ (fun card slot => SkillGather.Reference pool.tables (SkillGather.encodeInput card.skills) slot)
      pool.cards pool.slots ∧ (∀ slot ∈ pool.slots, SkillGather.Fits slot) ∧ pool.slots.length = pool.cards.length

theorem gather_success (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List GatherOrder.Card) (pool : Pool)
    (accepted : gather target hasEvent fixedCards fixedCharacters cards = .ok pool) :
    CardValidation.validate (cards.map Card.row) = .ok () ∧
      pool.cards = sort target hasEvent fixedCards fixedCharacters cards ∧ Valid pool := by
  cases checked : CardValidation.validate (cards.map Card.row) with
  | error error => simp [gather, checked] at accepted
  | ok token =>
      cases token
      let ordered := sort target hasEvent fixedCards fixedCharacters cards
      cases calls : SkillGather.run SkillGather.initial (ordered.map (fun card => SkillGather.encodeInput card.skills)) with
      | error error => simp [gather, checked, ordered, calls] at accepted
      | ok result =>
          rcases result with ⟨tables, slots⟩
          dsimp only [ordered] at calls
          have same : Pool.mk ordered tables slots = pool := by
            simpa only [gather, checked, calls, Except.ok.injEq] using accepted
          subst pool
          have rowsValid := (CardValidation.validate_ok _).mp checked
          have sortedValid := sort_preserves_validation target hasEvent fixedCards fixedCharacters cards rowsValid
          have reference := SkillGather.concrete_from_empty (ordered.map Card.skills) (tables, slots)
            (by simpa only [List.map_map] using calls)
          refine ⟨rfl, rfl, sortedValid, reference.1, ?_, reference.2.2.1, ?_⟩
          · simpa only [List.forall₂_map_left_iff] using reference.2.1
          · simpa only [List.length_map] using reference.2.2.2

theorem validation_failure (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List GatherOrder.Card) (error : Error)
    (rejected : CardValidation.validate (cards.map Card.row) = .error error) :
    gather target hasEvent fixedCards fixedCharacters cards = .error error := by simp [gather, rejected]

theorem interner_failure (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List GatherOrder.Card) (error : Error)
    (checked : CardValidation.validate (cards.map Card.row) = .ok ())
    (rejected : SkillGather.run SkillGather.initial
      ((sort target hasEvent fixedCards fixedCharacters cards).map (fun card => SkillGather.encodeInput card.skills)) = .error error) :
    gather target hasEvent fixedCards fixedCharacters cards = .error error := by simp [gather, checked, rejected]

/-- A successful stream initializes every dense row of every mapped column;
slots have the same full length, independently of the optional 512-bit cache. -/
theorem successful_columns (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List GatherOrder.Card) (pool : Pool)
    (accepted : gather target hasEvent fixedCards fixedCharacters cards = .ok pool) :
    pool.cards.length ≤ 65535 ∧ pool.slots.length = pool.cards.length ∧
      ∀ index : Fin pool.cards.length,
        DenseWrites.fill pool.cards 0 (fun _ => none) index.val = some pool.cards[index] := by
  have valid := (gather_success target hasEvent fixedCards fixedCharacters cards pool accepted).2.2
  have count := ((CardTable.validate_iff _).mp valid.1).1
  exact ⟨by simpa only [List.length_map] using count, valid.2.2.2.2, DenseWrites.initialized_column pool.cards⟩

theorem row_domain (pool : Pool) (valid : Valid pool) (index : Fin pool.cards.length) :
    CardTable.Domain (pool.cards[index]).row :=
  CardTable.accepted_row _ valid.1 _
    (List.mem_map.mpr ⟨pool.cards[index], List.getElem_mem index.isLt, rfl⟩)

/-- Actual successful gather, followed by a carried-unit lookup, supplies all
encoding premises from its own validator. -/
theorem gather_power_rows (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List GatherOrder.Card) (pool : Pool)
    (accepted : gather target hasEvent fixedCards fixedCharacters cards = .ok pool)
    (index : Fin pool.cards.length) (unit : Fin 6) (member : Fin 4)
    (carried : (pool.cards[index]).row.unitMask.testBit unit.val = true) :
    let card := (pool.cards[index]).row
    let valid := row_domain pool (gather_success target hasEvent fixedCards fixedCharacters cards pool accepted).2.2 index
    PowerEncoding.decode (PowerEncoding.rowValues card valid)
      (PowerEncoding.profileWord (PowerEncoding.rowMask card valid))
      (PowerEncoding.slot (PowerEncoding.profile (PowerEncoding.rowMask card valid) unit) member) =
        (card.power unit member).toNat := by
  exact PowerEncoding.row_decode _ _ unit member carried

theorem gather_bonus_rows (target : Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
    (cards : List GatherOrder.Card) (pool : Pool)
    (accepted : gather target hasEvent fixedCards fixedCharacters cards = .ok pool)
    (before after : List CardTable.Input) (card : CardTable.Input)
    (position : pool.cards.map Card.row = before ++ card :: after) :
    let index := LimitedCode.code (CardTable.collected [] before) card.limitedBonus
    index ≤ 15 ∧ LimitedCode.decode (CardTable.collected [] (pool.cards.map Card.row)) index = card.limitedBonus := by
  have valid := (gather_success target hasEvent fixedCards fixedCharacters cards pool accepted).2.2.1
  rw [position] at valid ⊢
  exact LimitedCode.accepted_reference before after card valid

/-- A full internal gather example: admitted scalar fields and no optional
payload do not force the stored raw slot to be an ordinary skill. -/
def rawSlotCard (raw : Fin 256 × Fin 256) : Card :=
  { core := { publicId := 0
              character := 0
              attr := 0
              unitMask := 0
              power := fun _ _ => 0
              baseBonus := 0
              limitedBonus := 0
              skillMin := 0
              skillMax := 0 }
    powerMax := ⟨0, by decide, by decide⟩
    powerMin := ⟨0, by decide, by decide⟩
    trained := false
    skills := ⟨none, none, none, raw⟩ }

theorem tagged_raw_gather :
    CardValidation.validate [(rawSlotCard (1, 7)).row] = .ok () ∧
    gather .power false [] [] [rawSlotCard (1, 7)] =
      .ok ⟨[rawSlotCard (1, 7)], SkillGather.initial, [(1, 7)]⟩ ∧
    (SkillGather.initial : SkillGather.State SkillGather.Payload).stored 0 = [] := by
  have checked : CardValidation.validate [(rawSlotCard (1, 7)).row] = .ok () := rfl
  refine ⟨checked, ?_, rfl⟩
  have ordered : GatherOrder.sort .power false [] [] [rawSlotCard (1, 7)] = [rawSlotCard (1, 7)] := by
    simp [GatherOrder.sort]
  rw [gather, show [rawSlotCard (1, 7)].map Card.row = [(rawSlotCard (1, 7)).row] from rfl, checked, ordered]
  rfl

end GatherPipeline

namespace GatherImage
open GatherOrder

/-- Values written to the compact columns, with source-width casts retained. -/
def powerValues (pool : GatherPipeline.Pool) : List Nat :=
  pool.cards.map (fun card => card.powerMax.value.toNat % 4294967296)

def skillValues (pool : GatherPipeline.Pool) : List Nat :=
  pool.cards.map (fun card => card.row.skillMax.val % 256)

def bonusColumns (before : List CardTable.Input) : List CardTable.Input → List Nat
  | [] => []
  | card :: rest =>
      (LimitedCode.pack (card.baseBonus + card.limitedBonus)
        (LimitedCode.code (CardTable.collected [] before) card.limitedBonus) % 65536) ::
        bonusColumns (before ++ [card]) rest

def readBonus (table : List Nat) (word : Nat) : Nat × Nat :=
  let limited := LimitedCode.decode table (word &&& 15)
  ((word >>> 4) - limited, limited)

def bonusValues (pool : GatherPipeline.Pool) : List (Nat × Nat) :=
  let rows := pool.cards.map Card.row
  (bonusColumns [] rows).map (readBonus (CardTable.collected [] rows))

theorem powers_exact (pool : GatherPipeline.Pool) :
    powerValues pool = pool.cards.map (fun card => card.powerMax.value.toNat) := by
  simp only [powerValues, GatherOrder.nonnegative_u32_cast]

theorem skills_exact (pool : GatherPipeline.Pool) :
    skillValues pool = pool.cards.map (fun card => card.row.skillMax.val) := by
  unfold skillValues
  apply List.map_congr_left
  intro card _
  exact Nat.mod_eq_of_lt card.row.skillMax.isLt

theorem bonus_columns_exact (before rest : List CardTable.Input)
    (accepted : CardTable.validate (before ++ rest) = true) :
    (bonusColumns before rest).map (readBonus (CardTable.collected [] (before ++ rest))) =
      rest.map (fun card => (card.baseBonus, card.limitedBonus)) := by
  induction rest generalizing before with
  | nil => rfl
  | cons card rest ih =>
      have packed := LimitedCode.accepted_packing before rest card accepted
      have next : CardTable.validate ((before ++ [card]) ++ rest) = true := by
        simpa only [List.append_assoc, List.singleton_append] using accepted
      have remaining := ih (before ++ [card]) next
      simp only [List.append_assoc, List.singleton_append] at remaining
      simp only [bonusColumns, List.map_cons, packed.1, readBonus, packed.2.2.1,
        packed.2.2.2, Nat.add_sub_cancel_right]
      exact congrArg (List.cons (card.baseBonus, card.limitedBonus)) remaining

theorem bonuses_exact (pool : GatherPipeline.Pool) (valid : GatherPipeline.Valid pool) :
    bonusValues pool = pool.cards.map (fun card => (card.row.baseBonus, card.row.limitedBonus)) := by
  simpa only [bonusValues, List.nil_append, List.map_map] using
    bonus_columns_exact [] (pool.cards.map Card.row) valid.1

def multiRow (card : Card) : Fin 8 → Nat :=
  match card.multiPower with
  | none => fun _ => 0
  | some values => fun index => (values index).value.toNat % 4294967296

/-- Pool-wide allocation has one row per card. Rows without a mixed detail
retain the zero initialization of the first allocation. -/
def multiColumns (pool : GatherPipeline.Pool) : Option (List (Fin 8 → Nat)) :=
  if pool.cards.any (fun card => card.multiPower.isSome) then some (pool.cards.map multiRow) else none

theorem multi_absent (pool : GatherPipeline.Pool) :
    multiColumns pool = none ↔ ∀ card ∈ pool.cards, card.multiPower = none := by
  simp [multiColumns]

theorem multi_count (pool : GatherPipeline.Pool) (rows : List (Fin 8 → Nat))
    (present : multiColumns pool = some rows) : rows.length = pool.cards.length := by
  unfold multiColumns at present
  split at present
  · cases Option.some.inj present
    simp
  · cases present

theorem multi_column (pool : GatherPipeline.Pool) (rows : List (Fin 8 → Nat))
    (present : multiColumns pool = some rows) (index : Fin pool.cards.length) :
    rows[index.val]? = some (multiRow pool.cards[index]) := by
  unfold multiColumns at present
  split at present
  · cases Option.some.inj present
    simp
  · cases present

theorem multi_row_exact (card : Card) (values : Fin 8 → Signed32) (present : card.multiPower = some values) :
    ∀ index, multiRow card index = (values index).value.toNat := by
  intro index
  simp only [multiRow, present, GatherOrder.nonnegative_u32_cast]

/-- The indexed column is the result of sequential dense writes, not a
precondition asserting that the builder and its input happen to agree. -/
theorem power_column (pool : GatherPipeline.Pool) (index : Fin pool.cards.length) :
    DenseWrites.fill (powerValues pool) 0 (fun _ => none) index.val =
      some (pool.cards[index]).powerMax.value.toNat := by
  rw [powers_exact]
  simpa using DenseWrites.initialized_column
    (pool.cards.map (fun card => card.powerMax.value.toNat)) ⟨index.val, by simp⟩

theorem skill_column (pool : GatherPipeline.Pool) (index : Fin pool.cards.length) :
    DenseWrites.fill (skillValues pool) 0 (fun _ => none) index.val =
      some (pool.cards[index]).row.skillMax.val := by
  rw [skills_exact]
  simpa using DenseWrites.initialized_column
    (pool.cards.map (fun card => card.row.skillMax.val)) ⟨index.val, by simp⟩

end GatherImage

namespace NumericFlow
open GatherOrder

inductive Live where
  | solo | auto | multi | cheerful | challenge | challengeAuto | mysekai
  deriving DecidableEq

inductive Ceiling where
  | power | bonus | rate | live | eventInner | eventPoint
  deriving DecidableEq

/-- Results of the named arithmetic checks at the numeric-domain boundary.
This interface describes control flow, not the floating-point range or error
proof for the expressions producing those results. -/
structure Checks where
  base : Except Error Unit
  autoBase : Except Error Unit
  fever : Except Error Unit
  skillRates : List (List (Except Error Unit))
  supportDefault : List (Except Error Unit)
  supportByCharacter : List (List (Except Error Unit))
  ceiling : Ceiling → Except Error Unit

def initialChecks (checks : Checks) : List (Except Error Unit) :=
  [checks.base, checks.autoBase, checks.fever] ++ checks.skillRates.flatten ++
    checks.supportDefault ++ checks.supportByCharacter.flatten

def chain : Except Error Unit → Except Error Unit → Except Error Unit
  | .error error, _ => .error error
  | .ok (), next => next

@[simp] theorem chain_done (result : Except Error Unit) : chain result (.ok ()) = result := by
  cases result with
  | error error => rfl
  | ok token => cases token; rfl

def execute : List (Except Error Unit) → Except Error Unit
  | [] => .ok ()
  | check :: rest => chain check (execute rest)

theorem execute_append (before after : List (Except Error Unit)) :
    execute (before ++ after) = chain (execute before) (execute after) := by
  induction before with
  | nil => rfl
  | cons check rest ih =>
      cases check with
      | error error => rfl
      | ok token => cases token; exact ih

theorem execute_ok (checks : List (Except Error Unit)) :
    execute checks = .ok () ↔ ∀ check ∈ checks, check = .ok () := by
  induction checks with
  | nil => simp [execute]
  | cons check rest ih =>
      cases check with
      | error error => simp [execute, chain]
      | ok token => cases token; simpa [execute, chain] using ih

/-- Exact early-return order of numeric_domain, after its finite/nonnegative
loops and the uncapped power check. -/
def run (target : Target) (hasEvent : Bool) (live : Live) (checks : Checks) : Except Error Unit :=
  chain (execute (initialChecks checks)) (chain (checks.ceiling .power)
    (if target = .power ∨ target = .skill then .ok () else
      chain (checks.ceiling .bonus)
        (if target = .mysekai then .ok () else
          chain (checks.ceiling .rate) (chain (checks.ceiling .live)
            (if target ≠ .score ∨ hasEvent = false then .ok () else
              match live with
              | .solo | .auto | .multi | .cheerful =>
                  chain (checks.ceiling .eventInner) (checks.ceiling .eventPoint)
              | _ => .ok ())))))

def required (target : Target) (hasEvent : Bool) (live : Live) : List Ceiling :=
  if target = .power ∨ target = .skill then [.power]
  else if target = .mysekai then [.power, .bonus]
  else [.power, .bonus, .rate, .live] ++
    if target = .score ∧ hasEvent = true then
      match live with
      | .solo | .auto | .multi | .cheerful => [.eventInner, .eventPoint]
      | _ => []
    else []

theorem source_order (target : Target) (hasEvent : Bool) (live : Live) (checks : Checks) :
    run target hasEvent live checks =
      execute (initialChecks checks ++ (required target hasEvent live).map checks.ceiling) := by
  cases target <;> cases hasEvent <;> cases live <;>
    simp [run, required, execute_append, execute]

theorem successful_checks (target : Target) (hasEvent : Bool) (live : Live) (checks : Checks)
    (accepted : run target hasEvent live checks = .ok ()) :
    (∀ check ∈ initialChecks checks, check = .ok ()) ∧
      ∀ ceiling ∈ required target hasEvent live, checks.ceiling ceiling = .ok () := by
  rw [source_order, execute_ok] at accepted
  simpa only [List.forall_mem_append, List.forall_mem_map] using accepted

theorem first_failure (before after : List (Except Error Unit)) (error : Error)
    (passed : execute before = .ok ()) :
    execute (before ++ .error error :: after) = .error error := by
  rw [execute_append, passed]
  rfl

end NumericFlow

namespace BuildPipeline
open GatherOrder

/-- The numeric-domain check follows completed gather. Its expression
implementation is a separate arithmetic obligation; no claim about a search
ceiling is obtained merely by accepting its result. -/
def build (target : Target) (hasEvent : Bool) (live : NumericFlow.Live)
    (fixedCards fixedCharacters : List Nat) (cards : List GatherOrder.Card)
    (numericChecks : GatherPipeline.Pool → NumericFlow.Checks) : Except Error GatherPipeline.Pool :=
  match GatherPipeline.gather target hasEvent fixedCards fixedCharacters cards with
  | .error error => .error error
  | .ok pool =>
      match NumericFlow.run target hasEvent live (numericChecks pool) with
      | .error error => .error error
      | .ok () => .ok pool

theorem successful_build (target : Target) (hasEvent : Bool) (live : NumericFlow.Live)
    (fixedCards fixedCharacters : List Nat) (cards : List GatherOrder.Card)
    (numericChecks : GatherPipeline.Pool → NumericFlow.Checks) (pool : GatherPipeline.Pool)
    (accepted : build target hasEvent live fixedCards fixedCharacters cards numericChecks = .ok pool) :
    GatherPipeline.gather target hasEvent fixedCards fixedCharacters cards = .ok pool ∧
      GatherPipeline.Valid pool ∧ NumericFlow.run target hasEvent live (numericChecks pool) = .ok () ∧
      (∀ check ∈ NumericFlow.initialChecks (numericChecks pool), check = .ok ()) ∧
      ∀ ceiling ∈ NumericFlow.required target hasEvent live, (numericChecks pool).ceiling ceiling = .ok () := by
  cases gathered : GatherPipeline.gather target hasEvent fixedCards fixedCharacters cards with
  | error error => simp [build, gathered] at accepted
  | ok constructed =>
      cases numeric : NumericFlow.run target hasEvent live (numericChecks constructed) with
      | error error => simp [build, gathered, numeric] at accepted
      | ok token =>
          cases token
          have same : constructed = pool := by simpa [build, gathered, numeric] using accepted
          subst pool
          have valid := (GatherPipeline.gather_success target hasEvent fixedCards fixedCharacters cards constructed gathered).2.2
          have checks := NumericFlow.successful_checks target hasEvent live (numericChecks constructed) numeric
          exact ⟨rfl, valid, numeric, checks⟩

theorem gather_failure (target : Target) (hasEvent : Bool) (live : NumericFlow.Live)
    (fixedCards fixedCharacters : List Nat) (cards : List GatherOrder.Card)
    (numericChecks : GatherPipeline.Pool → NumericFlow.Checks) (error : Error)
    (rejected : GatherPipeline.gather target hasEvent fixedCards fixedCharacters cards = .error error) :
    build target hasEvent live fixedCards fixedCharacters cards numericChecks = .error error := by
  simp [build, rejected]

theorem numeric_failure (target : Target) (hasEvent : Bool) (live : NumericFlow.Live)
    (fixedCards fixedCharacters : List Nat) (cards : List GatherOrder.Card)
    (numericChecks : GatherPipeline.Pool → NumericFlow.Checks) (pool : GatherPipeline.Pool) (error : Error)
    (gathered : GatherPipeline.gather target hasEvent fixedCards fixedCharacters cards = .ok pool)
    (rejected : NumericFlow.run target hasEvent live (numericChecks pool) = .error error) :
    build target hasEvent live fixedCards fixedCharacters cards numericChecks = .error error := by
  simp [build, gathered, rejected]

end BuildPipeline

namespace SourceNumeric
open GatherOrder

inductive EventKind where
  | marathon | cheerful | worldBloom
  deriving DecidableEq

structure SupportProfile where
  bonuses : List Binary64.Value
  count : Fin 256

/-- Fields read by numeric_domain. Signed teammate and life fields retain
their source representation; request validation is a separate operation. -/
structure Context where
  target : Target
  eventKind : Option EventKind
  live : NumericFlow.Live
  fixedCards : List (Fin 65536)
  fixedCharacters : List (Fin 256)
  base : Binary64.Value
  autoBase : Binary64.Value
  fever : Binary64.Value
  skillRates : Fin 3 → Fin 6 → Binary64.Value
  supportDefault : SupportProfile
  supportByCharacter : List SupportProfile
  honor : Fin 4294967296
  powerCap : Option (Fin 4294967296)
  attributeBonus : Fin 6 → Fin 65536
  extraBonus : Fin 4294967296
  teammateScore : Option Signed32
  teammatePower : Option Signed32
  life : Signed32
  musicRate : Fin 4294967296
  boostRate : Fin 4294967296

def Context.hasEvent (context : Context) : Bool := context.eventKind.isSome

def Context.effectiveLive (context : Context) : NumericFlow.Live :=
  if context.live = .multi ∧ context.eventKind = some .cheerful then .cheerful else context.live

def isMulti (live : NumericFlow.Live) : Bool := decide (live = .multi ∨ live = .cheerful)

def greatest (values : List Nat) : Nat := values.foldl max 0

def greatestFive (values : List Nat) : Nat :=
  ((values.mergeSort (fun a b => decide (b ≤ a))).take 5).sum

theorem sum_bounded (values : List Nat) (bound : Nat) (bounded : ∀ value ∈ values, value ≤ bound) :
    values.sum ≤ values.length * bound := by
  induction values with
  | nil => simp
  | cons value rest ih =>
      have first := bounded value (by simp)
      have later := ih (fun item member => bounded item (by simp [member]))
      simp only [List.sum_cons, List.length_cons]
      nlinarith

theorem greatest_five_bounded (values : List Nat) (bound : Nat)
    (bounded : ∀ value ∈ values, value ≤ bound) : greatestFive values ≤ 5 * bound := by
  let ordered := values.mergeSort (fun a b => decide (b ≤ a))
  have limited : ∀ value ∈ ordered.take 5, value ≤ bound := by
    intro value member
    exact bounded value ((List.mergeSort_perm _ _).mem_iff.mp (List.mem_of_mem_take member))
  have total := sum_bounded (ordered.take 5) bound limited
  have count : (ordered.take 5).length ≤ 5 := by simp
  exact total.trans (Nat.mul_le_mul_right bound count)

def leaderBonus (pool : GatherPipeline.Pool) (index : Nat) : Nat :=
  Arithmetic.ceilDiv ((((pool.cards.map Card.leaderHonor)[index]?).map Fin.val).getD 0 +
    (((pool.cards.map Card.leaderLimit)[index]?).map Fin.val).getD 0) 10

def rawPower (pool : GatherPipeline.Pool) (context : Context) : Nat :=
  greatestFive (GatherImage.powerValues pool) + context.honor.val

theorem raw_power_u64 (pool : GatherPipeline.Pool) (context : Context) :
    rawPower pool context < 2 ^ 64 := by
  have bound := greatest_five_bounded (GatherImage.powerValues pool) 2147483647 (by
    rw [GatherImage.powers_exact]
    intro value member
    obtain ⟨card, _, rfl⟩ := List.mem_map.mp member
    have bound := card.powerMax.upper
    omega)
  have honorBound := context.honor.isLt
  unfold rawPower
  omega

def cardBonuses (pool : GatherPipeline.Pool) : List Nat :=
  (GatherImage.bonusValues pool).map (fun value =>
    Arithmetic.ceilDiv value.1 10 + Arithmetic.ceilDiv value.2 10)

theorem card_bonuses_bounded (pool : GatherPipeline.Pool) (valid : GatherPipeline.Valid pool) :
    ∀ value ∈ cardBonuses pool, value ≤ 4095 := by
  rw [cardBonuses, GatherImage.bonuses_exact pool valid, List.map_map]
  intro value member
  obtain ⟨card, cardMember, rfl⟩ := List.mem_map.mp member
  have domain := CardTable.accepted_row (pool.cards.map Card.row) valid.1 card.row
    (List.mem_map.mpr ⟨card, cardMember, rfl⟩)
  have total := domain.2.2.2.2.2.2.2.1
  have base := Arithmetic.ceilDiv_minimal card.row.baseBonus 10 card.row.baseBonus (by decide) (by omega)
  have limited := Arithmetic.ceilDiv_minimal card.row.limitedBonus 10 card.row.limitedBonus (by decide) (by omega)
  dsimp only [Function.comp_def]
  omega

theorem bonus_sum_u64 (pool : GatherPipeline.Pool) (valid : GatherPipeline.Valid pool) :
    greatestFive (cardBonuses pool) < 2 ^ 64 := by
  have bound := greatest_five_bounded (cardBonuses pool) 4095 (card_bonuses_bounded pool valid)
  omega

noncomputable def power (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  Binary64.ofNat (match context.powerCap with
    | none => rawPower pool context
    | some cap => min (rawPower pool context) cap.val)

def skillPeak (pool : GatherPipeline.Pool) : Nat := greatest (GatherImage.skillValues pool)

noncomputable def supportSum (context : Context) : Binary64.Value :=
  Binary64.sum ((context.supportDefault :: context.supportByCharacter).map
    (fun profile => Binary64.sortedSum profile.bonuses profile.count.val))

/-- The additions occur in the same left-associated order as the source. -/
noncomputable def bonus (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  let leaders := (List.range pool.cards.length).map (leaderBonus pool)
  let diversity := List.ofFn (fun index : Fin 6 => (context.attributeBonus index).val)
  Binary64.add (Binary64.add (Binary64.add (Binary64.add
    (Binary64.ofNat (greatestFive (cardBonuses pool))) (Binary64.ofNat (greatest leaders)))
    (Binary64.ofNat (greatest diversity))) (Binary64.ceil (supportSum context)))
    (Binary64.ofNat context.extraBonus.val)

noncomputable def baseAndRates (context : Context) : Binary64.Value × List Binary64.Value :=
  match context.effectiveLive with
  | .auto | .challengeAuto => (context.autoBase, List.ofFn (context.skillRates 2))
  | .multi | .cheerful =>
      (Binary64.add context.base (Binary64.mul context.fever (Binary64.round (1 / 2 : ℚ))),
        List.ofFn (context.skillRates 1))
  | _ => (context.base, List.ofFn (context.skillRates 0))

noncomputable def slotPeak (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  let peak := Binary64.ofNat (skillPeak pool)
  if isMulti context.effectiveLive then
    Binary64.maximum (Binary64.divNat (Binary64.mul peak (Binary64.ofNat 9)) 5)
      (Binary64.ofInt ((context.teammateScore.map Signed32.value).getD 0))
  else peak

noncomputable def rate (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  let pair := baseAndRates context
  Binary64.add pair.1 (Binary64.divNat (Binary64.mul (slotPeak pool context) (Binary64.sum pair.2)) 100)

noncomputable def active (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  if !isMulti context.effectiveLive then Binary64.zero else
    let coefficient := Binary64.round (3 / 40 : ℚ)
    match context.teammatePower with
    | some teammate => Binary64.mul coefficient (Binary64.add (power pool context)
        (Binary64.mul (Binary64.ofNat 4) (Binary64.ofInt teammate.value)))
    | none => Binary64.mul (Binary64.mul coefficient (Binary64.ofNat 5)) (power pool context)

noncomputable def liveScore (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  Binary64.add (Binary64.mul (Binary64.mul (Binary64.ofNat 4) (power pool context)) (rate pool context))
    (active pool context)

noncomputable def eventBase (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  match context.effectiveLive with
  | .solo | .auto => Binary64.add (Binary64.ofNat 100) (Binary64.divNat (liveScore pool context) 20000)
  | .multi | .cheerful => Binary64.add (Binary64.ofNat 123) (Binary64.divNat (liveScore pool context) 17000)
  | _ => Binary64.zero

noncomputable def eventInner (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  Binary64.divNat (Binary64.mul (Binary64.mul (eventBase pool context) (Binary64.ofNat context.musicRate.val))
    (Binary64.add (bonus pool context) (Binary64.ofNat 100))) 10000

noncomputable def lifeRate (context : Context) : Binary64.Value :=
  if context.effectiveLive = .cheerful then
    Binary64.divNat (Binary64.ofInt (5750 + min 1000 (max 500 context.life.value))) 5000
  else Binary64.ofNat 1

noncomputable def eventPoint (pool : GatherPipeline.Pool) (context : Context) : Binary64.Value :=
  Binary64.divNat (Binary64.mul (Binary64.mul (eventInner pool context) (lifeRate context))
    (Binary64.ofNat context.boostRate.val)) 100

def nonnegativeCheck (field : Field) (value : Binary64.Value) : Except Error Unit :=
  if Binary64.nonnegative value then .ok () else .error (.invalidNumber field)

noncomputable def maximumCheck (field : Field) (value : Binary64.Value) (maximum : Nat) : Except Error Unit :=
  if Binary64.le value (Binary64.ofNat maximum) then .ok () else
    .error (.capacity field
      (Binary64.toU64 (Binary64.minimum (Binary64.ceil value) (Binary64.ofNat (2 ^ 64 - 1)))) maximum)

noncomputable def ceilingValue (pool : GatherPipeline.Pool) (context : Context) : NumericFlow.Ceiling → Binary64.Value
  | .power => Binary64.ofNat (rawPower pool context)
  | .bonus => bonus pool context
  | .rate => rate pool context
  | .live => liveScore pool context
  | .eventInner => eventInner pool context
  | .eventPoint => eventPoint pool context

def ceilingLimit : NumericFlow.Ceiling → Nat
  | .power => 2 ^ 24
  | .bonus => 2 ^ 20
  | .rate => 2 ^ 16
  | .live => 2 ^ 27
  | .eventInner => 2 ^ 27
  | .eventPoint => 2 ^ 30

def ceilingField : NumericFlow.Ceiling → Field
  | .power => .deckPower
  | .bonus => .bonusCeiling
  | .rate => .liveRate
  | .live => .liveScore
  | .eventInner => .eventInner
  | .eventPoint => .eventPoint

/-- Concrete pool/context construction replaces the arbitrary check-result
callback. Rounding is explicit at every source floating operation. -/
noncomputable def checks (pool : GatherPipeline.Pool) (context : Context) : NumericFlow.Checks :=
  { base := nonnegativeCheck .baseScore context.base
    autoBase := nonnegativeCheck .autoBaseScore context.autoBase
    fever := nonnegativeCheck .feverScore context.fever
    skillRates := List.ofFn (fun row : Fin 3 => List.ofFn
      (fun slot : Fin 6 => nonnegativeCheck .skillScoreRate (context.skillRates row slot)))
    supportDefault := context.supportDefault.bonuses.map (nonnegativeCheck .supportBonus)
    supportByCharacter := context.supportByCharacter.map
      (fun profile => profile.bonuses.map (nonnegativeCheck .supportBonus))
    ceiling := fun ceiling => maximumCheck (ceilingField ceiling) (ceilingValue pool context ceiling) (ceilingLimit ceiling) }

/-- Every scalar in the mandatory finite/nonnegative loops, in source order. -/
def initialInputs (context : Context) : List (Field × Binary64.Value) :=
  [(.baseScore, context.base), (.autoBaseScore, context.autoBase), (.feverScore, context.fever)] ++
    (List.ofFn (fun row : Fin 3 => List.ofFn
      (fun slot : Fin 6 => (Field.skillScoreRate, context.skillRates row slot)))).flatten ++
    context.supportDefault.bonuses.map (fun value => (Field.supportBonus, value)) ++
    (context.supportByCharacter.map (fun profile =>
      profile.bonuses.map (fun value => (Field.supportBonus, value)))).flatten

theorem initial_checks (pool : GatherPipeline.Pool) (context : Context) :
    NumericFlow.initialChecks (checks pool context) =
      (initialInputs context).map (fun entry => nonnegativeCheck entry.1 entry.2) := by
  simp [NumericFlow.initialChecks, checks, initialInputs, Function.comp_def]

noncomputable def numericDomain (pool : GatherPipeline.Pool) (context : Context) : Except Error Unit :=
  NumericFlow.run context.target context.hasEvent context.effectiveLive (checks pool context)

theorem nonnegative_check_iff (field : Field) (value : Binary64.Value) :
    nonnegativeCheck field value = .ok () ↔ Binary64.nonnegative value = true := by
  unfold nonnegativeCheck
  split <;> simp_all

theorem maximum_check_iff (field : Field) (value : Binary64.Value) (maximum : Nat) :
    maximumCheck field value maximum = .ok () ↔ Binary64.le value (Binary64.ofNat maximum) = true := by
  unfold maximumCheck
  split <;> simp_all

theorem nonnegative_failure (field : Field) (value : Binary64.Value)
    (rejected : Binary64.nonnegative value = false) :
    nonnegativeCheck field value = .error (.invalidNumber field) := by
  simp [nonnegativeCheck, rejected]

theorem maximum_failure (field : Field) (value : Binary64.Value) (maximum : Nat)
    (rejected : Binary64.le value (Binary64.ofNat maximum) = false) :
    maximumCheck field value maximum = .error (.capacity field
      (Binary64.toU64 (Binary64.minimum (Binary64.ceil value) (Binary64.ofNat (2 ^ 64 - 1)))) maximum) := by
  simp [maximumCheck, rejected]

def Accepted (pool : GatherPipeline.Pool) (context : Context) : Prop :=
  (∀ entry ∈ initialInputs context, Binary64.nonnegative entry.2 = true) ∧
  ∀ ceiling ∈ NumericFlow.required context.target context.hasEvent context.effectiveLive,
    Binary64.le (ceilingValue pool context ceiling) (Binary64.ofNat (ceilingLimit ceiling)) = true

theorem numeric_iff (pool : GatherPipeline.Pool) (context : Context) :
    numericDomain pool context = .ok () ↔ Accepted pool context := by
  rw [numericDomain, NumericFlow.source_order, NumericFlow.execute_ok, initial_checks]
  simp only [List.forall_mem_append, List.forall_mem_map, nonnegative_check_iff,
    checks, maximum_check_iff, Accepted]

theorem successful_ceiling (pool : GatherPipeline.Pool) (context : Context)
    (accepted : numericDomain pool context = .ok ()) (ceiling : NumericFlow.Ceiling)
    (required : ceiling ∈ NumericFlow.required context.target context.hasEvent context.effectiveLive) :
    Binary64.le (ceilingValue pool context ceiling) (Binary64.ofNat (ceilingLimit ceiling)) = true := by
  have passed := (NumericFlow.successful_checks context.target context.hasEvent context.effectiveLive
    (checks pool context) accepted).2 ceiling required
  exact (maximum_check_iff _ _ _).mp passed

noncomputable def build (cards : List GatherOrder.Card) (context : Context) : Except Error GatherPipeline.Pool :=
  BuildPipeline.build context.target context.hasEvent context.effectiveLive
    (context.fixedCards.map Fin.val) (context.fixedCharacters.map Fin.val) cards
    (fun pool => checks pool context)

theorem concrete_success (cards : List GatherOrder.Card) (context : Context) (pool : GatherPipeline.Pool)
    (accepted : build cards context = .ok pool) :
    GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
      (context.fixedCharacters.map Fin.val) cards = .ok pool ∧
    GatherPipeline.Valid pool ∧ numericDomain pool context = .ok () ∧
    ∀ ceiling ∈ NumericFlow.required context.target context.hasEvent context.effectiveLive,
      Binary64.le (ceilingValue pool context ceiling) (Binary64.ofNat (ceilingLimit ceiling)) = true := by
  have complete := BuildPipeline.successful_build context.target context.hasEvent context.effectiveLive
    (context.fixedCards.map Fin.val) (context.fixedCharacters.map Fin.val) cards (fun pool => checks pool context) pool accepted
  exact ⟨complete.1, complete.2.1, complete.2.2.1,
    fun ceiling required => successful_ceiling pool context complete.2.2.1 ceiling required⟩

theorem build_success_iff (cards : List GatherOrder.Card) (context : Context) (pool : GatherPipeline.Pool) :
    build cards context = .ok pool ↔
      GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
        (context.fixedCharacters.map Fin.val) cards = .ok pool ∧ Accepted pool context := by
  constructor
  · intro accepted
    have complete := concrete_success cards context pool accepted
    exact ⟨complete.1, (numeric_iff pool context).mp complete.2.2.1⟩
  · rintro ⟨gathered, accepted⟩
    have numeric := (numeric_iff pool context).mpr accepted
    unfold numericDomain at numeric
    simp [build, BuildPipeline.build, gathered, numeric]

theorem build_failure_iff (cards : List GatherOrder.Card) (context : Context) (error : Error) :
    build cards context = .error error ↔
      GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
        (context.fixedCharacters.map Fin.val) cards = .error error ∨
      ∃ pool, GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
        (context.fixedCharacters.map Fin.val) cards = .ok pool ∧ numericDomain pool context = .error error := by
  cases gathered : GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
      (context.fixedCharacters.map Fin.val) cards with
  | error rejected => simp [build, BuildPipeline.build, gathered]
  | ok pool =>
      cases numeric : NumericFlow.run context.target context.hasEvent context.effectiveLive (checks pool context) with
      | error rejected => simp [build, BuildPipeline.build, gathered, numericDomain, numeric]
      | ok token =>
          cases token
          simp [build, BuildPipeline.build, gathered, numericDomain, numeric]

end SourceNumeric

end Allium.Admission
