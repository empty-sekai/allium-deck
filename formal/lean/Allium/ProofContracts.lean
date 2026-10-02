import Allium.Construction
import Allium.PreparedSkills
import Allium.Arithmetic
import Allium.Canonical
import Allium.Enumeration
import Allium.Budget
import Allium.MixedPower
import Allium.Gate
import Allium.SimdMask
import Allium.CompactDelta
import Allium.SidecarLayout
import Allium.WorldBloom
import Allium.Refill
import Allium.SourceRows
import Allium.NumericSlots
import Allium.DominanceLoop
import Allium.ScenarioPower

/-!
# Typed proof obligations

These proposition definitions are the interface consumed by the verification
script. A certificate must be a theorem whose type is definitionally equal to
the declared proposition. Component certificates do not certify completion of
an obligation with additional requirements in the coverage manifest.
-/
namespace Allium.ProofContracts

/-- The same sorted unused-character scan is updated by compact exclusion;
all selected-row positions fit the five-entry storage. -/
def P09 : Prop :=
  (∀ (weight : Nat → Nat) (used : Finset Nat) (count card : Nat),
    SuffixDelta.prefixSum weight (CompactDelta.productionRows weight (insert card used)) count =
      SuffixDelta.prefixSum weight (CompactDelta.productionRows weight used) count -
        CompactDelta.productionDelta weight used count card) ∧
  (∀ (weight : Nat → Nat) (used : Finset Nat) (count card : Nat), count ≤ 5 →
    card ∈ (CompactDelta.productionRows weight used).take count →
    let selected := ((CompactDelta.productionRows weight used).take count).toFinset
    CompactDelta.population (CompactDelta.selectedMask selected &&& CompactDelta.lowerMask card) =
      CompactDelta.position selected card ∧ CompactDelta.position selected card < 5) ∧
  (∀ (weight : Nat → Nat) (used : Finset Nat) (count card selected candidate threshold : Nat),
    selected + candidate + SuffixDelta.prefixSum weight
      (CompactDelta.productionRows weight (insert card used)) count < threshold ↔
    selected + candidate + (SuffixDelta.prefixSum weight (CompactDelta.productionRows weight used) count -
      CompactDelta.productionDelta weight used count card) < threshold)

/-- Integer no-event numerator comparison. -/
def P13 : Prop :=
  ∀ n d t : Nat, 0 < d → (n / d < t ↔ n < t * d)

/-- Scalar and vector lane semantics, including tails and equality. -/
def P19 : Prop :=
  (∀ (upper : Nat → BitVec 64) (threshold : BitVec 64),
    SimdMask.vectorMask upper threshold = SimdMask.scalarMask upper threshold) ∧
  (∀ (length : Nat) (character : Nat → Fin 27) (used : Finset (Fin 27))
      (upper : Nat → BitVec 64) (threshold : BitVec 64) (lane : Nat),
    (SimdMask.surviving length character used upper threshold).getLsbD lane = true ↔
      lane < 16 ∧ lane < length ∧ character lane ∉ used ∧
        threshold.toNat ≤ (upper lane).toNat) ∧
  (∀ (upper : Nat → BitVec 64) (old fresh : BitVec 64), old.toNat ≤ fresh.toNat →
    SimdMask.scalarMask upper old &&& SimdMask.scalarMask upper fresh =
      SimdMask.scalarMask upper fresh)

/-- Merging per-character challenge results retains canonical Top-K. -/
def P40 : Prop :=
  ∀ (k : Nat) (groups : Finset Nat) (pool : Nat → Finset Canonical.Key),
    topK Canonical.identity k (groups.biUnion (fun g => topK Canonical.identity k (pool g))) =
      topK Canonical.identity k (groups.biUnion pool)

/-- Canonical collection returns exactly the requested number of public sets. -/
def S01 : Prop :=
  (∀ (k : Nat) (keys : List Canonical.Key),
    collect Canonical.identity k keys = topK Canonical.identity k keys.toFinset) ∧
  (∀ (k : Nat) (keys : Finset Canonical.Key),
    (topK Canonical.identity k keys).card = min k (keys.image Canonical.identity).card) ∧
  (∀ (s p s' p' : Nat) (ids order dense ids' order' dense' : List Nat),
    Canonical.make s p ids order dense < Canonical.make s' p' ids' order' dense' ↔
      s' < s ∨ (s = s' ∧ (p' < p ∨ (p = p' ∧
        (ids < ids' ∨ (ids = ids' ∧ (order < order' ∨ (order = order' ∧ dense < dense'))))))))

/-- General kernel contract; solver construction is a separate obligation. -/
def S02 : Prop :=
  ∀ (k : Nat) (tree : SearchTree Canonical.Key), tree.Sound Canonical.score →
    ∀ (seeds : Finset Canonical.Key), seeds ⊆ tree.leaves →
    topK Canonical.identity k (search Canonical.identity Canonical.score k seeds tree) =
      topK Canonical.identity k tree.leaves

/-- A timed-out result remains legal; a completed sound search is exact. -/
def S03 : Prop :=
  (∀ (k : Nat) (expired : SearchTree Canonical.Key → Bool) (tree : SearchTree Canonical.Key)
      (seeds : Finset Canonical.Key), seeds ⊆ tree.leaves →
    (searchInterruptible Canonical.identity Canonical.score k expired seeds tree).retained ⊆ tree.leaves) ∧
  (∀ (k : Nat) (expired : SearchTree Canonical.Key → Bool) (tree : SearchTree Canonical.Key),
    tree.Sound Canonical.score → ∀ (seeds : Finset Canonical.Key), seeds ⊆ tree.leaves →
    (searchInterruptible Canonical.identity Canonical.score k expired seeds tree).deadlineHit = false →
    topK Canonical.identity k
      (searchInterruptible Canonical.identity Canonical.score k expired seeds tree).retained =
        topK Canonical.identity k tree.leaves)

/-- Dense cultivation IDs are enumerated before public-set deduplication. -/
def S04 : Prop :=
  (∀ (pool : Finset Nat) (publicId character : Nat → Nat) (roles : Enumeration.Roles)
      (uniqueCharacters : Bool) (deck : List Nat),
    deck ∈ Enumeration.allValidDecks pool publicId character roles uniqueCharacters ↔
      Enumeration.ValidDeck pool publicId character roles uniqueCharacters deck) ∧
  (∀ (pool : Finset Nat) (publicId denseId character : Nat → Nat) (roles : Enumeration.Roles)
      (uniqueCharacters : Bool) (score power : List Nat → Nat) (key : Canonical.Key),
    key ∈ Enumeration.exhaustiveKeys pool publicId denseId character roles uniqueCharacters score power ↔
      ∃ deck, Enumeration.ValidDeck pool publicId character roles uniqueCharacters deck ∧
        Canonical.ofDeck publicId denseId score power deck = key)

/-- Component of composition coverage: legacy/mixed scenes and their numeric bounds. -/
def MixedScenes : Prop :=
  (∀ (data : Nat → MixedPower.Card) (deck : List Nat),
    ∃ r : MixedPower.Regime, MixedPower.Matches data deck r) ∧
  (∀ (mode : MixedPower.Mode) (data : Nat → MixedPower.Card) (deck : List Nat)
      (r : MixedPower.Regime), MixedPower.Matches data deck r → deck.length = 5 → ∀ id,
    MixedPower.cardPower mode data deck id ≤ MixedPower.regimeBound mode (data id) r) ∧
  (∀ (values : Fin 8 → Nat) (regime : Composition.Regime),
    MixedPower.multiBound values regime = MixedPower.filteredMultiBound values regime) ∧
  (∀ (pool : SidecarLayout.Pool Nat) (deck : List Nat), deck.length = 5 →
    ∃ plan ∈ MixedPower.sourcePlans pool.mode pool.mixed.isSome, ∀ id,
      MixedPower.cardPower pool.mode pool.card deck id ≤ MixedPower.planBound pool.mode (pool.card id) plan.1 plan.2) ∧
  (∀ (mode : MixedPower.Mode) (card : MixedPower.Card) (mixed attr : Bool)
      (common allowed : Finset PowerModel.UnitId), common ⊆ allowed →
    MixedPower.effective mode mixed common attr card ≤ MixedPower.suffixBound mode card allowed attr) ∧
  (∀ (mode : MixedPower.Mode) (data : Nat → MixedPower.Card) (deck : List Nat), deck.length = 5 → ∀ id,
    MixedPower.cardPower mode data deck id ≤ ScenarioPower.sourceBound mode (data id)
      (PowerModel.commonUnits (fun card => MixedPower.legacy (data card)) deck)
      (PowerModel.sharesAttribute (fun card => MixedPower.legacy (data card)) deck))

/-- Component of numeric power: optional-sidecar extrema, not fixed-slot DFS. -/
def MixedExtrema : Prop :=
  ∀ (mode : MixedPower.Mode) (mixed : Bool) (common : Finset PowerModel.UnitId)
      (attr : Bool) (card : MixedPower.Card),
    MixedPower.minimum card ≤ MixedPower.effective mode mixed common attr card ∧
      MixedPower.effective mode mixed common attr card ≤ MixedPower.maximum card

/-- Gate selection is stable by user level, independent of table availability. -/
def GateSelection : Prop :=
  (∀ (rows : List Gate.UserGate) (best : Gate.UserGate), Gate.highest rows = some best →
    best ∈ rows ∧ ∀ row ∈ rows, row.level ≤ best.level) ∧
  (∀ (table : Gate.RateTable) (attr : PowerModel.Attribute) (rows : List Gate.UserGate)
      (row : Gate.UserGate), Gate.highest rows = some row → table row.unit row.level = none →
    Gate.rate table ⟨DeckComposition.piapro, none, attr⟩ rows = 0)

/-- The concrete card-table validator and one-based interner preserve every
accepted identity and stored reference, including through later insertions. -/
def CardCapacity : Prop :=
  (∀ cards : List Admission.CardTable.Input,
    Admission.CardTable.validate cards = true ↔ cards.length ≤ 65535 ∧
      (∀ card ∈ cards, Admission.CardTable.Domain card) ∧
      (Admission.CardTable.limitedSet cards).card ≤ 15) ∧
  (∀ (Value : Type) [DecidableEq Value] (inputs output : List Value),
    Admission.Intern.run [] inputs = .ok output → output.length ≤ 255 ∧ output.Nodup) ∧
  (∀ (Value : Type) [DecidableEq Value] (values : List Value) (value : Value)
      (result : Admission.Intern.Result Value), values.length ≤ 255 → values.Nodup →
    Admission.Intern.intern values value = .ok result →
    0 < result.index ∧ result.index ≤ 255 ∧ result.values[result.index - 1]? = some value ∧
      result.values.length ≤ 255 ∧ result.values.Nodup) ∧
  (∀ (Value : Type) [DecidableEq Value] (values inputs output : List Value),
    Admission.Intern.run values inputs = .ok output → ∀ index : Nat, index < values.length →
      output[index]? = values[index]?) ∧
  (∀ (cards : List Admission.CardTable.Input), Admission.CardTable.validate cards = true →
    ∀ index : Fin cards.length, ∃ encoded : Fin 65536, encoded.val = index.val) ∧
  (∀ (before after : List Admission.CardTable.Input) (card : Admission.CardTable.Input),
    Admission.CardTable.validate (before ++ card :: after) = true →
    let index := Admission.LimitedCode.code (Admission.CardTable.collected [] before) card.limitedBonus
    let total := card.baseBonus + card.limitedBonus
    Admission.LimitedCode.pack total index % 65536 = Admission.LimitedCode.pack total index ∧
      index % 256 = index ∧ Admission.LimitedCode.pack total index >>> 4 = total ∧
      Admission.LimitedCode.decode (Admission.CardTable.collected [] (before ++ card :: after))
        (Admission.LimitedCode.pack total index &&& 15) = card.limitedBonus) ∧
  (∀ (card : Admission.CardTable.Input) (valid : Admission.CardTable.Domain card)
      (unit : Fin 6) (member : Fin 4), card.unitMask.testBit unit.val = true →
    Admission.PowerEncoding.decode (Admission.PowerEncoding.rowValues card valid)
      (Admission.PowerEncoding.profileWord (Admission.PowerEncoding.rowMask card valid))
      (Admission.PowerEncoding.slot (Admission.PowerEncoding.profile (Admission.PowerEncoding.rowMask card valid) unit) member) =
        (card.power unit member).toNat) ∧
  (∀ (left right : Admission.SkillGather.Payload),
    Admission.SkillGather.sourceEqual left right = true ↔ left = right) ∧
  (∀ (inputs : List Admission.SkillGather.ConcreteInput)
      (output : Admission.SkillGather.State Admission.SkillGather.Payload × List Admission.SkillGather.Slot),
    Admission.SkillGather.run Admission.SkillGather.initial (inputs.map Admission.SkillGather.encodeInput) = .ok output →
    Admission.SkillGather.Healthy output.1 ∧
      List.Forall₂ (fun input slot => Admission.SkillGather.Reference output.1 (Admission.SkillGather.encodeInput input) slot)
        inputs output.2 ∧ (∀ slot ∈ output.2, Admission.SkillGather.Fits slot) ∧ output.2.length = inputs.length) ∧
  (∀ (target : Admission.GatherOrder.Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
      (cards : List Admission.GatherOrder.Card),
    Admission.CardTable.validate (cards.map Admission.GatherOrder.Card.row) = true →
    Admission.CardTable.validate ((Admission.GatherOrder.sort target hasEvent fixedCards fixedCharacters cards).map
      Admission.GatherOrder.Card.row) = true) ∧
  (∀ (target : Admission.GatherOrder.Target) (hasEvent : Bool) (fixedCards fixedCharacters : List Nat)
      (cards : List Admission.GatherOrder.Card) (pool : Admission.GatherPipeline.Pool),
    Admission.GatherPipeline.gather target hasEvent fixedCards fixedCharacters cards = .ok pool →
    Admission.CardValidation.validate (cards.map Admission.GatherOrder.Card.row) = .ok () ∧
      pool.cards = Admission.GatherOrder.sort target hasEvent fixedCards fixedCharacters cards ∧
      Admission.GatherPipeline.Valid pool) ∧
  (∀ (target : Admission.GatherOrder.Target) (hasEvent : Bool) (live : Admission.NumericFlow.Live)
      (checks : Admission.NumericFlow.Checks),
    Admission.NumericFlow.run target hasEvent live checks =
      Admission.NumericFlow.execute (Admission.NumericFlow.initialChecks checks ++
        (Admission.NumericFlow.required target hasEvent live).map checks.ceiling))

/-- Internal construction preserves raw slots when no payload is present.
This component does not establish producer reachability or the semantic
validity of every preserved tagged slot, and is not the complete P03 contract. -/
def ConstructionCapacity : Prop :=
  CardCapacity ∧
  (∀ (cards : List Admission.GatherOrder.Card) (context : Admission.SourceNumeric.Context)
      (pool : Admission.GatherPipeline.Pool),
    Admission.SourceNumeric.build cards context = .ok pool →
      Admission.Construction.Success cards context pool) ∧
  (∀ (cards : List Admission.GatherOrder.Card) (context : Admission.SourceNumeric.Context)
      (pool : Admission.GatherPipeline.Pool),
    Admission.SourceNumeric.build cards context = .ok pool ↔
      Admission.GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
        (context.fixedCharacters.map Fin.val) cards = .ok pool ∧ Admission.SourceNumeric.Accepted pool context) ∧
  (∀ (cards : List Admission.GatherOrder.Card) (context : Admission.SourceNumeric.Context) (error : Admission.Error),
    Admission.SourceNumeric.build cards context = .error error ↔
      Admission.GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
        (context.fixedCharacters.map Fin.val) cards = .error error ∨
      ∃ pool, Admission.GatherPipeline.gather context.target context.hasEvent (context.fixedCards.map Fin.val)
        (context.fixedCharacters.map Fin.val) cards = .ok pool ∧
        Admission.SourceNumeric.numericDomain pool context = .error error) ∧
  (∀ (field : Admission.Field) (value : Binary64.Value), Binary64.nonnegative value = false →
    Admission.SourceNumeric.nonnegativeCheck field value = .error (.invalidNumber field)) ∧
  (∀ (field : Admission.Field) (value : Binary64.Value) (maximum : Nat),
    Binary64.le value (Binary64.ofNat maximum) = false →
    Admission.SourceNumeric.maximumCheck field value maximum = .error (.capacity field
      (Binary64.toU64 (Binary64.minimum (Binary64.ceil value) (Binary64.ofNat (2 ^ 64 - 1)))) maximum)) ∧
  (∀ (state : Admission.SkillGather.State Admission.SkillGather.Payload)
      (input : Admission.SkillGather.Input Admission.SkillGather.Payload)
      (kind : Admission.SkillGather.Kind) (value : Admission.SkillGather.Payload),
    Admission.SkillGather.selected input = some (kind, value) → value ∉ state.seen kind →
    255 ≤ (state.seen kind).length → Admission.SkillGather.step state input =
      .error (.capacity (Admission.SkillGather.tableField kind) ((state.seen kind).length + 1) 255))

/-- Capacity safety at the original internal boundary, together with the
actual producer-to-gather construction that supplies semantic slot validity.
No shape premise is imposed on the internal gather's raw inputs. -/
def P03 : Prop :=
  ConstructionCapacity ∧
  (∀ (tables : Admission.SkillProducer.Tables) (seeds : List Admission.PreparedSkills.Seed)
      (context : Admission.SourceNumeric.Context) (keep unitFilter : Admission.GatherOrder.Card → Bool)
      (pool : Admission.GatherPipeline.Pool),
    Admission.PreparedSkills.build tables seeds context keep unitFilter = .ok pool →
    ∃ prepared, Admission.PreparedSkills.prepare tables seeds = .ok prepared ∧
      Admission.SourceNumeric.build (Admission.PreparedSkills.intermediates prepared keep unitFilter) context = .ok pool ∧
      Admission.Construction.Success (Admission.PreparedSkills.intermediates prepared keep unitFilter) context pool ∧
      (∀ card ∈ pool.cards, Admission.PreparedSkills.Origin tables seeds card) ∧
      (∀ card ∈ pool.cards, Admission.SkillProducer.Produced card.skills) ∧
      (∀ slot ∈ pool.slots, Admission.PreparedSkills.Usable pool.tables slot)) ∧
  (∀ (tables : Admission.SkillProducer.Tables) (seeds : List Admission.PreparedSkills.Seed)
      (context : Admission.SourceNumeric.Context) (keep unitFilter : Admission.GatherOrder.Card → Bool)
      (error : Admission.Error),
    Admission.PreparedSkills.build tables seeds context keep unitFilter = .error error ↔
      Admission.PreparedSkills.prepare tables seeds = .error error ∨
      ∃ prepared, Admission.PreparedSkills.prepare tables seeds = .ok prepared ∧
        Admission.SourceNumeric.build (Admission.PreparedSkills.intermediates prepared keep unitFilter) context = .error error)

/-- Raw six-bit membership includes zero. Its lower bound follows the actual
mode policy and empty legacy scan, not a nonempty-unit premise. -/
def RawMembershipPower : Prop :=
  (∀ (mask : Fin 64) (attr : PowerModel.Attribute) (profile : PowerModel.UnitId → Bool)
      (values : Fin 8 → Nat) (multi : Option (Fin 8 → Nat)),
    Admission.CardTable.population mask.val ≤ 2 →
    (MixedPower.legacy (MixedPower.fromRaw mask attr profile values multi)).units =
      MixedPower.rawMembership mask) ∧
  (∀ (mode : MixedPower.Mode) (mixed : Bool) (common : Finset PowerModel.UnitId)
      (attr : Bool) (card : MixedPower.Card),
    MixedPower.minimumFor mode card ≤ MixedPower.effective mode mixed common attr card) ∧
  (∀ (mode : MixedPower.Mode) (card : MixedPower.Card), card.emptyMask = true →
    MixedPower.minimumFor mode card = 0) ∧
  (∀ (card : MixedPower.Card), card.emptyMask = true →
    ∀ (common : Finset PowerModel.UnitId) (attr : Bool),
      PowerModel.resolved (MixedPower.legacy card) common attr = 0)

/-- Support inheritance includes character eligibility and strict positivity
before deduplication; finite rational rates are copied without modification. -/
def FinaleInheritance : Prop :=
  (∀ (events : List SourceRows.Event) (chapters : List SourceRows.Chapter)
      (characters : List Int) (destination : Int) (rows : List (SourceRows.Row ℚ)) (target : Int × Int),
    SourceRows.findKey target (SourceRows.inheritFinale events chapters characters destination rows) =
      (SourceRows.firstEligible (SourceRows.sourceIds events chapters)
        (SourceRows.supportCharacters characters) target rows).map (SourceRows.relabel destination)) ∧
  (∀ (events : List SourceRows.Event) (chapters : List SourceRows.Chapter)
      (characters : List Int) (destination : Int) (rows : List (SourceRows.Row ℚ)) (result : SourceRows.Row ℚ),
    result ∈ SourceRows.inheritFinale events chapters characters destination rows →
    result.character ∈ characters ∧ 1 ≤ result.character ∧ result.character ≤ 26 ∧
      0 < result.rate ∧ result.event = destination) ∧
  (∀ (sources characters : Finset Int) (destination : Int) (first next : SourceRows.Row ℚ),
    first.rate ≤ 0 → next.event ∈ sources ∧ next.character ∈ characters ∧ 0 < next.rate →
    SourceRows.inherit sources characters destination ∅ [first, next] = [SourceRows.relabel destination next])

/-- The suffix scan is the sorted array of 27 concrete character maxima,
with used characters filtered out; later dense starts cannot increase it. -/
def SuffixScans : Prop :=
  (∀ (weight : Nat → Nat) (used : Finset Nat) (count : Nat),
    SuffixDelta.prefixSum weight (CompactDelta.productionRows weight used) count =
      FiniteBounds.maxSum count (Finset.range 27 \ used) weight) ∧
  (∀ (mode : MixedPower.Mode) (data : Nat → MixedPower.Card) (deck cards : List Nat)
      (start : Nat) (used : Finset Nat) (slots : Nat) (character : Nat → Fin 27) (picks : Finset Nat),
    picks ⊆ SuffixScan.densePool cards start → picks.card ≤ slots → Set.InjOn character picks →
    (∀ card ∈ picks, (character card).val ∉ used) →
    (∑ card ∈ picks, MixedPower.cardPower mode data deck card) ≤
      SuffixScan.scanBound cards start used slots character (fun card => MixedPower.maximum (data card))) ∧
  (∀ (cards : List Nat) (used : Finset Nat) (slots : Nat) (character : Nat → Fin 27) (weight : Nat → Nat),
    Antitone (fun start => SuffixScan.scanBound cards start used slots character weight))

/-- Numeric global Power bounds retain the actual saturation and cap order;
the maximizing branch derives its representable domain from acceptance. -/
def NumericGlobalPower : Prop :=
  (∀ (mode : MixedPower.Mode) (pool : Finset Nat) (data : Nat → MixedPower.Card)
      (selected free : List Nat) (honor threshold : Nat) (cap : Option Nat),
    NumericPower.LegalPoolDeck pool (selected ++ free) → NumericPower.accepted pool data honor = true →
    PowerModel.clamp cap (NumericPower.relaxed (selected.map (fun card => MixedPower.maximum (data card)))
      (NumericPower.globalMaximum pool data) (5 - selected.length) honor) < threshold →
    PowerModel.clamp cap (MixedPower.total mode data (selected ++ free) + honor) < threshold) ∧
  (∀ (mode : MixedPower.Mode) (pool : Finset Nat) (data : Nat → MixedPower.Card)
      (selected free : List Nat) (honor threshold : Nat) (cap : Option Nat),
    NumericPower.LegalPoolDeck pool (selected ++ free) →
    threshold < PowerModel.clamp cap
      (NumericPower.relaxed (selected.map (fun card => MixedPower.minimumFor mode (data card)))
        (NumericPower.globalMinimum mode pool data) (5 - selected.length) honor) →
    threshold < PowerModel.clamp cap (MixedPower.total mode data (selected ++ free) + honor)) ∧
  (∀ (pool : Finset Nat) (publicId character : Nat → Nat) (roles : Enumeration.Roles)
      (unique : Bool) (deck : List Nat), deck.length = 5 →
    NumericSlots.Steps pool publicId character roles unique [] deck → NumericPower.LegalPoolDeck pool deck)

/-- A checked sequence constructs, rather than assumes, the parent-power
invariant and transports it through every compressed root chain. -/
def CheckedRootPower : Prop :=
  ∀ (count : Nat) (publicId character : Fin count → Nat) (cards : Fin count → MixedPower.Card)
      (fixedIds : Finset Nat) (schedule : List (Fin count × Fin count)) (deck : List (Fin count))
      (mode : MixedPower.Mode) (honor : Nat) (cap : Option Nat),
    let state := DominanceLoop.scan (DominanceLoop.initial publicId character cards fixedIds) schedule
    PowerModel.clamp cap (MixedPower.total mode id (deck.map cards) + honor) ≤
      PowerModel.clamp cap
        (MixedPower.total mode id ((deck.map (RootForest.root state.forest)).map cards) + honor)

end Allium.ProofContracts
