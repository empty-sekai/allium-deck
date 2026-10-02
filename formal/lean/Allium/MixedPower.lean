import Allium.DeckComposition
import Allium.Admission

/-!
# Effective power with an optional mixed-unit sidecar

The legacy two-profile table and the mixed-unit table are different objects.
The latter is indexed by original-unit all-match, support-unit all-match, and
attribute all-match. All tables may be non-monotone. Bounds enumerate admissible
states rather than assuming that activating an area effect increases power.
-/
namespace Allium.MixedPower
open PowerModel DeckComposition

inductive Mode where
  | byDeck
  | forceOn
  | forceOff
  deriving DecidableEq, Repr

def enabled : Mode → Bool → Bool
  | .byDeck, mixed => mixed
  | .forceOn, _ => true
  | .forceOff, _ => false

structure Card where
  facts : CardFacts
  profile : UnitId → Bool
  legacyValues : Fin 8 → Nat
  multiValues : Option (Fin 8 → Nat)
  /-- Empty raw membership has no original/support unit. The facts payload is
  ignored for unit composition in this case, but its attribute remains valid. -/
  emptyMask : Bool := false

def legacy (card : Card) : CardData :=
  ⟨card.facts.attr, if card.emptyMask then ∅ else membership card.facts, card.profile, card.legacyValues⟩

def rawMembership (mask : Fin 64) : Finset UnitId :=
  Finset.univ.filter (fun unit => mask.val.testBit unit.val = true)

def rawOriginal (mask : Fin 64) : Option UnitId :=
  if mask.val.testBit 5 then some piapro
  else (List.finRange 6).find? (fun unit => mask.val.testBit unit.val)

def rawSupport (mask : Fin 64) : Option UnitId :=
  (List.finRange 6).find? (fun unit =>
    mask.val.testBit unit.val && decide (some unit ≠ rawOriginal mask))

def rawFacts (mask : Fin 64) (attr : Attribute) : CardFacts :=
  ⟨(rawOriginal mask).getD ⟨0, by decide⟩, rawSupport mask, attr⟩

/-- The constructor derives empty membership from the raw six-bit mask; it
is not an extra precondition supplied by a bound caller. -/
def fromRaw (mask : Fin 64) (attr : Attribute) (profile : UnitId → Bool)
    (values : Fin 8 → Nat) (multi : Option (Fin 8 → Nat)) : Card :=
  { facts := rawFacts mask attr
    profile := profile
    legacyValues := values
    multiValues := multi
    emptyMask := decide (mask.val = 0) }

/-- The bit expression includes original_unit(0)=0, not unit ID zero. The
finite proof checks every admitted six-bit mask using the kernel evaluator. -/
theorem raw_original_bits : ∀ mask : Fin 64,
    ((rawOriginal mask).map (fun unit => 2 ^ unit.val)).getD 0 =
      if mask.val.testBit 5 then 32 else mask.val &&& ((256 - mask.val) % 256) := by
  decide

theorem raw_population_exact : ∀ mask : Fin 64,
    Admission.CardTable.population mask.val = (rawMembership mask).card := by
  decide

theorem raw_support_bits : ∀ mask : Fin 64, (rawMembership mask).card ≤ 2 →
    ((rawSupport mask).map (fun unit => 2 ^ unit.val)).getD 0 =
      mask.val &&& (255 - ((rawOriginal mask).map (fun unit => 2 ^ unit.val)).getD 0) := by
  decide

/-- Two optional unit fields encode exactly the accepted zero-, one-, and
two-bit masks, including the otherwise-unused all-zero case. -/
theorem raw_membership_exact : ∀ mask : Fin 64, (rawMembership mask).card ≤ 2 →
    (if mask.val = 0 then ∅ else membership (rawFacts mask ⟨0, by decide⟩)) = rawMembership mask := by
  decide

theorem from_raw_units (mask : Fin 64) (attr : Attribute) (profile : UnitId → Bool)
    (values : Fin 8 → Nat) (multi : Option (Fin 8 → Nat))
    (capacity : Admission.CardTable.population mask.val ≤ 2) :
    (legacy (fromRaw mask attr profile values multi)).units = rawMembership mask := by
  have bound : (rawMembership mask).card ≤ 2 := by rwa [← raw_population_exact mask]
  simpa only [legacy, fromRaw, decide_eq_true_eq, rawFacts, membership] using raw_membership_exact mask bound

@[simp] theorem from_raw_empty (mask : Fin 64) (attr : Attribute) (profile : UnitId → Bool)
    (values : Fin 8 → Nat) (multi : Option (Fin 8 → Nat)) :
    (fromRaw mask attr profile values multi).emptyMask = true ↔ mask.val = 0 := by
  simp [fromRaw]

def originalAll (card : Card) (common : Finset UnitId) : Bool :=
  if card.emptyMask then false else decide (card.facts.original ∈ common)

def supportAll (card : Card) (common : Finset UnitId) : Bool :=
  if card.emptyMask then false else
    match card.facts.support with
    | none => false
    | some unit => decide (unit ∈ common)

/-- Empty masks contribute no original unit, no VS support pass, and no
shuffle unit. They are not reinterpreted as the first ordinary unit. -/
def activeFact (card : Card) : Option CardFacts :=
  if card.emptyMask then none else some card.facts

def effective (mode : Mode) (mixed : Bool) (common : Finset UnitId)
    (attr : Bool) (card : Card) : Nat :=
  match card.multiValues with
  | none => resolved (legacy card) common attr
  | some values =>
      if enabled mode mixed then
        values (tableIndex (originalAll card common) (supportAll card common) attr)
      else resolved (legacy card) common attr

/-- With no owned mixed-unit effects the complete mode selector is inert. -/
theorem no_multi_mode_invariant (first second : Mode) (mixed : Bool)
    (common : Finset UnitId) (attr : Bool) (card : Card)
    (h : card.multiValues = none) :
    effective first mixed common attr card = effective second mixed common attr card := by
  simp [effective, h]

@[simp] theorem force_off_legacy (mixed : Bool) (common : Finset UnitId)
    (attr : Bool) (card : Card) :
    effective .forceOff mixed common attr card = resolved (legacy card) common attr := by
  unfold effective
  cases card.multiValues <;> simp [enabled]

/-- Universal bounds retain the legacy entries and all optional sidecar entries. -/
def maximum (card : Card) : Nat :=
  match card.multiValues with
  | none => tableMax (legacy card)
  | some values => max (tableMax (legacy card)) (Finset.univ.sup values)

/-- A zero membership mask performs an empty legacy scan, whose value is zero
regardless of the eight stored entries. -/
def legacyMinimum (card : Card) : Nat :=
  if card.emptyMask then 0 else tableMin (legacy card)

def minimum (card : Card) : Nat :=
  match card.multiValues with
  | none => legacyMinimum card
  | some values => min (legacyMinimum card)
      (Finset.univ.inf' Finset.univ_nonempty values)

/-- ForceOff excludes the sidecar from the production minimum; the other
policies retain both the legacy and optional sidecar state families. -/
def minimumFor (mode : Mode) (card : Card) : Nat :=
  match mode with
  | .forceOff => legacyMinimum card
  | _ => minimum card

theorem legacy_units_nonempty (card : Card) (present : card.emptyMask = false) :
    (legacy card).units.Nonempty := by
  exact ⟨card.facts.original, by simp [legacy, membership, present]⟩

theorem empty_legacy_zero (card : Card) (empty : card.emptyMask = true)
    (common : Finset UnitId) (attr : Bool) : resolved (legacy card) common attr = 0 := by
  simp [resolved, legacy, empty]

theorem legacy_lower (card : Card) (common : Finset UnitId) (attr : Bool) :
    legacyMinimum card ≤ resolved (legacy card) common attr := by
  by_cases empty : card.emptyMask = true
  · simp [legacyMinimum, empty]
  · have present := Bool.eq_false_iff.mpr empty
    simpa only [legacyMinimum, if_neg empty] using
      resolved_lower (legacy card) (legacy_units_nonempty card present) common attr

theorem empty_minimum_zero (mode : Mode) (card : Card) (empty : card.emptyMask = true) :
    minimumFor mode card = 0 := by
  cases mode <;> simp only [minimumFor, minimum, legacyMinimum, empty, ↓reduceIte]
  all_goals cases card.multiValues <;> simp

theorem effective_upper (mode : Mode) (mixed : Bool) (common : Finset UnitId)
    (attr : Bool) (card : Card) :
    effective mode mixed common attr card ≤ maximum card := by
  cases hv : card.multiValues with
  | none => simpa only [effective, maximum, hv] using resolved_upper (legacy card) common attr
  | some values =>
      simp only [effective, maximum, hv]
      by_cases he : enabled mode mixed = true
      · simp only [if_pos he]
        exact (Finset.le_sup (f := values) (Finset.mem_univ _)).trans (le_max_right _ _)
      · simp only [if_neg he]
        exact (resolved_upper _ _ _).trans (le_max_left _ _)

theorem effective_lower (mode : Mode) (mixed : Bool) (common : Finset UnitId)
    (attr : Bool) (card : Card) :
    minimum card ≤ effective mode mixed common attr card := by
  cases hv : card.multiValues with
  | none => simpa only [effective, minimum, hv] using legacy_lower card common attr
  | some values =>
      simp only [effective, minimum, hv]
      by_cases he : enabled mode mixed = true
      · simp only [if_pos he]
        exact (min_le_right _ _).trans (Finset.inf'_le values (Finset.mem_univ _))
      · simp only [if_neg he]
        exact (min_le_left _ _).trans (legacy_lower card common attr)

theorem effective_lower_for (mode : Mode) (mixed : Bool) (common : Finset UnitId)
    (attr : Bool) (card : Card) :
    minimumFor mode card ≤ effective mode mixed common attr card := by
  cases mode with
  | byDeck => exact effective_lower .byDeck mixed common attr card
  | forceOn => exact effective_lower .forceOn mixed common attr card
  | forceOff => simpa only [minimumFor, force_off_legacy] using legacy_lower card common attr

variable {Id : Type*}

def deckFacts (data : Id → Card) (deck : List Id) : List CardFacts :=
  deck.filterMap (fun id => activeFact (data id))

abbrev Regime := Composition.Regime × Bool

def Matches (data : Id → Card) (deck : List Id) (r : Regime) : Prop :=
  Composition.Matches (fun id => legacy (data id)) deck r.1 ∧
    isMultiUnit (deckFacts data deck) = r.2

theorem regimes_cover (data : Id → Card) (deck : List Id) :
    ∃ r : Regime, Matches data deck r := by
  obtain ⟨r, hr⟩ := Composition.regimes_cover (fun id => legacy (data id)) deck
  exact ⟨(r, isMultiUnit (deckFacts data deck)), hr, rfl⟩

def multiBound (values : Fin 8 → Nat) (r : Composition.Regime) : Nat :=
  (Composition.unitFlags r).sup (fun original =>
    (Composition.unitFlags r).sup (fun support =>
      values (tableIndex original support (Composition.attrFlag r))))

/-- Four member-key bits used by the production regime enum. -/
def memberMask (r : Composition.Regime) : Nat :=
  if r.1.isSome then (if r.2.isSome then 10 else 5)
  else if r.2.isSome then 2 else 1

def stateAllowed (r : Composition.Regime) (state : Fin 8) : Bool :=
  (memberMask r).testBit ((if state.val &&& 6 = 0 then 0 else 2) + (state.val &&& 1))

theorem encoded_state_allowed : ∀ (r : Composition.Regime) (original support attr : Bool),
    stateAllowed r (tableIndex original support attr) = true ↔
      original ∈ Composition.unitFlags r ∧ support ∈ Composition.unitFlags r ∧
        attr = Composition.attrFlag r := by
  decide

def filteredMultiBound (values : Fin 8 → Nat) (r : Composition.Regime) : Nat :=
  (Finset.univ.filter (fun state => stateAllowed r state = true)).sup values

/-- The separate original/support flag suprema equal the actual eight-state
filter, whose member-key unit bit is the OR of original and support bits. -/
theorem multi_bound_eq_filtered (values : Fin 8 → Nat) (r : Composition.Regime) :
    multiBound values r = filteredMultiBound values r := by
  unfold multiBound filteredMultiBound
  apply Nat.le_antisymm
  · apply Finset.sup_le
    intro original ho
    apply Finset.sup_le
    intro support hs
    exact Finset.le_sup (f := values) (Finset.mem_filter.mpr
      ⟨Finset.mem_univ _, (encoded_state_allowed r original support _).mpr ⟨ho, hs, rfl⟩⟩)
  · apply Finset.sup_le
    intro state selected
    obtain ⟨original, support, attr, same⟩ := tableIndex_surjective state
    subst state
    obtain ⟨ho, hs, ha⟩ := (encoded_state_allowed r original support attr).mp
      (Finset.mem_filter.mp selected).2
    subst attr
    exact (Finset.le_sup (f := fun support => values (tableIndex original support (Composition.attrFlag r))) hs).trans
      (Finset.le_sup (f := fun original => (Composition.unitFlags r).sup
        (fun support => values (tableIndex original support (Composition.attrFlag r)))) ho)

def regimeBound (mode : Mode) (card : Card) (r : Regime) : Nat :=
  match card.multiValues with
  | none => Composition.powerBound (legacy card) r.1
  | some values =>
      if enabled mode r.2 then multiBound values r.1
      else Composition.powerBound (legacy card) r.1

theorem false_unit_flag_mem (r : Composition.Regime) :
    false ∈ Composition.unitFlags r := by
  unfold Composition.unitFlags
  split <;> simp

theorem support_flag_mem (data : Id → Card) (deck : List Id) (r : Regime)
    (h : Matches data deck r) (card : Card) :
    supportAll card (commonUnits (fun id => legacy (data id)) deck) ∈
      Composition.unitFlags r.1 := by
  unfold supportAll
  split
  next => exact false_unit_flag_mem _
  next =>
    cases card.facts.support with
    | none => exact false_unit_flag_mem _
    | some unit => exact Composition.actual_unit_flag_mem _ _ _ h.1 unit

theorem original_flag_mem (data : Id → Card) (deck : List Id) (r : Regime)
    (h : Matches data deck r) (card : Card) :
    originalAll card (commonUnits (fun id => legacy (data id)) deck) ∈
      Composition.unitFlags r.1 := by
  unfold originalAll
  split
  next => exact false_unit_flag_mem _
  next => exact Composition.actual_unit_flag_mem _ _ _ h.1 card.facts.original

noncomputable def cardPower (mode : Mode) (data : Id → Card)
    (deck : List Id) (id : Id) : Nat :=
  effective mode (isMultiUnit (deckFacts data deck))
    (if deck.length = 5 then commonUnits (fun id => legacy (data id)) deck else ∅)
    (if deck.length = 5 then sharesAttribute (fun id => legacy (data id)) deck else false) (data id)

/-- Partial decks do not enable either ALL_MATCH flag. -/
theorem partial_deck_flags (mode : Mode) (data : Id → Card) (deck : List Id)
    (id : Id) (partialDeck : deck.length ≠ 5) :
    cardPower mode data deck id =
      effective mode (isMultiUnit (deckFacts data deck)) ∅ false (data id) := by
  simp [cardPower, partialDeck]

/-- Scene bounds include the actual sidecar index, including simultaneous
all-match and mixed-unit activation. No monotonicity of the table is required. -/
theorem regime_bound_sound (mode : Mode) (data : Id → Card) (deck : List Id)
    (r : Regime) (h : Matches data deck r) (full : deck.length = 5) (id : Id) :
    cardPower mode data deck id ≤ regimeBound mode (data id) r := by
  cases hv : (data id).multiValues with
  | none =>
      simpa only [cardPower, effective, regimeBound, hv, full, ↓reduceIte] using
        Composition.power_bound_sound (fun id => legacy (data id)) deck r.1 h.1 id
  | some values =>
      simp only [cardPower, effective, regimeBound, hv, h.2, full, ↓reduceIte]
      by_cases he : enabled mode r.2 = true
      · simp only [if_pos he]
        unfold multiBound
        rw [Composition.actual_attribute_flag _ _ _ h.1]
        exact (Finset.le_sup (f := fun support =>
          values (tableIndex (originalAll (data id)
            (commonUnits (fun id => legacy (data id)) deck)) support
            (Composition.attrFlag r.1))) (support_flag_mem data deck r h (data id))).trans
          (Finset.le_sup (f := fun original => (Composition.unitFlags r.1).sup
            (fun support => values (tableIndex original support (Composition.attrFlag r.1))))
            (original_flag_mem data deck r h (data id)))
      · simp only [if_neg he]
        exact Composition.power_bound_sound _ _ _ h.1 id

/-- The source's per-plan flag means table activation, not physical mixed
composition. In particular ForceOn maps both physical classes to true. -/
def planActivation (mode : Mode) (owned mixed : Bool) : Option Bool :=
  if owned then some (enabled mode mixed) else none

def allowedActivations (mode : Mode) (owned : Bool) : Finset (Option Bool) :=
  if owned then
    (([false, true].filter (fun active =>
      !(active && decide (mode = .forceOff)) && !(!active && decide (mode = .forceOn)))).map some).toFinset
  else {none}

theorem activation_member (mode : Mode) (owned mixed : Bool) :
    planActivation mode owned mixed ∈ allowedActivations mode owned := by
  cases mode <;> cases owned <;> cases mixed <;> decide

def policyBound (mode : Mode) (card : Card) (r : Composition.Regime) : Nat :=
  let ordinary := Composition.powerBound (legacy card) r
  match card.multiValues with
  | none => ordinary
  | some values =>
      match mode with
      | .forceOff => ordinary
      | .forceOn => multiBound values r
      | .byDeck => max ordinary (multiBound values r)

def planBound (mode : Mode) (card : Card) (r : Composition.Regime) : Option Bool → Nat
  | none => policyBound mode card r
  | some false => Composition.powerBound (legacy card) r
  | some true => (card.multiValues.map (fun values => multiBound values r)).getD 0

theorem plan_bound_eq (mode : Mode) (card : Card) (scene : Regime) :
    planBound mode card scene.1 (planActivation mode card.multiValues.isSome scene.2) =
      regimeBound mode card scene := by
  cases present : card.multiValues with
  | none => cases mode <;> simp [planBound, planActivation, policyBound, regimeBound, present]
  | some values =>
      cases mode <;> cases mixed : scene.2 <;>
        simp [planBound, planActivation, regimeBound, present, mixed, enabled]

abbrev SourcePlan := Composition.Regime × Option Bool

def sourcePlans (mode : Mode) (owned : Bool) : Finset SourcePlan :=
  Finset.univ ×ˢ allowedActivations mode owned

/-- Every full deck is covered by a plan that the source actually enumerates.
Uniform sidecar presence is a pool-layout fact, not an objective upper-bound
premise; SidecarLayout instantiates it for the concrete optional pool column. -/
theorem source_plans_cover (mode : Mode) (owned : Bool) (data : Id → Card) (deck : List Id)
    (presence : ∀ id, (data id).multiValues.isSome = owned) (full : deck.length = 5) :
    ∃ plan ∈ sourcePlans mode owned, ∀ id,
      cardPower mode data deck id ≤ planBound mode (data id) plan.1 plan.2 := by
  obtain ⟨scene, matching⟩ := regimes_cover data deck
  refine ⟨(scene.1, planActivation mode owned scene.2), ?_, ?_⟩
  · exact Finset.mem_product.mpr ⟨Finset.mem_univ _, activation_member mode owned scene.2⟩
  · intro id
    have same := plan_bound_eq mode (data id) scene
    rw [presence id] at same
    rw [same]
    exact regime_bound_sound mode data deck scene matching full id

noncomputable def total (mode : Mode) (data : Id → Card) (deck : List Id) : Nat :=
  (deck.map (cardPower mode data deck)).sum

/-- The same sum bound feeds every objective using total power. -/
theorem regime_total_bound (mode : Mode) (data : Id → Card) (deck : List Id)
    (r : Regime) (h : Matches data deck r) (full : deck.length = 5) :
    total mode data deck ≤ (deck.map (fun id => regimeBound mode (data id) r)).sum :=
  List.sum_le_sum (fun id _ => regime_bound_sound mode data deck r h full id)

theorem total_upper (mode : Mode) (data : Id → Card) (deck : List Id) :
    total mode data deck ≤ (deck.map (fun id => maximum (data id))).sum :=
  List.sum_le_sum (fun id _ => effective_upper mode _ _ _ (data id))

theorem total_lower (mode : Mode) (data : Id → Card) (deck : List Id) :
    (deck.map (fun id => minimum (data id))).sum ≤ total mode data deck :=
  List.sum_le_sum (fun id _ => effective_lower mode _ _ _ (data id))

theorem maximizing_prune (mode : Mode) (data : Id → Card) (deck : List Id)
    (honor threshold : Nat) (cap : Option Nat)
    (cut : clamp cap ((deck.map (fun id => maximum (data id))).sum + honor) < threshold) :
    clamp cap (total mode data deck + honor) < threshold :=
  lt_of_le_of_lt (clamp_monotone cap (Nat.add_le_add_right (total_upper mode data deck) honor)) cut

theorem minimizing_prune (mode : Mode) (data : Id → Card) (deck : List Id)
    (honor threshold : Nat) (cap : Option Nat)
    (cut : threshold < clamp cap ((deck.map (fun id => minimum (data id))).sum + honor)) :
    threshold < clamp cap (total mode data deck + honor) :=
  lt_of_lt_of_le cut (clamp_monotone cap (Nat.add_le_add_right (total_lower mode data deck) honor))

/-- The suffix retains the non-all-match value even if all-match is possible. -/
def legacySuffixBound (card : CardData) (allowed : Finset UnitId) (attr : Bool) : Nat :=
  card.units.sup (fun unit => max (unitValue card ∅ attr unit)
    (if unit ∈ allowed then unitValue card {unit} attr unit else 0))

theorem legacy_suffix_bound (card : CardData) (common allowed : Finset UnitId) (attr : Bool)
    (inside : common ⊆ allowed) : resolved card common attr ≤ legacySuffixBound card allowed attr := by
  apply Finset.sup_le
  intro unit member
  apply le_trans _ (Finset.le_sup (f := fun unit => max (unitValue card ∅ attr unit)
    (if unit ∈ allowed then unitValue card {unit} attr unit else 0)) member)
  dsimp only
  by_cases shared : unit ∈ common
  · have same : unitValue card common attr unit = unitValue card {unit} attr unit := by
      simp [unitValue, shared]
    rw [same, if_pos (inside shared)]
    exact le_max_right _ _
  · have same : unitValue card common attr unit = unitValue card ∅ attr unit := by
      simp [unitValue, shared]
    rw [same]
    exact le_max_left _ _

def suffixStateAllowed (original support attr : Bool) (state : Fin 8) : Bool :=
  decide ((state.val &&& 1 ≠ 0 ↔ attr = true) ∧
    (state.val &&& 4 ≠ 0 → original = true) ∧
    (state.val &&& 2 ≠ 0 → support = true))

theorem suffix_encoded_state : ∀ (original support attr availableOriginal availableSupport : Bool),
    suffixStateAllowed availableOriginal availableSupport attr (tableIndex original support attr) = true ↔
      (original = true → availableOriginal = true) ∧ (support = true → availableSupport = true) := by
  decide

def multiSuffixBound (card : Card) (values : Fin 8 → Nat) (allowed : Finset UnitId) (attr : Bool) : Nat :=
  (Finset.univ.filter (fun state =>
    suffixStateAllowed (originalAll card allowed) (supportAll card allowed) attr state = true)).sup values

theorem original_flag_mono (card : Card) (common allowed : Finset UnitId) (inside : common ⊆ allowed) :
    originalAll card common = true → originalAll card allowed = true := by
  unfold originalAll
  split
  · simp
  · simpa only [decide_eq_true_eq] using (fun member => inside member :
      card.facts.original ∈ common → card.facts.original ∈ allowed)

theorem support_flag_mono (card : Card) (common allowed : Finset UnitId) (inside : common ⊆ allowed) :
    supportAll card common = true → supportAll card allowed = true := by
  unfold supportAll
  split
  · simp
  · cases card.facts.support with
    | none => simp
    | some unit => simpa only [decide_eq_true_eq] using
        (fun member => inside member : unit ∈ common → unit ∈ allowed)

theorem multi_suffix_bound (card : Card) (values : Fin 8 → Nat)
    (common allowed : Finset UnitId) (attr : Bool) (inside : common ⊆ allowed) :
    values (tableIndex (originalAll card common) (supportAll card common) attr) ≤
      multiSuffixBound card values allowed attr := by
  apply Finset.le_sup (f := values)
  refine Finset.mem_filter.mpr ⟨Finset.mem_univ _, ?_⟩
  exact (suffix_encoded_state _ _ _ _ _).mpr
    ⟨original_flag_mono card common allowed inside, support_flag_mono card common allowed inside⟩

/-- Both branches match the source guards: no owned table always scans legacy,
ForceOn excludes legacy only when the optional table exists, and ForceOff never
reads that table. -/
def suffixBound (mode : Mode) (card : Card) (allowed : Finset UnitId) (attr : Bool) : Nat :=
  let ordinary := legacySuffixBound (legacy card) allowed attr
  match card.multiValues with
  | none => ordinary
  | some values =>
      match mode with
      | .forceOff => ordinary
      | .forceOn => multiSuffixBound card values allowed attr
      | .byDeck => max ordinary (multiSuffixBound card values allowed attr)

theorem suffix_bound_sound (mode : Mode) (card : Card) (mixed attr : Bool)
    (common allowed : Finset UnitId) (inside : common ⊆ allowed) :
    effective mode mixed common attr card ≤ suffixBound mode card allowed attr := by
  have ordinary := legacy_suffix_bound (legacy card) common allowed attr inside
  cases present : card.multiValues with
  | none => simpa only [effective, suffixBound, present] using ordinary
  | some values =>
      have special := multi_suffix_bound card values common allowed attr inside
      cases mode with
      | forceOff => simpa only [effective, suffixBound, present, enabled, Bool.false_eq_true,
          ↓reduceIte] using ordinary
      | forceOn => simpa only [effective, suffixBound, present, enabled, ↓reduceIte] using special
      | byDeck =>
          cases mixed
          · simpa only [effective, suffixBound, present, enabled, Bool.false_eq_true, ↓reduceIte] using
              ordinary.trans (le_max_left _ (multiSuffixBound card values allowed attr))
          · simpa only [effective, suffixBound, present, enabled, ↓reduceIte] using
              special.trans (le_max_right (legacySuffixBound (legacy card) allowed attr) _)

end Allium.MixedPower
