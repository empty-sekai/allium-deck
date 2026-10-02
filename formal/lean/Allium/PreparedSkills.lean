import Allium.SkillProducer
import Allium.Construction

/-!
# Prepared skill slots

The two-slot preparation calls buildSkill, may delete one result, then clones,
flattens and filters the prepared records before gather. The original internal
gather entry remains available for arbitrary byte-valued raw slots.
-/
namespace Allium.Admission.PreparedSkills
open GatherOrder SkillGather SkillProducer

abbrev Variant := Training × SkillProducer.Output
abbrev Options := Option Variant × Option Variant

def optionsHave (predicate : Variant → Prop) (options : Options) : Prop :=
  (∀ value, options.1 = some value → predicate value) ∧
  (∀ value, options.2 = some value → predicate value)

def optionsProduced (options : Options) : Prop := optionsHave (fun value => Produced value.2.skill) options

def statesFor (request : SkillProducer.Request) (keepAfter imageKindTrained afterTraining : Bool) : List Training :=
  if keepAfter then [if imageKindTrained then .after else .before]
  else if request.trainedSkill.isSome then [.after, .before]
  else [if imageKindTrained || afterTraining then .after else .before]

def runStates (tables : Tables) (request : SkillProducer.Request) : List Training → Except Error (List Variant)
  | [] => .ok []
  | training :: rest => match buildSkill tables request training with
      | .error error => .error error
      | .ok result => match runStates tables request rest with
          | .error error => .error error
          | .ok results => .ok ((training, result) :: results)

theorem run_states_produced (tables : Tables) (request : SkillProducer.Request)
    (states : List Training) (results : List Variant) (accepted : runStates tables request states = .ok results) :
    ∀ result ∈ results, Produced result.2.skill := by
  induction states generalizing results with
  | nil =>
      have same : results = [] := by simpa only [runStates, Except.ok.injEq] using accepted.symm
      subst results
      simp
  | cons training rest ih =>
      cases built : buildSkill tables request training with
      | error error => simp [runStates, built] at accepted
      | ok result =>
          cases later : runStates tables request rest with
          | error error => simp [runStates, built, later] at accepted
          | ok remaining =>
              have same : (training, result) :: remaining = results := by
                simpa only [runStates, built, later, Except.ok.injEq] using accepted
              subst results
              exact List.forall_mem_cons.mpr ⟨build_skill_produced tables request training result built, ih remaining later⟩

theorem run_states_origin (tables : Tables) (request : SkillProducer.Request)
    (states : List Training) (results : List Variant) (accepted : runStates tables request states = .ok results) :
    ∀ result ∈ results, result.1 ∈ states ∧ buildSkill tables request result.1 = .ok result.2 := by
  induction states generalizing results with
  | nil =>
      have same : results = [] := by simpa only [runStates, Except.ok.injEq] using accepted.symm
      subst results
      simp
  | cons training rest ih =>
      cases built : buildSkill tables request training with
      | error error => simp [runStates, built] at accepted
      | ok result =>
          cases later : runStates tables request rest with
          | error error => simp [runStates, built, later] at accepted
          | ok remaining =>
              have same : (training, result) :: remaining = results := by
                simpa only [runStates, built, later, Except.ok.injEq] using accepted
              subst results
              refine List.forall_mem_cons.mpr ⟨⟨by simp, built⟩, ?_⟩
              intro value member
              have origin := ih remaining later value member
              exact ⟨List.mem_cons_of_mem _ origin.1, origin.2⟩

def padded (results : List Variant) : Options := (results[0]?, results[1]?)

theorem padded_have (predicate : Variant → Prop) (results : List Variant)
    (valid : ∀ result ∈ results, predicate result) : optionsHave predicate (padded results) := by
  constructor <;> intro value found <;>
    exact valid value (List.mem_of_getElem? found)

theorem padded_produced (results : List Variant) (valid : ∀ result ∈ results, Produced result.2.skill) :
    optionsProduced (padded results) := by
  constructor <;> intro value found <;>
    exact valid value (List.mem_of_getElem? found)

def bfesPair (first second : SkillProducer.Output) : Bool :=
  decide (first.skillId ≠ second.skillId) &&
    (first.skill.reference.isSome || second.skill.reference.isSome || first.hasReference || second.hasReference ||
      first.skill.differentUnit.isSome || second.skill.differentUnit.isSome ||
      first.skill.unitCount.isSome || second.skill.unitCount.isSome)

def collapse (options : Options) (count : Nat) : Options × Bool :=
  if count ≠ 2 then (options, false) else
    match options with
    | (some first, some second) =>
        if bfesPair first.2 second.2 then (options, true)
        else if second.2.maximum < first.2.maximum then ((some first, none), false)
        else ((none, some second), false)
    | _ => (options, false)

theorem collapse_preserves (predicate : Variant → Prop) (options : Options) (count : Nat)
    (valid : optionsHave predicate options) : optionsHave predicate (collapse options count).1 := by
  by_cases countTwo : count = 2
  · rcases options with ⟨first, second⟩
    cases first with
    | none => simpa only [collapse, countTwo, ne_eq, not_true_eq_false, if_false] using valid
    | some first =>
        cases second with
        | none => simpa only [collapse, countTwo, ne_eq, not_true_eq_false, if_false] using valid
        | some second =>
            by_cases bfes : bfesPair first.2 second.2 = true
            · simpa only [collapse, countTwo, ne_eq, not_true_eq_false, if_false, if_pos bfes] using valid
            · by_cases better : second.2.maximum < first.2.maximum
              · simp only [collapse, countTwo, ne_eq, not_true_eq_false, if_false, if_neg bfes, if_pos better]
                exact ⟨valid.1, by simp⟩
              · simp only [collapse, countTwo, ne_eq, not_true_eq_false, if_false, if_neg bfes, if_neg better]
                exact ⟨by simp, valid.2⟩
  · simpa only [collapse, if_pos countTwo] using valid

theorem collapse_produced (options : Options) (count : Nat) (valid : optionsProduced options) :
    optionsProduced (collapse options count).1 := collapse_preserves _ options count valid

/-- The non-skill fields entering CardIntermediate. The skill fields are
constructed exclusively from a successful buildSkill result. -/
structure CardData where
  publicId : Int
  character : Nat
  attr : Nat
  unitMask : Nat
  power : Fin 6 → Fin 4 → Int
  baseBonus : Nat
  limitedBonus : Nat
  powerMax : Signed32
  powerMin : Signed32
  multiPower : Option (Fin 8 → Signed32) := none
  defaultTrained : Bool
  leaderHonor : Fin 65536 := 0
  leaderLimit : Fin 65536 := 0

structure Seed where
  data : CardData
  request : SkillProducer.Request
  keepAfterTraining : Bool
  imageKindTrained : Bool
  afterTraining : Bool

structure Prepared where
  data : CardData
  options : Options
  controlsImage : Bool

def prepareOne (tables : Tables) (seed : Seed) : Except Error Prepared :=
  let states := statesFor seed.request seed.keepAfterTraining seed.imageKindTrained seed.afterTraining
  match runStates tables seed.request states with
  | .error error => .error error
  | .ok results =>
      let selected := collapse (padded results) states.length
      .ok ⟨seed.data, selected.1, selected.2⟩

theorem prepare_one_produced (tables : Tables) (seed : Seed) (prepared : Prepared)
    (accepted : prepareOne tables seed = .ok prepared) : optionsProduced prepared.options := by
  let states := statesFor seed.request seed.keepAfterTraining seed.imageKindTrained seed.afterTraining
  cases produced : runStates tables seed.request states with
  | error error => simp [prepareOne, states, produced] at accepted
  | ok results =>
      have same : Prepared.mk seed.data (collapse (padded results) states.length).1
          (collapse (padded results) states.length).2 = prepared := by
        simpa only [prepareOne, states, produced, Except.ok.injEq] using accepted
      subst prepared
      exact collapse_produced _ _ (padded_produced results (run_states_produced tables seed.request states results produced))

theorem prepare_one_origin (tables : Tables) (seed : Seed) (prepared : Prepared)
    (accepted : prepareOne tables seed = .ok prepared) :
    optionsHave (fun value =>
      value.1 ∈ statesFor seed.request seed.keepAfterTraining seed.imageKindTrained seed.afterTraining ∧
      buildSkill tables seed.request value.1 = .ok value.2) prepared.options := by
  let states := statesFor seed.request seed.keepAfterTraining seed.imageKindTrained seed.afterTraining
  cases produced : runStates tables seed.request states with
  | error error => simp [prepareOne, states, produced] at accepted
  | ok results =>
      have same : Prepared.mk seed.data (collapse (padded results) states.length).1
          (collapse (padded results) states.length).2 = prepared := by
        simpa only [prepareOne, states, produced, Except.ok.injEq] using accepted
      subst prepared
      exact collapse_preserves _ _ _ (padded_have _ results (run_states_origin tables seed.request states results produced))

def prepare (tables : Tables) : List Seed → Except Error (List Prepared)
  | [] => .ok []
  | seed :: rest => match prepareOne tables seed with
      | .error error => .error error
      | .ok result => match prepare tables rest with
          | .error error => .error error
          | .ok results => .ok (result :: results)

theorem prepare_produced (tables : Tables) (seeds : List Seed) (prepared : List Prepared)
    (accepted : prepare tables seeds = .ok prepared) : ∀ card ∈ prepared, optionsProduced card.options := by
  induction seeds generalizing prepared with
  | nil =>
      have same : prepared = [] := by simpa only [prepare, Except.ok.injEq] using accepted.symm
      subst prepared
      simp
  | cons seed rest ih =>
      cases built : prepareOne tables seed with
      | error error => simp [prepare, built] at accepted
      | ok card =>
          cases later : prepare tables rest with
          | error error => simp [prepare, built, later] at accepted
          | ok remaining =>
              have same : card :: remaining = prepared := by
                simpa only [prepare, built, later, Except.ok.injEq] using accepted
              subst prepared
              exact List.forall_mem_cons.mpr ⟨prepare_one_produced tables seed card built, ih remaining later⟩

theorem prepare_origin (tables : Tables) (seeds : List Seed) (prepared : List Prepared)
    (accepted : prepare tables seeds = .ok prepared) :
    ∀ card ∈ prepared, ∃ seed ∈ seeds, prepareOne tables seed = .ok card := by
  induction seeds generalizing prepared with
  | nil =>
      have same : prepared = [] := by simpa only [prepare, Except.ok.injEq] using accepted.symm
      subst prepared
      simp
  | cons seed rest ih =>
      cases built : prepareOne tables seed with
      | error error => simp [prepare, built] at accepted
      | ok card =>
          cases later : prepare tables rest with
          | error error => simp [prepare, built, later] at accepted
          | ok remaining =>
              have same : card :: remaining = prepared := by
                simpa only [prepare, built, later, Except.ok.injEq] using accepted
              subst prepared
              refine List.forall_mem_cons.mpr ⟨⟨seed, by simp, built⟩, ?_⟩
              intro value member
              obtain ⟨source, sourceMember, origin⟩ := ih remaining later value member
              exact ⟨source, List.mem_cons_of_mem _ sourceMember, origin⟩

def variants (options : Options) : List Variant := options.1.toList ++ options.2.toList

theorem variants_have (predicate : Variant → Prop) (options : Options)
    (valid : optionsHave predicate options) : ∀ value ∈ variants options, predicate value := by
  intro value member
  rcases List.mem_append.mp member with first | second
  · exact valid.1 value (Option.mem_toList.mp first)
  · exact valid.2 value (Option.mem_toList.mp second)

def intermediate (card : Prepared) (variant : Variant) : Card :=
  { core := { publicId := card.data.publicId
              character := card.data.character
              attr := card.data.attr
              unitMask := card.data.unitMask
              power := card.data.power
              baseBonus := card.data.baseBonus
              limitedBonus := card.data.limitedBonus
              skillMin := variant.2.minimum
              skillMax := variant.2.maximum }
    powerMax := card.data.powerMax
    powerMin := card.data.powerMin
    multiPower := card.data.multiPower
    trained := if card.controlsImage then decide (variant.1 = .after) else card.data.defaultTrained
    skills := variant.2.skill
    leaderHonor := card.data.leaderHonor
    leaderLimit := card.data.leaderLimit }

/-- Both keep_card and the subsequent unit filter only remove intermediates.
The preservation statement is uniform in their predicates; it does not claim
to verify their separate candidate-set semantics. -/
def intermediates (prepared : List Prepared) (keep unitFilter : Card → Bool) : List Card :=
  ((prepared.flatMap (fun card => (variants card.options).map (intermediate card))).filter keep).filter unitFilter

theorem intermediates_produced (prepared : List Prepared) (keep unitFilter : Card → Bool)
    (valid : ∀ card ∈ prepared, optionsProduced card.options) :
    ∀ card ∈ intermediates prepared keep unitFilter, Produced card.skills := by
  intro card member
  have retained := (List.mem_filter.mp (List.mem_filter.mp member).1).1
  obtain ⟨source, sourceMember, variantMember⟩ := List.mem_flatMap.mp retained
  obtain ⟨variant, inVariants, rfl⟩ := List.mem_map.mp variantMember
  have correct := valid source sourceMember
  rcases List.mem_append.mp inVariants with first | second
  · exact correct.1 variant (Option.mem_toList.mp first)
  · exact correct.2 variant (Option.mem_toList.mp second)

def Origin (tables : Tables) (seeds : List Seed) (card : Card) : Prop :=
  ∃ seed ∈ seeds, ∃ (prepared : Prepared) (variant : Variant),
    prepareOne tables seed = .ok prepared ∧ variant ∈ variants prepared.options ∧
    variant.1 ∈ statesFor seed.request seed.keepAfterTraining seed.imageKindTrained seed.afterTraining ∧
    buildSkill tables seed.request variant.1 = .ok variant.2 ∧ intermediate prepared variant = card

theorem intermediates_origin (tables : Tables) (seeds : List Seed) (prepared : List Prepared)
    (accepted : prepare tables seeds = .ok prepared) (keep unitFilter : Card → Bool) :
    ∀ card ∈ intermediates prepared keep unitFilter, Origin tables seeds card := by
  intro card member
  have retained := (List.mem_filter.mp (List.mem_filter.mp member).1).1
  obtain ⟨source, sourceMember, variantMember⟩ := List.mem_flatMap.mp retained
  obtain ⟨variant, inVariants, same⟩ := List.mem_map.mp variantMember
  obtain ⟨seed, seedMember, generated⟩ := prepare_origin tables seeds prepared accepted source sourceMember
  have origin := variants_have _ source.options (prepare_one_origin tables seed source generated) variant inVariants
  exact ⟨seed, seedMember, source, variant, generated, inVariants, origin.1, origin.2, same⟩

/-- No arbitrary skill record enters this preparation-to-gather path. -/
noncomputable def build (tables : Tables) (seeds : List Seed) (context : SourceNumeric.Context)
    (keep unitFilter : Card → Bool) : Except Error GatherPipeline.Pool :=
  match prepare tables seeds with
  | .error error => .error error
  | .ok prepared => SourceNumeric.build (intermediates prepared keep unitFilter) context

def tagged (kind : Kind) : Payload → Prop
  | .unitCount _ => kind = 0
  | .differentUnit _ => kind = 1
  | .reference _ => kind = 2

/-- Ordinary values require no side table. Tagged values have a correctly
typed payload at the exact nonzero one-based address in their final table. -/
def Usable (state : SkillGather.State Payload) (slot : Slot) : Prop :=
  slot.1 = 0 ∨ ∃ (kind : Kind) (value : Payload), tagged kind value ∧
    slot.1 = kind.val + 1 ∧ 0 < slot.2 ∧ slot.2 ≤ 255 ∧
    slot.2 ≤ (state.stored kind).length ∧ (state.stored kind)[slot.2 - 1]? = some value

theorem produced_reference (state : SkillGather.State Payload) (input : ConcreteInput) (slot : Slot)
    (produced : Produced input) (reference : Reference state (encodeInput input) slot) : Usable state slot := by
  cases produced with
  | ordinary value =>
      have same : slot = (0, value.val) := by
        simpa only [Reference, selected, encodeInput, Option.map_none] using reference
      exact Or.inl (congrArg Prod.fst same)
  | unitCount value =>
      refine Or.inr ⟨0, .unitCount value, rfl, ?_⟩
      simpa only [Reference, selected, encodeInput, Option.map_some] using reference
  | differentUnit value =>
      refine Or.inr ⟨1, .differentUnit value, rfl, ?_⟩
      simpa only [Reference, selected, encodeInput, Option.map_none, Option.map_some] using reference
  | reference value =>
      refine Or.inr ⟨2, .reference value, rfl, ?_⟩
      simpa only [Reference, selected, encodeInput, Option.map_none, Option.map_some] using reference

theorem references_usable (state : SkillGather.State Payload) (cards : List Card) (slots : List Slot)
    (reference : List.Forall₂ (fun card slot => Reference state (encodeInput card.skills) slot) cards slots)
    (produced : ∀ card ∈ cards, Produced card.skills) : ∀ slot ∈ slots, Usable state slot := by
  revert produced
  induction reference with
  | nil => simp
  | @cons card slot cards slots first rest ih =>
      intro produced
      exact List.forall_mem_cons.mpr
        ⟨produced_reference state card.skills slot (produced card (by simp)) first,
          ih (fun value member => produced value (by simp [member]))⟩

/-- The public preparation projection supplies the producer invariant itself.
No Produced/shape/column-consistency assumption is accepted by this theorem. -/
theorem build_success (tables : Tables) (seeds : List Seed) (context : SourceNumeric.Context)
    (keep unitFilter : Card → Bool) (pool : GatherPipeline.Pool)
    (accepted : build tables seeds context keep unitFilter = .ok pool) :
    ∃ prepared, prepare tables seeds = .ok prepared ∧
      SourceNumeric.build (intermediates prepared keep unitFilter) context = .ok pool ∧
      Construction.Success (intermediates prepared keep unitFilter) context pool ∧
      (∀ card ∈ pool.cards, Origin tables seeds card) ∧
      (∀ card ∈ pool.cards, Produced card.skills) ∧ (∀ slot ∈ pool.slots, Usable pool.tables slot) := by
  cases preparedResult : prepare tables seeds with
  | error error => simp [build, preparedResult] at accepted
  | ok prepared =>
      have constructed : SourceNumeric.build (intermediates prepared keep unitFilter) context = .ok pool := by
        simpa only [build, preparedResult] using accepted
      have post := Construction.success _ context pool constructed
      have source := intermediates_produced prepared keep unitFilter
        (prepare_produced tables seeds prepared preparedResult)
      have produced : ∀ card ∈ pool.cards, Produced card.skills := by
        intro card member
        exact source card (post.permutation.mem_iff.mp member)
      have origins := intermediates_origin tables seeds prepared preparedResult keep unitFilter
      exact ⟨prepared, rfl, constructed, post,
        fun card member => origins card (post.permutation.mem_iff.mp member), produced,
        references_usable pool.tables pool.cards pool.slots post.valid.2.2.1 produced⟩

theorem build_failure_iff (tables : Tables) (seeds : List Seed) (context : SourceNumeric.Context)
    (keep unitFilter : Card → Bool) (error : Error) :
    build tables seeds context keep unitFilter = .error error ↔
      prepare tables seeds = .error error ∨ ∃ prepared, prepare tables seeds = .ok prepared ∧
        SourceNumeric.build (intermediates prepared keep unitFilter) context = .error error := by
  cases preparedResult : prepare tables seeds <;> simp [build, preparedResult]

end Allium.Admission.PreparedSkills
