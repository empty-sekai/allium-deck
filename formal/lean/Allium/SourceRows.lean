import Allium.WorldBloom

/-!
# Source-row inheritance for a simulated finale

Eligible source events are third-edition World Bloom events with IDs below
1000 and without a finale chapter. Source IDs are signed, matching the input
tables; the character filter, rather than a natural-number type, excludes
negative character IDs. Support rows require a character present in the character-unit
table, with ID in 1..26, and a strictly positive rate. Eligibility is checked
before inserting the (character, card) key. The first eligible row wins in
master order; only its event is changed.

Rates are ordered values with zero. The selection algorithm does not convert,
round, or perform arithmetic on an inherited rate. Rational instantiation
models finite input values; a floating-point decoding refinement is separate.
-/
namespace Allium.SourceRows

structure Event where
  id : Int
  worldBloom : Bool
  edition : Int

structure Chapter where
  event : Int
  finale : Bool

def sourceEvent (chapters : List Chapter) (event : Event) : Bool :=
  decide (event.id < 1000) && event.worldBloom && decide (event.edition = 3) &&
    !(chapters.any (fun chapter => decide (chapter.event = event.id) && chapter.finale))

def sourceIds (events : List Event) (chapters : List Chapter) : Finset Int :=
  ((events.filter (sourceEvent chapters)).map Event.id).toFinset

@[simp] theorem mem_source_ids (events : List Event) (chapters : List Chapter) (id : Int) :
    id ∈ sourceIds events chapters ↔
      ∃ event ∈ events, sourceEvent chapters event = true ∧ event.id = id := by
  simp [sourceIds, and_assoc]

/-- The character-unit table can repeat characters; only IDs 1 through 26
contribute to the support-character set. -/
def supportCharacters (characterUnits : List Int) : Finset Int :=
  characterUnits.toFinset.filter (fun character => 1 ≤ character ∧ character ≤ 26)

@[simp] theorem mem_support_characters (characterUnits : List Int) (character : Int) :
    character ∈ supportCharacters characterUnits ↔
      character ∈ characterUnits ∧ 1 ≤ character ∧ character ≤ 26 := by
  simp [supportCharacters]

structure Row (Rate : Type*) where
  event : Int
  character : Int
  card : Int
  rate : Rate

def key {Rate : Type*} (row : Row Rate) : Int × Int := (row.character, row.card)

def relabel {Rate : Type*} (destination : Int) (row : Row Rate) : Row Rate :=
  { row with event := destination }

@[simp] theorem relabel_key {Rate : Type*} (destination : Int) (row : Row Rate) :
    key (relabel destination row) = key row := rfl

@[simp] theorem relabel_rate {Rate : Type*} (destination : Int) (row : Row Rate) :
    (relabel destination row).rate = row.rate := rfl

variable {Rate : Type*} [LinearOrder Rate] [Zero Rate]

def eligible (sources characters : Finset Int) (row : Row Rate) : Bool :=
  decide (row.event ∈ sources ∧ row.character ∈ characters ∧ 0 < row.rate)

@[simp] theorem eligible_iff (sources characters : Finset Int) (row : Row Rate) :
    eligible sources characters row = true ↔
      row.event ∈ sources ∧ row.character ∈ characters ∧ 0 < row.rate := by
  simp [eligible]

def inherit (sources characters : Finset Int) (destination : Int) :
    Finset (Int × Int) → List (Row Rate) → List (Row Rate)
  | _, [] => []
  | seen, row :: rest =>
      if eligible sources characters row = true ∧ key row ∉ seen then
        relabel destination row :: inherit sources characters destination (insert (key row) seen) rest
      else inherit sources characters destination seen rest

/-- Every output value comes from an eligible source row unchanged. -/
theorem inherited_provenance (sources characters : Finset Int) (destination : Int)
    (seen : Finset (Int × Int)) (rows : List (Row Rate)) (result : Row Rate)
    (member : result ∈ inherit sources characters destination seen rows) :
    ∃ row ∈ rows, eligible sources characters row = true ∧ relabel destination row = result := by
  induction rows generalizing seen with
  | nil => simp [inherit] at member
  | cons row rest ih =>
      unfold inherit at member
      split at member
      next accepted =>
        rcases List.mem_cons.mp member with he | hm
        · exact ⟨row, List.mem_cons_self, accepted.1, he.symm⟩
        · obtain ⟨original, ho, hs, he⟩ := ih _ hm
          exact ⟨original, List.mem_cons_of_mem row ho, hs, he⟩
      next =>
        obtain ⟨original, ho, hs, he⟩ := ih _ member
        exact ⟨original, List.mem_cons_of_mem row ho, hs, he⟩

theorem inherited_positive (sources characters : Finset Int) (destination : Int)
    (seen : Finset (Int × Int)) (rows : List (Row Rate)) (result : Row Rate)
    (member : result ∈ inherit sources characters destination seen rows) : 0 < result.rate := by
  obtain ⟨row, _, accepted, he⟩ := inherited_provenance sources characters destination seen rows result member
  rw [← he, relabel_rate]
  exact ((eligible_iff sources characters row).mp accepted).2.2

/-- A zero or negative row leaves the seen-key set unchanged, allowing a
later positive row with the same key to be selected. -/
theorem nonpositive_skipped (sources characters : Finset Int) (destination : Int)
    (seen : Finset (Int × Int)) (row : Row Rate) (rest : List (Row Rate))
    (nonpositive : row.rate ≤ 0) :
    inherit sources characters destination seen (row :: rest) =
      inherit sources characters destination seen rest := by
  have rejected : eligible sources characters row ≠ true := by
    intro accepted
    exact (not_lt_of_ge nonpositive) (((eligible_iff sources characters row).mp accepted).2.2)
  rw [inherit, if_neg (fun accepted => rejected accepted.1)]

theorem unsupported_character_skipped (sources characters : Finset Int) (destination : Int)
    (seen : Finset (Int × Int)) (row : Row Rate) (rest : List (Row Rate))
    (unsupported : row.character ∉ characters) :
    inherit sources characters destination seen (row :: rest) =
      inherit sources characters destination seen rest := by
  have rejected : eligible sources characters row ≠ true := by
    intro accepted
    exact unsupported (((eligible_iff sources characters row).mp accepted).2.1)
  rw [inherit, if_neg (fun accepted => rejected accepted.1)]

theorem inherited_not_seen (sources characters : Finset Int) (destination : Int)
    (seen : Finset (Int × Int)) (rows : List (Row Rate)) (result : Row Rate)
    (member : result ∈ inherit sources characters destination seen rows) : key result ∉ seen := by
  induction rows generalizing seen with
  | nil => simp [inherit] at member
  | cons row rest ih =>
      unfold inherit at member
      split at member
      next accepted =>
        rcases List.mem_cons.mp member with he | hm
        · subst result
          exact accepted.2
        · have h := ih (insert (key row) seen) hm
          intro hs
          exact h (Finset.mem_insert_of_mem hs)
      next => exact ih seen member

theorem inherited_keys_unique (sources characters : Finset Int) (destination : Int)
    (seen : Finset (Int × Int)) (rows : List (Row Rate)) :
    ((inherit sources characters destination seen rows).map key).Nodup := by
  induction rows generalizing seen with
  | nil => simp [inherit]
  | cons row rest ih =>
      unfold inherit
      split
      next =>
        simp only [List.map_cons, List.nodup_cons, relabel_key]
        constructor
        · intro hm
          obtain ⟨other, ho, hk⟩ := List.mem_map.mp hm
          have hn := inherited_not_seen sources characters destination (insert (key row) seen) rest other ho
          apply hn
          rw [hk]
          exact Finset.mem_insert_self _ _
        · exact ih _
      next => exact ih _

def findKey {R : Type*} (target : Int × Int) : List (Row R) → Option (Row R)
  | [] => none
  | row :: rest => if key row = target then some row else findKey target rest

def firstEligible (sources characters : Finset Int) (target : Int × Int) :
    List (Row Rate) → Option (Row Rate)
  | [] => none
  | row :: rest =>
      if eligible sources characters row = true ∧ key row = target then some row
      else firstEligible sources characters target rest

/-- Deduplication keeps the first row satisfying all three eligibility tests,
not the first source-event row, maximum rate, or a uniform replacement rate. -/
theorem inherits_first_eligible (sources characters : Finset Int) (destination : Int)
    (seen : Finset (Int × Int)) (rows : List (Row Rate)) (target : Int × Int)
    (fresh : target ∉ seen) :
    findKey target (inherit sources characters destination seen rows) =
      (firstEligible sources characters target rows).map (relabel destination) := by
  induction rows generalizing seen with
  | nil => rfl
  | cons row rest ih =>
      by_cases hs : eligible sources characters row = true
      · by_cases hk : key row = target
        · simp [inherit, firstEligible, findKey, hs, hk, fresh]
        · by_cases hn : key row ∈ seen
          · simp only [inherit, hn, not_true_eq_false, and_false, ↓reduceIte,
              firstEligible, hs, hk]
            exact ih seen fresh
          · have hf : target ∉ insert (key row) seen := by simp [fresh, Ne.symm hk]
            simpa only [inherit, hs, hn, not_false_eq_true, and_self, ↓reduceIte,
              findKey, relabel_key, hk, firstEligible, and_false] using ih _ hf
      · simpa only [inherit, hs, false_and, ↓reduceIte, firstEligible] using ih seen fresh

def inheritFinale (events : List Event) (chapters : List Chapter) (characterUnits : List Int)
    (destination : Int) (rows : List (Row Rate)) : List (Row Rate) :=
  inherit (sourceIds events chapters) (supportCharacters characterUnits) destination ∅ rows

theorem finale_first_eligible (events : List Event) (chapters : List Chapter) (characterUnits : List Int)
    (destination : Int) (rows : List (Row Rate)) (target : Int × Int) :
    findKey target (inheritFinale events chapters characterUnits destination rows) =
      (firstEligible (sourceIds events chapters) (supportCharacters characterUnits) target rows).map
        (relabel destination) :=
  inherits_first_eligible _ _ destination ∅ rows target (by simp)

theorem finale_output_domain (events : List Event) (chapters : List Chapter) (characterUnits : List Int)
    (destination : Int) (rows : List (Row Rate)) (result : Row Rate)
    (member : result ∈ inheritFinale events chapters characterUnits destination rows) :
    result.character ∈ characterUnits ∧ 1 ≤ result.character ∧ result.character ≤ 26 ∧
      0 < result.rate ∧ result.event = destination := by
  obtain ⟨row, _, accepted, he⟩ := inherited_provenance _ _ destination ∅ rows result member
  have valid := (eligible_iff _ _ row).mp accepted
  have hc := (mem_support_characters characterUnits row.character).mp valid.2.1
  rw [← he]
  exact ⟨hc.1, hc.2.1, hc.2.2, valid.2.2, rfl⟩

/-- An ineligible prefix never consumes the key of an eligible following row. -/
theorem nonpositive_then_positive (sources characters : Finset Int) (destination : Int)
    (first next : Row Rate) (nonpositive : first.rate ≤ 0)
    (accepted : eligible sources characters next = true) :
    inherit sources characters destination ∅ [first, next] = [relabel destination next] := by
  rw [nonpositive_skipped sources characters destination ∅ first [next] nonpositive]
  simp [inherit, accepted]

end Allium.SourceRows
