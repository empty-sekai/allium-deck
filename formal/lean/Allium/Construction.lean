import Allium.Admission

/-!
# Internal checked construction

The sole success premise is the result of the concrete intermediate-input
build function. Reference metadata, encoded column reads, sorted dense
addresses and numeric checks are consequences of that result. When no skill
payload is present, a raw slot is merely preserved: its semantic validity
requires a separate producer-to-construction proof.
Floating arithmetic is the Binary64 numeric semantics; search-ceiling error
budgets and compiler refinement are separate obligations.
-/
namespace Allium.Admission.Construction
open GatherOrder

/-- The complete representational postcondition of a successful construction. -/
structure Success (cards : List Card) (context : SourceNumeric.Context) (pool : GatherPipeline.Pool) : Prop where
  checked : CardValidation.validate (cards.map Card.row) = .ok ()
  ordered : pool.cards = GatherOrder.sort context.target context.hasEvent
    (context.fixedCards.map Fin.val) (context.fixedCharacters.map Fin.val) cards
  permutation : pool.cards.Perm cards
  sorted : pool.cards.Sorted (fun left right =>
    GatherOrder.key context.target context.hasEvent (context.fixedCards.map Fin.val)
      (context.fixedCharacters.map Fin.val) left ≤
    GatherOrder.key context.target context.hasEvent (context.fixedCards.map Fin.val)
      (context.fixedCharacters.map Fin.val) right)
  valid : GatherPipeline.Valid pool
  count : pool.cards.length ≤ 65535
  dense : ∀ index : Fin pool.cards.length, index.val % 65536 = index.val
  identities : ∀ card ∈ pool.cards,
    card.row.publicId.toNat % 65536 = card.row.publicId.toNat ∧
    card.row.character % 256 = card.row.character ∧ card.row.attr % 256 = card.row.attr ∧
    card.row.unitMask % 256 = card.row.unitMask
  metadata : ∀ card ∈ pool.cards, card.row.character < 27 ∧ card.row.attr < 5
  powerColumn : ∀ index : Fin pool.cards.length,
    DenseWrites.fill (GatherImage.powerValues pool) 0 (fun _ => none) index.val =
      some (pool.cards[index]).powerMax.value.toNat
  skillColumn : ∀ index : Fin pool.cards.length,
    DenseWrites.fill (GatherImage.skillValues pool) 0 (fun _ => none) index.val =
      some (pool.cards[index]).row.skillMax.val
  bonusColumns : GatherImage.bonusValues pool =
    pool.cards.map (fun card => (card.row.baseBonus, card.row.limitedBonus))
  powerRows : ∀ (index : Fin pool.cards.length) (unit : Fin 6) (member : Fin 4),
    (pool.cards[index]).row.unitMask.testBit unit.val = true →
    let card := (pool.cards[index]).row
    let admitted := GatherPipeline.row_domain pool valid index
    PowerEncoding.decode (PowerEncoding.rowValues card admitted)
      (PowerEncoding.profileWord (PowerEncoding.rowMask card admitted))
      (PowerEncoding.slot (PowerEncoding.profile (PowerEncoding.rowMask card admitted) unit) member) =
      (card.power unit member).toNat
  multiAbsent : GatherImage.multiColumns pool = none ↔ ∀ card ∈ pool.cards, card.multiPower = none
  multiColumns : ∀ rows, GatherImage.multiColumns pool = some rows →
    rows.length = pool.cards.length ∧ ∀ index : Fin pool.cards.length,
      rows[index.val]? = some (GatherImage.multiRow pool.cards[index])
  multiValues : ∀ card ∈ pool.cards, ∀ values : Fin 8 → Signed32, card.multiPower = some values →
    ∀ index, GatherImage.multiRow card index = (values index).value.toNat
  referenceUpper : ∀ card ∈ pool.cards, ∀ reference : SkillGather.PairPayload,
    card.skills.reference = some reference → card.row.skillMin.val + reference.2.val ≤ card.row.skillMax.val
  numeric : SourceNumeric.Accepted pool context
  powerSumFits : SourceNumeric.rawPower pool context < 2 ^ 64
  bonusSumFits : SourceNumeric.greatestFive (SourceNumeric.cardBonuses pool) < 2 ^ 64

theorem success (cards : List Card) (context : SourceNumeric.Context) (pool : GatherPipeline.Pool)
    (accepted : SourceNumeric.build cards context = .ok pool) : Success cards context pool := by
  have constructed := SourceNumeric.concrete_success cards context pool accepted
  have gathered := GatherPipeline.gather_success context.target context.hasEvent
    (context.fixedCards.map Fin.val) (context.fixedCharacters.map Fin.val) cards pool constructed.1
  have valid := constructed.2.1
  have narrowed := CardTable.accepted_narrowing (pool.cards.map Card.row) valid.1
  have domain : ∀ card ∈ pool.cards, CardTable.Domain card.row := by
    intro card member
    exact CardTable.accepted_row _ valid.1 _ (List.mem_map.mpr ⟨card, member, rfl⟩)
  refine {
    checked := gathered.1
    ordered := gathered.2.1
    permutation := ?_
    sorted := ?_
    valid := valid
    count := ?_
    dense := ?_
    identities := ?_
    metadata := fun card member => CardTable.metadata_addresses card.row (domain card member)
    powerColumn := GatherImage.power_column pool
    skillColumn := GatherImage.skill_column pool
    bonusColumns := GatherImage.bonuses_exact pool valid
    powerRows := fun index unit member carried => PowerEncoding.row_decode _ _ unit member carried
    multiAbsent := GatherImage.multi_absent pool
    multiColumns := fun rows present => ⟨GatherImage.multi_count pool rows present,
      GatherImage.multi_column pool rows present⟩
    multiValues := fun card _ values present => GatherImage.multi_row_exact card values present
    referenceUpper := ?_
    numeric := (SourceNumeric.numeric_iff pool context).mp constructed.2.2.1
    powerSumFits := SourceNumeric.raw_power_u64 pool context
    bonusSumFits := SourceNumeric.bonus_sum_u64 pool valid }
  · rw [gathered.2.1]
    exact GatherOrder.sort_permutation _ _ _ _ _
  · rw [gathered.2.1]
    exact GatherOrder.sorted _ _ _ _ _
  · simpa only [List.length_map] using ((CardTable.validate_iff _).mp valid.1).1
  · intro index
    exact narrowed.2.1 ⟨index.val, by simpa only [List.length_map] using index.isLt⟩
  · intro card member
    exact narrowed.2.2 card.row (List.mem_map.mpr ⟨card, member, rfl⟩)
  · intro card member reference present
    have bound := (domain card member).2.2.2.2.2.2.2.2
    simpa only [GatherOrder.reference_field, present, Option.map_some] using bound

end Allium.Admission.Construction
