import Allium.ScenarioPower
import Allium.ScenarioSearch
import Allium.FiniteBounds
import Allium.Saturating
import Allium.SuffixScan
import Allium.Canonical

/-!
# Dedicated Power scenarios

The unit table is built by a growing intersection worklist. Scenario rows are
sorted by power, public ID and dense ID, and completion scans consume the first
unused entry of each character. Bounds concern each scenario's required class;
incidental admitted decks are evaluated from their actual composition.
-/
namespace Allium.PowerScenarios

abbrev Mask := Fin 64

def intersection (left right : Mask) : Mask :=
  ⟨left.val &&& right.val, Nat.and_le_left.trans_lt left.isLt⟩

def appendNew (values : List Mask) (value : Mask) : List Mask :=
  if value ∈ values then values else values ++ [value]

def initialMasks (masks : List Mask) : List Mask := masks.foldl appendNew [0]

/-- The inner loop reads only positions below the fixed cursor. Appended
intersections do not change either the current entry or that earlier prefix. -/
def advanceMasks (values : List Mask) (cursor : Nat) : List Mask :=
  ((values.take cursor).map (intersection ((values[cursor]?).getD 0))).foldl appendNew values

/-- There are only 64 distinct six-bit masks. The termination proof must show
that this fuel cannot expire with an unprocessed entry; it is not a truncation
of the mathematical search space. -/
def closeMasks : Nat → Nat → List Mask → List Mask
  | 0, _, values => values
  | fuel + 1, cursor, values =>
      if cursor < values.length then closeMasks fuel (cursor + 1) (advanceMasks values cursor) else values

def unitSets (masks : List Mask) : List Mask := closeMasks 64 1 (initialMasks masks)

@[simp] theorem append_new_membership (values : List Mask) (value item : Mask) :
    item ∈ appendNew values value ↔ item ∈ values ∨ item = value := by
  by_cases present : value ∈ values
  · rw [appendNew, if_pos present]
    constructor
    · exact Or.inl
    · rintro (member | rfl)
      · exact member
      · exact present
  · simp [appendNew, present]

theorem append_new_nodup (values : List Mask) (value : Mask) (unique : values.Nodup) :
    (appendNew values value).Nodup := by
  by_cases present : value ∈ values
  · simpa only [appendNew, if_pos present] using unique
  · rw [appendNew, if_neg present]
    apply List.nodup_append.mpr
    refine ⟨unique, by simp, ?_⟩
    intro previous member other singleton same
    have last : other = value := List.mem_singleton.mp singleton
    exact present (same.trans last ▸ member)

def Extends (before after : List Mask) : Prop := ∃ added, after = before ++ added

theorem extends_refl (values : List Mask) : Extends values values := ⟨[], by simp⟩

theorem extends_trans (first second third : List Mask) (left : Extends first second)
    (right : Extends second third) : Extends first third := by
  obtain ⟨a, rfl⟩ := left
  obtain ⟨b, rfl⟩ := right
  exact ⟨a ++ b, List.append_assoc _ _ _⟩

theorem extends_member (before after : List Mask) (extended : Extends before after)
    (value : Mask) (member : value ∈ before) : value ∈ after := by
  obtain ⟨added, rfl⟩ := extended
  exact List.mem_append_left _ member

theorem extends_lookup (before after : List Mask) (extended : Extends before after)
    (index : Nat) (bound : index < before.length) : after[index]? = before[index]? := by
  obtain ⟨added, rfl⟩ := extended
  exact List.getElem?_append_left bound

theorem append_new_extends (values : List Mask) (value : Mask) : Extends values (appendNew values value) := by
  by_cases present : value ∈ values
  · rw [appendNew, if_pos present]
    exact extends_refl values
  · exact ⟨[value], by simp [appendNew, present]⟩

theorem fold_extends (additions values : List Mask) : Extends values (additions.foldl appendNew values) := by
  induction additions generalizing values with
  | nil => exact extends_refl values
  | cons first rest ih =>
      exact extends_trans _ _ _ (append_new_extends values first) (ih (appendNew values first))

theorem fold_nodup (additions values : List Mask) (unique : values.Nodup) :
    (additions.foldl appendNew values).Nodup := by
  induction additions generalizing values with
  | nil => exact unique
  | cons first rest ih => exact ih _ (append_new_nodup values first unique)

theorem fold_membership (additions values : List Mask) (item : Mask) :
    item ∈ additions.foldl appendNew values ↔ item ∈ values ∨ item ∈ additions := by
  induction additions generalizing values with
  | nil => simp
  | cons first rest ih =>
      simp only [List.foldl_cons, ih, append_new_membership, List.mem_cons]
      tauto

theorem masks_length (values : List Mask) (unique : values.Nodup) : values.length ≤ 64 := by
  have bound := Finset.card_le_univ values.toFinset
  simpa only [List.toFinset_card_of_nodup unique, Fintype.card_fin] using bound

def Processed (cursor : Nat) (values : List Mask) : Prop :=
  ∀ left < cursor, left < values.length → ∀ right < left,
    intersection ((values[left]?).getD 0) ((values[right]?).getD 0) ∈ values

theorem advance_extends (values : List Mask) (cursor : Nat) : Extends values (advanceMasks values cursor) :=
  fold_extends _ _

theorem advance_nodup (values : List Mask) (cursor : Nat) (unique : values.Nodup) :
    (advanceMasks values cursor).Nodup := fold_nodup _ _ unique

theorem advance_pair (values : List Mask) (cursor right : Nat)
    (active : cursor < values.length) (earlier : right < cursor) :
    intersection ((values[cursor]?).getD 0) ((values[right]?).getD 0) ∈ advanceMasks values cursor := by
  apply (fold_membership _ _ _).mpr
  right
  apply List.mem_map.mpr
  refine ⟨(values[right]?).getD 0, ?_, rfl⟩
  have bound : right < values.length := earlier.trans active
  have taken : right < (values.take cursor).length := by simp; omega
  have member := List.getElem_mem taken
  simpa only [List.getElem_take, List.getElem?_eq_getElem bound, Option.getD_some] using member

theorem advance_processed (values : List Mask) (cursor : Nat) (active : cursor < values.length)
    (processed : Processed cursor values) : Processed (cursor + 1) (advanceMasks values cursor) := by
  intro left leftCursor _ right rightLeft
  have extension := advance_extends values cursor
  by_cases current : left = cursor
  · subst left
    rw [extends_lookup values _ extension cursor active,
      extends_lookup values _ extension right (rightLeft.trans active)]
    exact advance_pair values cursor right active rightLeft
  · have earlier : left < cursor := by omega
    have leftBound : left < values.length := earlier.trans active
    rw [extends_lookup values _ extension left leftBound,
      extends_lookup values _ extension right (rightLeft.trans leftBound)]
    exact extends_member values _ extension _ (processed left earlier leftBound right rightLeft)

theorem close_masks_spec (fuel cursor : Nat) (values : List Mask) (unique : values.Nodup)
    (processed : Processed cursor values) (enough : 64 < cursor + fuel) :
    Extends values (closeMasks fuel cursor values) ∧ (closeMasks fuel cursor values).Nodup ∧
      Processed (closeMasks fuel cursor values).length (closeMasks fuel cursor values) := by
  induction fuel generalizing cursor values with
  | zero =>
      change Extends values values ∧ values.Nodup ∧ Processed values.length values
      refine ⟨extends_refl values, unique, ?_⟩
      have size := masks_length values unique
      intro left leftBound _ right rightLeft
      exact processed left (by omega) leftBound right rightLeft
  | succ fuel ih =>
      by_cases active : cursor < values.length
      · have next := ih (cursor + 1) (advanceMasks values cursor)
          (advance_nodup values cursor unique) (advance_processed values cursor active processed) (by omega)
        rw [closeMasks, if_pos active]
        exact ⟨extends_trans _ _ _ (advance_extends values cursor) next.1, next.2⟩
      · rw [closeMasks, if_neg active]
        refine ⟨extends_refl values, unique, ?_⟩
        intro left leftBound _ right rightLeft
        exact processed left (by omega) leftBound right rightLeft

theorem intersection_comm (left right : Mask) : intersection left right = intersection right left := by
  apply Fin.ext
  exact Nat.and_comm _ _

@[simp] theorem intersection_self (value : Mask) : intersection value value = value := by
  apply Fin.ext
  exact Nat.and_self _

theorem processed_closed (values : List Mask) (complete : Processed values.length values)
    (left right : Mask) (leftMember : left ∈ values) (rightMember : right ∈ values) :
    intersection left right ∈ values := by
  cases leftFound : Admission.Intern.firstIndex left values with
  | none => exact False.elim ((Admission.Intern.first_index_none left values).mp leftFound leftMember)
  | some leftIndex =>
      obtain ⟨leftBound, leftLookup⟩ := Admission.Intern.first_index_lookup left values leftIndex leftFound
      cases rightFound : Admission.Intern.firstIndex right values with
      | none => exact False.elim ((Admission.Intern.first_index_none right values).mp rightFound rightMember)
      | some rightIndex =>
          obtain ⟨rightBound, rightLookup⟩ := Admission.Intern.first_index_lookup right values rightIndex rightFound
          rcases lt_trichotomy leftIndex rightIndex with less | same | greater
          · have pair := complete rightIndex rightBound rightBound leftIndex less
            simpa only [leftLookup, rightLookup, Option.getD_some, intersection_comm right left] using pair
          · have equal : left = right := by
              rw [same, rightLookup] at leftLookup
              exact (Option.some.inj leftLookup).symm
            simpa only [equal, intersection_self] using rightMember
          · have pair := complete leftIndex leftBound leftBound rightIndex greater
            simpa only [leftLookup, rightLookup, Option.getD_some] using pair

theorem unit_sets_spec (masks : List Mask) :
    (unitSets masks).Nodup ∧ (∀ value ∈ masks, value ∈ unitSets masks) ∧ 0 ∈ unitSets masks ∧
      ∀ left ∈ unitSets masks, ∀ right ∈ unitSets masks, intersection left right ∈ unitSets masks := by
  have unique : (initialMasks masks).Nodup := fold_nodup masks [0] (by simp)
  have processed : Processed 1 (initialMasks masks) := by
    intro left leftSmall _ right rightSmall
    omega
  have closed := close_masks_spec 64 1 (initialMasks masks) unique processed (by decide)
  refine ⟨closed.2.1, ?_, ?_, fun left hleft right hright => processed_closed _ closed.2.2 left right hleft hright⟩
  · intro value member
    exact extends_member _ _ closed.1 value ((fold_membership masks [0] value).mpr (Or.inr member))
  · exact extends_member _ _ closed.1 0 ((fold_membership masks [0] 0).mpr (Or.inl (by simp)))

theorem fold_intersection_mem (table : List Mask)
    (closed : ∀ left ∈ table, ∀ right ∈ table, intersection left right ∈ table)
    (masks : List Mask) (initial : Mask) (start : initial ∈ table)
    (members : ∀ value ∈ masks, value ∈ table) : masks.foldl intersection initial ∈ table := by
  induction masks generalizing initial with
  | nil => exact start
  | cons first rest ih =>
      apply ih (intersection initial first)
      · exact closed initial start first (members first (by simp))
      · intro value member
        exact members value (List.mem_cons_of_mem _ member)

theorem unit_sets_cover_intersection (pool : List Mask) (first : Mask) (rest : List Mask)
    (members : ∀ value ∈ first :: rest, value ∈ pool) :
    rest.foldl intersection first ∈ unitSets pool := by
  have spec := unit_sets_spec pool
  apply fold_intersection_mem _ spec.2.2.2 rest first
  · exact spec.2.1 first (members first (by simp))
  · intro value member
    exact spec.2.1 value (members value (List.mem_cons_of_mem _ member))

theorem membership_intersection (left right : Mask) :
    MixedPower.rawMembership (intersection left right) =
      MixedPower.rawMembership left ∩ MixedPower.rawMembership right := by
  ext unit
  simp [MixedPower.rawMembership, intersection, Nat.testBit_and]

@[simp] theorem full_intersection (mask : Mask) : intersection 63 mask = mask := by
  apply Fin.ext
  change 63 &&& mask.val = mask.val
  rw [Nat.and_comm]
  change mask.val &&& (2 ^ 6 - 1) = mask.val
  rw [Nat.and_two_pow_sub_one_eq_mod, Nat.mod_eq_of_lt mask.isLt]

def commonMask (masks : List Mask) : Mask := masks.foldl intersection 63

theorem common_mask_covered (pool masks : List Mask) (nonempty : masks ≠ [])
    (members : ∀ mask ∈ masks, mask ∈ pool) : commonMask masks ∈ unitSets pool := by
  cases masks with
  | nil => exact False.elim (nonempty rfl)
  | cons first rest =>
      simpa only [commonMask, List.foldl_cons, full_intersection] using
        unit_sets_cover_intersection pool first rest members

theorem membership_fold (masks : List Mask) (initial : Mask) :
    MixedPower.rawMembership (masks.foldl intersection initial) =
      MixedPower.rawMembership initial ∩
        masks.foldr (fun mask units => MixedPower.rawMembership mask ∩ units) Finset.univ := by
  induction masks generalizing initial with
  | nil => simp
  | cons first rest ih =>
      simp only [List.foldl_cons, ih, membership_intersection, List.foldr_cons, Finset.inter_assoc]

theorem common_mask_membership (masks : List Mask) :
    MixedPower.rawMembership (commonMask masks) =
      masks.foldr (fun mask units => MixedPower.rawMembership mask ∩ units) Finset.univ := by
  have full : MixedPower.rawMembership 63 = Finset.univ := by decide
  simp only [commonMask, membership_fold, full, Finset.univ_inter]

theorem intersection_subset_exact : ∀ left right : Mask,
    intersection left right = left ↔ MixedPower.rawMembership left ⊆ MixedPower.rawMembership right := by decide

structure Entry where
  power : Nat
  dense : Nat
  publicId : Fin 65536
  character : Fin 27

def entryKey (entry : Entry) : List Int :=
  [-Int.ofNat entry.power, Int.ofNat entry.publicId.val, Int.ofNat entry.dense]

def sortedEntries (entries : List Entry) : List Entry :=
  entries.mergeSort (fun left right => decide (entryKey left ≤ entryKey right))

theorem sorted_entries_keys (entries : List Entry) :
    (sortedEntries entries).Pairwise (fun left right => entryKey left ≤ entryKey right) := by
  simpa only [sortedEntries, decide_eq_true_eq] using
    List.sorted_mergeSort (le := fun left right => decide (entryKey left ≤ entryKey right))
      (by
        intro a b c ab bc
        simp only [decide_eq_true_eq] at ab bc ⊢
        exact ab.trans bc)
      (by
        intro a b
        simpa using le_total (entryKey a) (entryKey b)) entries

theorem entry_key_power (left right : Entry) (ordered : entryKey left ≤ entryKey right) :
    right.power ≤ left.power := by
  have heads : -Int.ofNat left.power ≤ -Int.ofNat right.power := by
    rcases lt_or_eq_of_le ordered with smaller | equal
    · exact List.head_le_of_lt smaller
    · exact le_of_eq (List.cons.inj equal).1
  exact Int.ofNat_le.mp (neg_le_neg_iff.mp heads)

theorem sorted_entries_power (entries : List Entry) :
    (sortedEntries entries).Pairwise (fun left right => right.power ≤ left.power) :=
  (sorted_entries_keys entries).imp (fun ordered => entry_key_power _ _ ordered)

/-- Direct projection of best_completion's seen/taken loop. -/
def completionEntries : List Entry → Finset (Fin 27) → Nat → List Entry
  | _, _, 0 => []
  | [], _, _ + 1 => []
  | entry :: rest, used, slots + 1 =>
      if entry.character ∈ used then completionEntries rest used (slots + 1)
      else entry :: completionEntries rest (insert entry.character used) slots

theorem completion_length (entries : List Entry) (used : Finset (Fin 27)) (slots : Nat) :
    (completionEntries entries used slots).length ≤ slots := by
  induction entries generalizing used slots with
  | nil => cases slots <;> simp [completionEntries]
  | cons first rest ih =>
      cases slots with
      | zero => simp [completionEntries]
      | succ slots =>
          by_cases seen : first.character ∈ used
          · simpa only [completionEntries, if_pos seen] using ih used (slots + 1)
          · simpa only [completionEntries, if_neg seen, List.length_cons, Nat.add_le_add_iff_right]
              using ih (insert first.character used) slots

theorem completion_members (entries : List Entry) (used : Finset (Fin 27)) (slots : Nat)
    (entry : Entry) (member : entry ∈ completionEntries entries used slots) : entry ∈ entries := by
  induction entries generalizing used slots with
  | nil => cases slots <;> simp [completionEntries] at member
  | cons first rest ih =>
      cases slots with
      | zero => simp [completionEntries] at member
      | succ slots =>
          by_cases seen : first.character ∈ used
          · rw [completionEntries, if_pos seen] at member
            exact List.mem_cons_of_mem _ (ih used (slots + 1) member)
          · rw [completionEntries, if_neg seen] at member
            rcases List.mem_cons.mp member with same | tail
            · exact List.mem_cons.mpr (Or.inl same)
            · exact List.mem_cons_of_mem _ (ih (insert first.character used) slots tail)

theorem completion_fresh (entries : List Entry) (used : Finset (Fin 27)) (slots : Nat)
    (entry : Entry) (member : entry ∈ completionEntries entries used slots) : entry.character ∉ used := by
  induction entries generalizing used slots with
  | nil => cases slots <;> simp [completionEntries] at member
  | cons first rest ih =>
      cases slots with
      | zero => simp [completionEntries] at member
      | succ slots =>
          by_cases seen : first.character ∈ used
          · rw [completionEntries, if_pos seen] at member
            exact ih used (slots + 1) member
          · rw [completionEntries, if_neg seen] at member
            rcases List.mem_cons.mp member with same | tail
            · simpa only [same] using seen
            · have fresh := ih (insert first.character used) slots tail
              exact fun old => fresh (Finset.mem_insert_of_mem old)

theorem completion_unique (entries : List Entry) (used : Finset (Fin 27)) (slots : Nat) :
    ((completionEntries entries used slots).map Entry.character).Nodup := by
  induction entries generalizing used slots with
  | nil => cases slots <;> simp [completionEntries]
  | cons first rest ih =>
      cases slots with
      | zero => simp [completionEntries]
      | succ slots =>
          by_cases seen : first.character ∈ used
          · simpa only [completionEntries, if_pos seen] using ih used (slots + 1)
          · rw [completionEntries, if_neg seen, List.map_cons, List.nodup_cons]
            refine ⟨?_, ih (insert first.character used) slots⟩
            rintro member
            obtain ⟨entry, belongs, same⟩ := List.mem_map.mp member
            have fresh := completion_fresh rest (insert first.character used) slots entry belongs
            exact fresh (by simp [same])

theorem completion_exhausted (entries : List Entry) (used : Finset (Fin 27)) (slots : Nat)
    (short : (completionEntries entries used slots).length < slots)
    (entry : Entry) (member : entry ∈ entries) :
    entry.character ∈ used ∨ entry.character ∈ (completionEntries entries used slots).map Entry.character := by
  induction entries generalizing used slots with
  | nil => simp at member
  | cons first rest ih =>
      cases slots with
      | zero => omega
      | succ slots =>
          by_cases seen : first.character ∈ used
          · rw [completionEntries, if_pos seen] at short ⊢
            rcases List.mem_cons.mp member with same | tail
            · exact Or.inl (same ▸ seen)
            · exact ih used (slots + 1) short tail
          · rw [completionEntries, if_neg seen] at short ⊢
            rcases List.mem_cons.mp member with same | tail
            · right
              simp only [List.map_cons, List.mem_cons]
              exact Or.inl (congrArg Entry.character same)
            · have remaining : (completionEntries rest (insert first.character used) slots).length < slots := by
                simpa only [List.length_cons, Nat.add_lt_add_iff_right] using short
              rcases ih (insert first.character used) slots remaining tail with inserted | taken
              · rcases Finset.mem_insert.mp inserted with same | old
                · exact Or.inr (by simp only [List.map_cons, List.mem_cons]; exact Or.inl same)
                · exact Or.inl old
              · exact Or.inr (List.mem_cons_of_mem _ taken)

theorem completion_sufficient (entries candidates : List Entry) (used : Finset (Fin 27))
    (slots : Nat) (size : candidates.length = slots)
    (unique : (candidates.map Entry.character).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ entries ∧ entry.character ∉ used) :
    (completionEntries entries used slots).length = slots := by
  have limit := completion_length entries used slots
  by_contra different
  have short : (completionEntries entries used slots).length < slots := by omega
  have included : (candidates.map Entry.character).toFinset ⊆
      ((completionEntries entries used slots).map Entry.character).toFinset := by
    intro character member
    obtain ⟨entry, belongs, rfl⟩ := List.mem_map.mp (List.mem_toFinset.mp member)
    have facts := available entry belongs
    have exhausted := completion_exhausted entries used slots short entry facts.1
    exact List.mem_toFinset.mpr (exhausted.resolve_left facts.2)
  have count := Finset.card_le_card included
  have lengthBound : candidates.length ≤ (completionEntries entries used slots).length := by
    simpa only [List.toFinset_card_of_nodup unique,
      List.toFinset_card_of_nodup (completion_unique entries used slots), List.length_map] using count
  omega

theorem exchange_candidate (first : Entry) (candidates : List Entry)
    (nonempty : candidates ≠ []) (unique : (candidates.map Entry.character).Nodup) :
    ∃ chosen rest, candidates.Perm (chosen :: rest) ∧
      ∀ entry ∈ rest, entry.character ≠ first.character := by
  classical
  by_cases present : ∃ chosen ∈ candidates, chosen.character = first.character
  · obtain ⟨chosen, member, same⟩ := present
    have permutation := List.perm_cons_erase member
    refine ⟨chosen, candidates.erase chosen, permutation, ?_⟩
    have distinct := (permutation.map Entry.character).nodup_iff.mp unique
    simp only [List.map_cons, List.nodup_cons] at distinct
    intro entry belongs equal
    apply distinct.1
    exact List.mem_map.mpr ⟨entry, belongs, equal.trans same.symm⟩
  · cases candidates with
    | nil => exact False.elim (nonempty rfl)
    | cons chosen rest =>
        refine ⟨chosen, rest, List.Perm.refl _, ?_⟩
        intro entry belongs equal
        exact present ⟨entry, List.mem_cons_of_mem _ belongs, equal⟩

theorem completion_maximal (entries candidates : List Entry) (used : Finset (Fin 27))
    (slots : Nat) (ordered : entries.Pairwise (fun left right => right.power ≤ left.power))
    (size : candidates.length = slots) (unique : (candidates.map Entry.character).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ entries ∧ entry.character ∉ used) :
    (candidates.map Entry.power).sum ≤ ((completionEntries entries used slots).map Entry.power).sum := by
  induction entries generalizing candidates used slots with
  | nil =>
      have empty : candidates = [] := by
        cases candidates with
        | nil => rfl
        | cons first rest => exact False.elim (by simpa using (available first (by simp)).1)
      subst candidates
      simp
  | cons first rest ih =>
      have ordering := List.pairwise_cons.mp ordered
      cases slots with
      | zero =>
          have empty : candidates = [] := List.length_eq_zero_iff.mp size
          subst candidates
          simp [completionEntries]
      | succ slots =>
          by_cases seen : first.character ∈ used
          · rw [completionEntries, if_pos seen]
            apply ih candidates used (slots + 1) ordering.2 size unique
            intro entry belongs
            have facts := available entry belongs
            refine ⟨?_, facts.2⟩
            rcases List.mem_cons.mp facts.1 with same | tail
            · exact False.elim (facts.2 (same ▸ seen))
            · exact tail
          · obtain ⟨chosen, remaining, permutation, avoids⟩ := exchange_candidate first candidates
              (by intro empty; simp [empty] at size) unique
            have chosenMember : chosen ∈ candidates := permutation.mem_iff.mpr (by simp)
            have chosenFacts := available chosen chosenMember
            have chosenBound : chosen.power ≤ first.power := by
              rcases List.mem_cons.mp chosenFacts.1 with same | tail
              · simp only [same, le_refl]
              · exact ordering.1 chosen tail
            have remainingSize : remaining.length = slots := by
              have length := permutation.length_eq
              simp only [List.length_cons] at length
              omega
            have remainingUnique := (permutation.map Entry.character).nodup_iff.mp unique
            simp only [List.map_cons, List.nodup_cons] at remainingUnique
            have tailBound := ih remaining (insert first.character used) slots ordering.2 remainingSize
              remainingUnique.2 (by
                intro entry belongs
                have facts := available entry (permutation.mem_iff.mpr (List.mem_cons_of_mem _ belongs))
                have different := avoids entry belongs
                refine ⟨?_, ?_⟩
                · rcases List.mem_cons.mp facts.1 with same | tail
                  · exact False.elim (different (congrArg Entry.character same))
                  · exact tail
                · simp only [Finset.mem_insert, not_or]
                  exact ⟨different, facts.2⟩)
            have sumEqual := (permutation.map Entry.power).sum_eq
            rw [completionEntries, if_neg seen, List.map_cons, List.sum_cons]
            simp only [List.map_cons, List.sum_cons] at sumEqual
            omega

theorem completion_optimal_powers (entries candidates : List Entry) (used : Finset (Fin 27))
    (slots : Nat) (ordered : entries.Pairwise (fun left right => right.power ≤ left.power))
    (size : candidates.length = slots) (unique : (candidates.map Entry.character).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ entries ∧ entry.character ∉ used)
    (equalSum : (candidates.map Entry.power).sum =
      ((completionEntries entries used slots).map Entry.power).sum) :
    (candidates.map Entry.power).Perm ((completionEntries entries used slots).map Entry.power) := by
  induction entries generalizing candidates used slots with
  | nil =>
      have empty : candidates = [] := by
        cases candidates with
        | nil => rfl
        | cons first rest => exact False.elim (by simpa using (available first (by simp)).1)
      subst candidates
      have zero : slots = 0 := size.symm
      subst slots
      exact List.Perm.refl _
  | cons first rest ih =>
      have ordering := List.pairwise_cons.mp ordered
      cases slots with
      | zero =>
          have empty : candidates = [] := List.length_eq_zero_iff.mp size
          subst candidates
          exact List.Perm.refl _
      | succ slots =>
          by_cases seen : first.character ∈ used
          · rw [completionEntries, if_pos seen] at equalSum ⊢
            apply ih candidates used (slots + 1) ordering.2 size unique _ equalSum
            intro entry belongs
            have facts := available entry belongs
            refine ⟨?_, facts.2⟩
            rcases List.mem_cons.mp facts.1 with same | tail
            · exact False.elim (facts.2 (same ▸ seen))
            · exact tail
          · obtain ⟨chosen, remaining, permutation, avoids⟩ := exchange_candidate first candidates
              (by intro empty; simp [empty] at size) unique
            have chosenFacts := available chosen (permutation.mem_iff.mpr (by simp))
            have chosenBound : chosen.power ≤ first.power := by
              rcases List.mem_cons.mp chosenFacts.1 with same | tail
              · simp only [same, le_refl]
              · exact ordering.1 chosen tail
            have remainingSize : remaining.length = slots := by
              have length := permutation.length_eq
              simp only [List.length_cons] at length
              omega
            have remainingUnique := (permutation.map Entry.character).nodup_iff.mp unique
            simp only [List.map_cons, List.nodup_cons] at remainingUnique
            have tailAvailable : ∀ entry ∈ remaining, entry ∈ rest ∧ entry.character ∉ insert first.character used := by
              intro entry belongs
              have facts := available entry (permutation.mem_iff.mpr (List.mem_cons_of_mem _ belongs))
              have different := avoids entry belongs
              refine ⟨?_, ?_⟩
              · rcases List.mem_cons.mp facts.1 with same | tail
                · exact False.elim (different (congrArg Entry.character same))
                · exact tail
              · simp only [Finset.mem_insert, not_or]
                exact ⟨different, facts.2⟩
            have tailBound := completion_maximal rest remaining (insert first.character used) slots
              ordering.2 remainingSize remainingUnique.2 tailAvailable
            have sumSplit := (permutation.map Entry.power).sum_eq
            simp only [List.map_cons, List.sum_cons] at sumSplit
            rw [completionEntries, if_neg seen, List.map_cons, List.sum_cons] at equalSum
            have headEqual : chosen.power = first.power := by omega
            have tailEqual : (remaining.map Entry.power).sum =
                ((completionEntries rest (insert first.character used) slots).map Entry.power).sum := by omega
            have tailPerm := ih remaining (insert first.character used) slots ordering.2 remainingSize
              remainingUnique.2 tailAvailable tailEqual
            rw [completionEntries, if_neg seen, List.map_cons]
            apply (permutation.map Entry.power).trans
            simpa only [List.map_cons, headEqual] using List.Perm.cons first.power tailPerm

theorem completion_sublist (entries : List Entry) (used : Finset (Fin 27)) (slots : Nat) :
    (completionEntries entries used slots).Sublist entries := by
  induction entries generalizing used slots with
  | nil => cases slots <;> simp [completionEntries]
  | cons first rest ih =>
      cases slots with
      | zero => simp [completionEntries]
      | succ slots =>
          by_cases seen : first.character ∈ used
          · rw [completionEntries, if_pos seen]
            exact (ih used (slots + 1)).cons first
          · rw [completionEntries, if_neg seen]
            exact (ih (insert first.character used) slots).cons₂ first

theorem last_power_lower (entries : List Entry)
    (ordered : entries.Pairwise (fun left right => right.power ≤ left.power))
    (entry : Entry) (member : entry ∈ entries) :
    (entries.getLast?.map Entry.power).getD 0 ≤ entry.power := by
  induction entries generalizing entry with
  | nil => simp at member
  | cons first rest ih =>
      have ordering := List.pairwise_cons.mp ordered
      cases rest with
      | nil =>
          have same := List.mem_singleton.mp member
          simp [same]
      | cons second tail =>
          rw [List.getLast?_cons_cons]
          rcases List.mem_cons.mp member with same | belongs
          · subst entry
            cases found : (second :: tail).getLast? with
            | none => simp
            | some last =>
                simpa only [Option.map_some, Option.getD_some] using
                  ordering.1 last (List.mem_of_getLast? found)
          · exact ih ordering.2 entry belongs

theorem completion_least_lower (entries : List Entry) (used : Finset (Fin 27)) (slots : Nat)
    (ordered : entries.Pairwise (fun left right => right.power ≤ left.power))
    (entry : Entry) (member : entry ∈ completionEntries entries used slots) :
    ((completionEntries entries used slots).getLast?.map Entry.power).getD 0 ≤ entry.power :=
  last_power_lower _ (ordered.sublist (completion_sublist entries used slots)) entry member

theorem optimal_completion_least (entries candidates : List Entry) (used : Finset (Fin 27))
    (slots : Nat) (ordered : entries.Pairwise (fun left right => right.power ≤ left.power))
    (size : candidates.length = slots) (unique : (candidates.map Entry.character).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ entries ∧ entry.character ∉ used)
    (equalSum : (candidates.map Entry.power).sum =
      ((completionEntries entries used slots).map Entry.power).sum)
    (entry : Entry) (member : entry ∈ candidates) :
    ((completionEntries entries used slots).getLast?.map Entry.power).getD 0 ≤ entry.power := by
  have permutation := completion_optimal_powers entries candidates used slots ordered size unique available equalSum
  have matched := permutation.mem_iff.mp (List.mem_map.mpr ⟨entry, member, rfl⟩)
  obtain ⟨selected, belongs, same⟩ := List.mem_map.mp matched
  rw [← same]
  exact completion_least_lower entries used slots ordered selected belongs

theorem selected_completion_unique (selected entries : List Entry) (slots : Nat)
    (unique : (selected.map Entry.character).Nodup) :
    ((selected ++ completionEntries entries (selected.map Entry.character).toFinset slots).map Entry.character).Nodup := by
  rw [List.map_append, List.nodup_append]
  refine ⟨unique, completion_unique _ _ _, ?_⟩
  intro character member other taken same
  obtain ⟨entry, belongs, equal⟩ := List.mem_map.mp taken
  have fresh := completion_fresh entries (selected.map Entry.character).toFinset slots entry belongs
  apply fresh
  apply List.mem_toFinset.mpr
  rw [equal]
  exact same ▸ member

def bestCompletion (entries : List Entry) (pos : Nat) (used : Finset (Fin 27)) (slots : Nat) : Option (Nat × Nat) :=
  let taken := completionEntries (entries.drop pos) used slots
  if taken.length = slots then some ((taken.map Entry.power).sum, (taken.getLast?.map Entry.power).getD 0) else none

theorem best_completion_spec (entries candidates : List Entry) (pos : Nat)
    (used : Finset (Fin 27)) (slots : Nat) (size : candidates.length = slots)
    (unique : (candidates.map Entry.character).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ (sortedEntries entries).drop pos ∧ entry.character ∉ used) :
    ∃ sum least, bestCompletion (sortedEntries entries) pos used slots = some (sum, least) ∧
      (candidates.map Entry.power).sum ≤ sum ∧
      ((candidates.map Entry.power).sum = sum → ∀ entry ∈ candidates, least ≤ entry.power) := by
  have ordered : ((sortedEntries entries).drop pos).Pairwise (fun left right => right.power ≤ left.power) :=
    (sorted_entries_power entries).drop
  have full := completion_sufficient _ candidates used slots size unique available
  refine ⟨_, ((completionEntries ((sortedEntries entries).drop pos) used slots).getLast?.map Entry.power).getD 0,
    ?_, completion_maximal _ candidates used slots ordered size unique available, ?_⟩
  · simp [bestCompletion, full]
  · intro equalSum entry member
    exact optimal_completion_least _ candidates used slots ordered size unique available equalSum entry member

namespace PowerAdmission

def limitBits : Binary64.PositiveBits := ⟨1047 * 2 ^ 52, by norm_num⟩
def successorBits : Binary64.PositiveBits := ⟨1047 * 2 ^ 52 + 2 ^ 28, by norm_num⟩

theorem limit_magnitude : Binary64.magnitude limitBits = (2 : ℚ) ^ (24 : Nat) := by
  norm_num [Binary64.magnitude, Binary64.exponent, Binary64.fraction, limitBits]

theorem successor_magnitude : Binary64.magnitude successorBits = (2 : ℚ) ^ (24 : Nat) + 1 := by
  norm_num [Binary64.magnitude, Binary64.exponent, Binary64.fraction, successorBits]

theorem closest_limit : Binary64.magnitude (Binary64.closest ((2 ^ 24 : Nat) : ℚ)) = (2 : ℚ) ^ (24 : Nat) := by
  have nearest := Binary64.nearest_distance (((2 ^ 24 : Nat) : ℚ)) limitBits
  rw [limit_magnitude] at nearest
  norm_num only [Nat.cast_pow, Nat.cast_ofNat, abs_of_nonneg (by positivity : (0 : ℚ) ≤ 2 ^ (24 : Nat)),
    sub_self, abs_zero] at nearest
  exact sub_eq_zero.mp (abs_eq_zero.mp (le_antisymm nearest (abs_nonneg _)))

theorem limit_finite : Binary64.ofNat (2 ^ 24) =
    .finite false (Binary64.closest ((2 ^ 24 : Nat) : ℚ)) := by
  have below : |((2 ^ 24 : Nat) : ℚ)| < Binary64.overflowMidpoint := by
    have large : (1 : ℚ) ≤ 2 ^ (970 : Nat) := one_le_pow₀ (by norm_num)
    have split : (2 : ℚ) ^ (1024 : Nat) = 2 ^ (970 : Nat) * 2 ^ (54 : Nat) := by
      rw [← pow_add]
    unfold Binary64.overflowMidpoint
    rw [split]
    generalize power : (2 : ℚ) ^ (970 : Nat) = largePower at *
    norm_num
    linarith only [large]
  simp only [Binary64.ofNat, Binary64.round, if_neg (not_le.mpr below)]
  congr 1

theorem accepted_integer_limit (value : Nat)
    (accepted : Binary64.le (Binary64.ofNat value) (Binary64.ofNat (2 ^ 24)) = true) : value ≤ 2 ^ 24 := by
  rw [limit_finite] at accepted
  have nonnegative : (0 : ℚ) ≤ value := Nat.cast_nonneg value
  have unsigned : decide ((value : ℚ) < 0) = false := by simp
  have absolute : |(value : ℚ)| = value := abs_of_nonneg nonnegative
  by_cases overflow : Binary64.overflowMidpoint ≤ (value : ℚ)
  · simp [Binary64.ofNat, Binary64.round, absolute, overflow, unsigned, Binary64.le] at accepted
  · have upper : Binary64.magnitude (Binary64.closest (value : ℚ)) ≤ (2 : ℚ) ^ (24 : Nat) := by
      simpa only [Binary64.ofNat, Binary64.round, absolute, if_neg overflow, unsigned,
        Binary64.le, Bool.false_eq_true, ↓reduceIte, decide_eq_true_eq, closest_limit] using accepted
    by_contra rejected
    have greater : (2 : ℚ) ^ (24 : Nat) + 1 ≤ (value : ℚ) := by
      have natural : 2 ^ 24 + 1 ≤ value := by omega
      exact_mod_cast natural
    have nearest := Binary64.nearest_distance (value : ℚ) successorBits
    rw [absolute, successor_magnitude,
      abs_of_nonpos (by linarith : Binary64.magnitude (Binary64.closest (value : ℚ)) - (value : ℚ) ≤ 0),
      abs_of_nonpos (by linarith : (2 : ℚ) ^ (24 : Nat) + 1 - (value : ℚ) ≤ 0)] at nearest
    linarith

theorem numeric_raw_limit (pool : Admission.GatherPipeline.Pool) (context : Admission.SourceNumeric.Context)
    (accepted : Admission.SourceNumeric.numericDomain pool context = .ok ()) :
    Admission.SourceNumeric.rawPower pool context ≤ 2 ^ 24 := by
  apply accepted_integer_limit
  have required : Admission.NumericFlow.Ceiling.power ∈
      Admission.NumericFlow.required context.target context.hasEvent context.effectiveLive := by
    unfold Admission.NumericFlow.required
    split
    · simp
    · split <;> simp
  exact Admission.SourceNumeric.successful_ceiling pool context accepted .power required

theorem built_raw_limit (cards : List Admission.GatherOrder.Card) (context : Admission.SourceNumeric.Context)
    (pool : Admission.GatherPipeline.Pool) (accepted : Admission.SourceNumeric.build cards context = .ok pool) :
    Admission.SourceNumeric.rawPower pool context ≤ 2 ^ 24 :=
  numeric_raw_limit pool context (Admission.SourceNumeric.concrete_success cards context pool accepted).2.2.1

theorem sorted_weights {Id : Type*} (weight : Id → Nat) (values : List Id) :
    (SuffixDelta.ordered weight values).map weight =
      (values.map weight).mergeSort (fun a b => decide (b ≤ a)) := by
  have permutation := (List.mergeSort_perm values (fun a b => decide (weight b ≤ weight a))).map weight
  have leftSorted : ((SuffixDelta.ordered weight values).map weight).Pairwise (fun a b => b ≤ a) :=
    List.pairwise_map.mpr (SuffixDelta.ordered_pairwise weight values)
  have rightSorted := SuffixDelta.ordered_pairwise (fun value : Nat => value) (values.map weight)
  exact List.eq_of_perm_of_sorted (permutation.trans (List.mergeSort_perm _ _).symm) leftSorted rightSorted

theorem top_five_bounds {Id : Type*} [DecidableEq Id] (weight : Id → Nat) (pool picks : List Id)
    (poolUnique : pool.Nodup) (unique : picks.Nodup) (count : picks.length ≤ 5)
    (members : ∀ value ∈ picks, value ∈ pool) :
    (picks.map weight).sum ≤ Admission.SourceNumeric.greatestFive (pool.map weight) := by
  have permutation := List.mergeSort_perm pool (fun a b => decide (weight b ≤ weight a))
  have sortedUnique := permutation.nodup_iff.mpr poolUnique
  have inside : picks.toFinset ⊆ (SuffixDelta.ordered weight pool).toFinset := by
    intro value member
    exact List.mem_toFinset.mpr (permutation.mem_iff.mpr (members value (List.mem_toFinset.mp member)))
  have capacity : picks.toFinset.card ≤ 5 := by
    simpa only [List.toFinset_card_of_nodup unique] using count
  have bound := SuffixScan.prefix_dominates weight _ (SuffixDelta.ordered_pairwise weight pool)
    sortedUnique 5 picks.toFinset inside capacity
  rw [SuffixScan.sum_toFinset weight picks unique] at bound
  rw [Admission.SourceNumeric.greatestFive, ← sorted_weights, ← List.map_take]
  exact bound

theorem stored_picks_limit (pool : Admission.GatherPipeline.Pool) (context : Admission.SourceNumeric.Context)
    (accepted : Admission.SourceNumeric.numericDomain pool context = .ok ())
    (picks : List (Fin pool.cards.length)) (unique : picks.Nodup) (count : picks.length ≤ 5) :
    (picks.map (fun index => (pool.cards[index]).powerMax.value.toNat)).sum + context.honor.val ≤ 2 ^ 24 := by
  let weight := fun index : Fin pool.cards.length => (pool.cards[index]).powerMax.value.toNat
  have bound := top_five_bounds weight (List.finRange pool.cards.length) picks
    (List.nodup_finRange _) unique count (fun index _ => List.mem_finRange index)
  have columns : (List.finRange pool.cards.length).map weight = Admission.GatherImage.powerValues pool := by
    rw [Admission.GatherImage.powers_exact, ← List.ofFn_eq_map]
    exact List.ofFn_getElem_eq_map pool.cards (fun card => card.powerMax.value.toNat)
  rw [columns] at bound
  have limit := numeric_raw_limit pool context accepted
  unfold Admission.SourceNumeric.rawPower at limit
  exact (Nat.add_le_add_right bound context.honor.val).trans limit

end PowerAdmission

namespace PowerProduction

def carriedUnits (mask : Mask) : List PowerModel.UnitId :=
  (List.finRange 6).filter (fun unit => mask.val.testBit unit.val)

/-- The SIMD primary/secondary lanes use the two lowest set bits; masks with
more than two bits use the scalar per-unit loop in the same ascending order. -/
def visitedUnits (mask : Mask) : List PowerModel.UnitId :=
  let units := carriedUnits mask
  let count := Admission.CardTable.population mask.val
  (if 1 ≤ count ∧ count ≤ 2 then units.take 1 else []) ++
    (if count = 2 then (units.drop 1).take 1 else []) ++
    (if 2 < count then units else [])

theorem visited_units_exact : ∀ mask : Mask, visitedUnits mask = carriedUnits mask := by decide

structure Input where
  mask : Mask
  attr : PowerModel.Attribute
  totals : PowerModel.UnitId → Fin 4 → Admission.GatherOrder.Signed32
  multi : Option (Fin 8 → Admission.GatherOrder.Signed32)

def detail (input : Input) (unit : PowerModel.UnitId) (key : Fin 4) : Int :=
  if input.mask.val.testBit unit.val then (input.totals unit key).value else 0

def legacyTotals (input : Input) : List Int :=
  (visitedUnits input.mask).flatMap (fun unit => List.ofFn (fun key => (input.totals unit key).value))

def summaryStep (state : Int × Int) (value : Int) : Int × Int :=
  (min state.1 value, max state.2 value)

def foldSummary (values : List Int) (initial : Int × Int) : Int × Int := values.foldl summaryStep initial

def legacySummary (input : Input) : Int × Int :=
  let state := foldSummary (legacyTotals input) (2147483647, -2147483648)
  if state.1 = 2147483647 then (0, 0) else state

def summary (input : Input) : Int × Int :=
  match input.multi with
  | none => legacySummary input
  | some values => foldSummary (List.ofFn (fun index => (values index).value)) (legacySummary input)

theorem fold_summary_extends (values : List Int) (initial : Int × Int) :
    (foldSummary values initial).1 ≤ initial.1 ∧ initial.2 ≤ (foldSummary values initial).2 := by
  induction values generalizing initial with
  | nil => exact ⟨le_rfl, le_rfl⟩
  | cons value rest ih =>
      have later := ih (summaryStep initial value)
      exact ⟨later.1.trans (min_le_left _ _), (le_max_left _ _).trans later.2⟩

theorem fold_summary_member (values : List Int) (initial : Int × Int) (value : Int)
    (member : value ∈ values) :
    (foldSummary values initial).1 ≤ value ∧ value ≤ (foldSummary values initial).2 := by
  induction values generalizing initial with
  | nil => simp at member
  | cons first rest ih =>
      rcases List.mem_cons.mp member with same | tail
      · subst value
        have later := fold_summary_extends rest (summaryStep initial first)
        exact ⟨later.1.trans (min_le_right _ _), (le_max_right _ _).trans later.2⟩
      · exact ih (summaryStep initial first) tail

theorem legacy_total_member (input : Input) (unit : PowerModel.UnitId) (key : Fin 4)
    (present : input.mask.val.testBit unit.val = true) :
    (input.totals unit key).value ∈ legacyTotals input := by
  unfold legacyTotals
  rw [visited_units_exact]
  apply List.mem_flatMap.mpr
  refine ⟨unit, ?_, ?_⟩
  · simp [carriedUnits, present]
  · exact List.mem_ofFn.mpr ⟨key, rfl⟩

theorem legacy_detail_upper (input : Input)
    (admitted : ∀ unit key, (detail input unit key).toNat ≤ 2 ^ 18 - 1)
    (unit : PowerModel.UnitId) (key : Fin 4) :
    (detail input unit key).toNat ≤ (legacySummary input).2.toNat := by
  by_cases present : input.mask.val.testBit unit.val = true
  · have member := legacy_total_member input unit key present
    have bounds := fold_summary_member (legacyTotals input) (2147483647, -2147483648)
      (input.totals unit key).value member
    have range := admitted unit key
    simp only [detail, present, ↓reduceIte] at range ⊢
    have below : (input.totals unit key).value < 2147483647 := by omega
    have notFallback : (foldSummary (legacyTotals input) (2147483647, -2147483648)).1 ≠ 2147483647 := by omega
    rw [legacySummary, if_neg notFallback]
    exact Int.toNat_le_toNat bounds.2
  · simp [detail, present]

theorem summary_legacy_upper (input : Input) : (legacySummary input).2 ≤ (summary input).2 := by
  cases multi : input.multi with
  | none => simp [summary, multi]
  | some values =>
      rw [summary, multi]
      exact (fold_summary_extends _ _).2

theorem detail_upper (input : Input)
    (admitted : ∀ unit key, (detail input unit key).toNat ≤ 2 ^ 18 - 1)
    (unit : PowerModel.UnitId) (key : Fin 4) :
    (detail input unit key).toNat ≤ (summary input).2.toNat :=
  (legacy_detail_upper input admitted unit key).trans (Int.toNat_le_toNat (summary_legacy_upper input))

theorem multi_upper (input : Input) (values : Fin 8 → Admission.GatherOrder.Signed32)
    (present : input.multi = some values) (index : Fin 8) :
    (values index).value.toNat ≤ (summary input).2.toNat := by
  rw [summary, present]
  exact Int.toNat_le_toNat (fold_summary_member _ _ _ (List.mem_ofFn.mpr ⟨index, rfl⟩)).2

def InRange (value : Int) : Prop := -2147483648 ≤ value ∧ value ≤ 2147483647

theorem fold_summary_range (values : List Int) (initial : Int × Int)
    (start : InRange initial.1 ∧ InRange initial.2)
    (bounded : ∀ value ∈ values, InRange value) :
    InRange (foldSummary values initial).1 ∧ InRange (foldSummary values initial).2 := by
  induction values generalizing initial with
  | nil => exact start
  | cons first rest ih =>
      apply ih (summaryStep initial first)
      · have bound := bounded first (by simp)
        simp only [summaryStep, InRange] at start bound ⊢
        omega
      · intro value member
        exact bounded value (List.mem_cons_of_mem _ member)

theorem legacy_summary_range (input : Input) :
    InRange (legacySummary input).1 ∧ InRange (legacySummary input).2 := by
  unfold legacySummary
  dsimp only
  split
  · norm_num [InRange]
  · apply fold_summary_range
    · norm_num [InRange]
    · intro value member
      obtain ⟨unit, _, belongs⟩ := List.mem_flatMap.mp member
      obtain ⟨key, same⟩ := List.mem_ofFn.mp belongs
      subst value
      exact ⟨(input.totals unit key).lower, (input.totals unit key).upper⟩

theorem summary_range (input : Input) : InRange (summary input).1 ∧ InRange (summary input).2 := by
  cases present : input.multi with
  | none => simpa only [summary, present] using legacy_summary_range input
  | some values =>
      rw [summary, present]
      apply fold_summary_range _ _ (legacy_summary_range input)
      intro value member
      obtain ⟨index, same⟩ := List.mem_ofFn.mp member
      subst value
      exact ⟨(values index).lower, (values index).upper⟩

/-- The metadata and skills are retained, while detail rows and the two
summary fields are all produced from the same power construction. -/
def attach (input : Input) (card : Admission.GatherOrder.Card) : Admission.GatherOrder.Card :=
  { card with
    core := { card.core with attr := input.attr.val, unitMask := input.mask.val, power := detail input }
    powerMin := ⟨(summary input).1, (summary_range input).1.1, (summary_range input).1.2⟩
    powerMax := ⟨(summary input).2, (summary_range input).2.1, (summary_range input).2.2⟩
    multiPower := input.multi }

def legacyValues (input : Input) (index : Fin 8) : Nat :=
  let unit := if index.val < 4 then Admission.PowerEncoding.primary input.mask else Admission.PowerEncoding.secondary input.mask
  (detail input unit ⟨index.val % 4, Nat.mod_lt _ (by decide)⟩).toNat

def data (input : Input) : MixedPower.Card :=
  MixedPower.fromRaw input.mask input.attr (Admission.PowerEncoding.profile input.mask)
    (legacyValues input) (input.multi.map (fun values index => (values index).value.toNat))

theorem data_legacy_upper (input : Input)
    (admitted : ∀ unit key, (detail input unit key).toNat ≤ 2 ^ 18 - 1) :
    PowerModel.tableMax (MixedPower.legacy (data input)) ≤ (summary input).2.toNat := by
  apply Finset.sup_le
  intro index _
  exact detail_upper input admitted _ _

theorem data_maximum_upper (input : Input)
    (admitted : ∀ unit key, (detail input unit key).toNat ≤ 2 ^ 18 - 1) :
    MixedPower.maximum (data input) ≤ (summary input).2.toNat := by
  have legacy := data_legacy_upper input admitted
  cases present : input.multi with
  | none => simpa only [data, MixedPower.fromRaw, MixedPower.maximum, present, Option.map_none] using legacy
  | some values =>
      have multi : Finset.univ.sup (fun index => (values index).value.toNat) ≤ (summary input).2.toNat := by
        apply Finset.sup_le
        intro index _
        exact multi_upper input values present index
      simpa only [data, MixedPower.fromRaw, MixedPower.maximum, present, Option.map_some] using max_le legacy multi

theorem attached_scenario_upper (input : Input) (card : Admission.GatherOrder.Card)
    (accepted : Admission.CardTable.Domain (attach input card).row)
    (mode : MixedPower.Mode) (common : Finset PowerModel.UnitId) (attr : Bool) :
    ScenarioPower.sourceBound mode (data input) common attr ≤ (attach input card).powerMax.value.toNat := by
  have admitted : ∀ unit key, (detail input unit key).toNat ≤ 2 ^ 18 - 1 := accepted.2.2.2.2.2.2.1
  exact (ScenarioPower.source_bound_le_maximum mode (data input) common attr).trans (data_maximum_upper input admitted)

theorem attached_values (input : Input) (card : Admission.GatherOrder.Card)
    (accepted : Admission.CardTable.Domain (attach input card).row) (index : Fin 8) :
    (Admission.PowerEncoding.rowValues (attach input card).row accepted index).val = legacyValues input index := rfl

def sourceCards (sources : List (Input × Admission.GatherOrder.Card)) : List Admission.GatherOrder.Card :=
  sources.map (fun source => attach source.1 source.2)

theorem built_origin (sources : List (Input × Admission.GatherOrder.Card))
    (context : Admission.SourceNumeric.Context) (pool : Admission.GatherPipeline.Pool)
    (accepted : Admission.SourceNumeric.build (sourceCards sources) context = .ok pool)
    (card : Admission.GatherOrder.Card) (member : card ∈ pool.cards) :
    ∃ source ∈ sources, attach source.1 source.2 = card := by
  have gathered := (Admission.SourceNumeric.concrete_success _ context pool accepted).1
  have sorted := (Admission.GatherPipeline.gather_success context.target context.hasEvent
    (context.fixedCards.map Fin.val) (context.fixedCharacters.map Fin.val) _ pool gathered).2.1
  rw [sorted] at member
  have original := (Admission.GatherOrder.sort_permutation context.target context.hasEvent
    (context.fixedCards.map Fin.val) (context.fixedCharacters.map Fin.val) (sourceCards sources)).mem_iff.mp member
  exact List.mem_map.mp original

def cardData (card : Admission.GatherOrder.Card) (valid : Admission.CardTable.Domain card.row) : MixedPower.Card :=
  let mask := Admission.PowerEncoding.rowMask card.row valid
  MixedPower.fromRaw mask ⟨card.row.attr, by have bound := valid.2.2.2.1; omega⟩
    (Admission.PowerEncoding.profile mask)
    (fun index => (Admission.PowerEncoding.rowValues card.row valid index).val)
    (card.multiPower.map (fun values index => (values index).value.toNat))

theorem attached_data (input : Input) (card : Admission.GatherOrder.Card)
    (valid : Admission.CardTable.Domain (attach input card).row) :
    cardData (attach input card) valid = data input := rfl

def poolData (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (index : Fin pool.cards.length) : MixedPower.Card :=
  cardData pool.cards[index] (Admission.GatherPipeline.row_domain pool valid index)

theorem built_scenario_upper (sources : List (Input × Admission.GatherOrder.Card))
    (context : Admission.SourceNumeric.Context) (pool : Admission.GatherPipeline.Pool)
    (accepted : Admission.SourceNumeric.build (sourceCards sources) context = .ok pool)
    (index : Fin pool.cards.length) (mode : MixedPower.Mode) (common : Finset PowerModel.UnitId) (attr : Bool) :
    let valid := (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1
    ScenarioPower.sourceBound mode (poolData pool valid index) common attr ≤ (pool.cards[index]).powerMax.value.toNat := by
  dsimp only
  let valid := (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1
  have domain := Admission.GatherPipeline.row_domain pool valid index
  obtain ⟨source, _, same⟩ := built_origin sources context pool accepted pool.cards[index] (List.getElem_mem index.isLt)
  change ScenarioPower.sourceBound mode (cardData pool.cards[index] domain) common attr ≤ _
  generalize cardEq : pool.cards[index] = card at domain ⊢
  rw [cardEq] at same
  subst card
  rw [attached_data]
  exact attached_scenario_upper source.1 source.2 domain mode common attr

theorem built_scenario_sum_limit (sources : List (Input × Admission.GatherOrder.Card))
    (context : Admission.SourceNumeric.Context) (pool : Admission.GatherPipeline.Pool)
    (accepted : Admission.SourceNumeric.build (sourceCards sources) context = .ok pool)
    (picks : List (Fin pool.cards.length)) (unique : picks.Nodup) (count : picks.length ≤ 5)
    (mode : MixedPower.Mode) (common : Finset PowerModel.UnitId) (attr : Bool) :
    let valid := (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1
    (picks.map (fun index => ScenarioPower.sourceBound mode (poolData pool valid index) common attr)).sum +
      context.honor.val ≤ 2 ^ 24 := by
  dsimp only
  have pointwise := List.sum_le_sum (fun index (_ : index ∈ picks) =>
    built_scenario_upper sources context pool accepted index mode common attr)
  have complete := Admission.SourceNumeric.concrete_success _ context pool accepted
  exact (Nat.add_le_add_right pointwise context.honor.val).trans
    (PowerAdmission.stored_picks_limit pool context complete.2.2.1 picks unique count)

theorem built_repeated_bound_u32 (sources : List (Input × Admission.GatherOrder.Card))
    (context : Admission.SourceNumeric.Context) (pool : Admission.GatherPipeline.Pool)
    (accepted : Admission.SourceNumeric.build (sourceCards sources) context = .ok pool)
    (picks : List (Fin pool.cards.length)) (unique : picks.Nodup) (count : picks.length ≤ 5)
    (index : Fin pool.cards.length) (slots : Nat) (remaining : slots ≤ 5)
    (mode : MixedPower.Mode) (common : Finset PowerModel.UnitId) (attr : Bool) :
    let valid := (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1
    (picks.map (fun item => ScenarioPower.sourceBound mode (poolData pool valid item) common attr)).sum +
      ScenarioPower.sourceBound mode (poolData pool valid index) common attr * slots ≤ Saturating.u32Max := by
  dsimp only
  have selected := built_scenario_sum_limit sources context pool accepted picks unique count mode common attr
  have single := built_scenario_sum_limit sources context pool accepted [index] (by simp) (by simp) mode common attr
  simp only [List.map_cons, List.map_nil, List.sum_cons, List.sum_nil, Nat.add_zero] at single
  have product := Nat.mul_le_mul_right slots (Nat.le_of_add_right_le single)
  have slotBound := Nat.mul_le_mul_left (2 ^ 24) remaining
  norm_num [Saturating.u32Max] at *
  omega

end PowerProduction

namespace SceneTable

def poolMask (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (index : Fin pool.cards.length) : Mask :=
  Admission.PowerEncoding.rowMask pool.cards[index].row (Admission.GatherPipeline.row_domain pool valid index)

def poolMasks (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool) : List Mask :=
  (List.finRange pool.cards.length).map (poolMask pool valid)

theorem pool_data_units (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (index : Fin pool.cards.length) :
    (MixedPower.legacy (PowerProduction.poolData pool valid index)).units =
      MixedPower.rawMembership (poolMask pool valid index) := by
  have domain := Admission.GatherPipeline.row_domain pool valid index
  exact MixedPower.from_raw_units _ _ _ _ _ domain.2.2.2.2.2.1

theorem deck_mask_membership (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (picks : List (Fin pool.cards.length)) :
    MixedPower.rawMembership (commonMask (picks.map (poolMask pool valid))) =
      PowerModel.commonUnits (fun index => MixedPower.legacy (PowerProduction.poolData pool valid index)) picks := by
  rw [common_mask_membership]
  induction picks with
  | nil => rfl
  | cons first rest ih =>
      simp only [List.map_cons, List.foldr_cons, PowerModel.commonUnits, pool_data_units, ih]

theorem deck_mask_covered (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (picks : List (Fin pool.cards.length)) (nonempty : picks ≠ []) :
    commonMask (picks.map (poolMask pool valid)) ∈ unitSets (poolMasks pool valid) := by
  apply common_mask_covered
  · cases picks <;> simp_all
  · intro mask member
    obtain ⟨index, _, same⟩ := List.mem_map.mp member
    exact List.mem_map.mpr ⟨index, List.mem_finRange index, same⟩

structure Key where
  units : Mask
  attributeSlot : Fin 7
  deriving DecidableEq

def keys (masks : List Mask) : List Key :=
  (unitSets masks).flatMap (fun units => (List.finRange 7).map (fun slot => ⟨units, slot⟩))

def sharedAttribute (key : Key) : Bool := decide (key.attributeSlot.val ≠ 0)

def allows (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (key : Key) (index : Fin pool.cards.length) : Bool :=
  decide (intersection key.units (poolMask pool valid index) = key.units) &&
    decide (key.attributeSlot.val = 0 ∨ pool.cards[index].row.attr + 1 = key.attributeSlot.val)

def entry (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (key : Key) (index : Fin pool.cards.length) : Entry :=
  let domain := Admission.GatherPipeline.row_domain pool valid index
  { power := ScenarioPower.sourceBound mode (PowerProduction.poolData pool valid index)
      (MixedPower.rawMembership key.units) (sharedAttribute key)
    dense := index.val
    publicId := ⟨pool.cards[index].row.publicId.toNat, by have lower := domain.1; have upper := domain.2.1; omega⟩
    character := ⟨pool.cards[index].row.character, by have bound := domain.2.2.1; omega⟩ }

def rawEntries (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (key : Key) : List Entry :=
  ((List.finRange pool.cards.length).filter (allows pool valid key)).map (entry pool valid mode key)

structure Scenario where
  key : Key
  entries : List Entry
  ceiling : Nat
  order : Nat

def buildOne (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (key : Key) (order : Nat) : Option Scenario :=
  let entries := sortedEntries (rawEntries pool valid mode key)
  (bestCompletion entries 0 ∅ 5).map (fun result => ⟨key, entries, result.1, order⟩)

def build (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) : List Scenario :=
  ((keys (poolMasks pool valid)).zipIdx).filterMap (fun pair => buildOne pool valid mode pair.1 pair.2)

theorem allowed_entry (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (key : Key) (index : Fin pool.cards.length)
    (allowed : allows pool valid key index = true) :
    entry pool valid mode key index ∈ rawEntries pool valid mode key := by
  apply List.mem_map.mpr
  exact ⟨index, List.mem_filter.mpr ⟨List.mem_finRange index, allowed⟩, rfl⟩

theorem viable_scene (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (key : Key) (order : Nat) (picks : List (Fin pool.cards.length))
    (size : picks.length = 5)
    (unique : ((picks.map (entry pool valid mode key)).map Entry.character).Nodup)
    (allowed : ∀ index ∈ picks, allows pool valid key index = true) :
    ∃ scenario, buildOne pool valid mode key order = some scenario ∧
      scenario.key = key ∧ scenario.order = order ∧
      ((picks.map (entry pool valid mode key)).map Entry.power).sum ≤ scenario.ceiling := by
  have available : ∀ item ∈ picks.map (entry pool valid mode key),
      item ∈ (sortedEntries (rawEntries pool valid mode key)).drop 0 ∧ item.character ∉ (∅ : Finset (Fin 27)) := by
    intro item member
    obtain ⟨index, belongs, rfl⟩ := List.mem_map.mp member
    constructor
    · simpa only [List.drop_zero] using
        (List.mergeSort_perm _ _).mem_iff.mpr (allowed_entry pool valid mode key index (allowed index belongs))
    · simp
  obtain ⟨sum, least, found, bound, _⟩ := best_completion_spec (rawEntries pool valid mode key)
    (picks.map (entry pool valid mode key)) 0 ∅ 5 (by simpa using size) unique available
  refine ⟨⟨key, sortedEntries (rawEntries pool valid mode key), sum, order⟩, ?_, rfl, rfl, bound⟩
  simp only [buildOne, found, Option.map_some]

theorem key_member (masks : List Mask) (units : Mask) (slot : Fin 7)
    (member : units ∈ unitSets masks) : (⟨units, slot⟩ : Key) ∈ keys masks :=
  List.mem_flatMap.mpr ⟨units, member, List.mem_map.mpr ⟨slot, List.mem_finRange slot, rfl⟩⟩

theorem required_key (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (picks : List (Fin pool.cards.length)) (nonempty : picks ≠ []) :
    ∃ key ∈ keys (poolMasks pool valid),
      MixedPower.rawMembership key.units =
        PowerModel.commonUnits (fun index => MixedPower.legacy (PowerProduction.poolData pool valid index)) picks ∧
      sharedAttribute key =
        PowerModel.sharesAttribute (fun index => MixedPower.legacy (PowerProduction.poolData pool valid index)) picks ∧
      ∀ index ∈ picks, allows pool valid key index = true := by
  classical
  let mask := commonMask (picks.map (poolMask pool valid))
  have covered : mask ∈ unitSets (poolMasks pool valid) := deck_mask_covered pool valid picks nonempty
  have maskExact := deck_mask_membership pool valid picks
  have contained : ∀ index ∈ picks, intersection mask (poolMask pool valid index) = mask := by
    intro index member
    apply (intersection_subset_exact _ _).mpr
    rw [deck_mask_membership]
    intro unit common
    have carried := (PowerModel.mem_commonUnits _ picks unit).mp common index member
    simpa only [pool_data_units] using carried
  by_cases uniform : ∃ attrValue, PowerModel.UniformAt
      (fun index => MixedPower.legacy (PowerProduction.poolData pool valid index)) picks attrValue
  · obtain ⟨attrValue, uniformAt⟩ := uniform
    let slot : Fin 7 := ⟨attrValue.val + 1, by have bound := attrValue.isLt; omega⟩
    refine ⟨⟨mask, slot⟩, key_member _ _ _ covered, maskExact, ?_, ?_⟩
    · have shared := (PowerModel.sharesAttribute_eq_true _ _).mpr ⟨attrValue, uniformAt⟩
      rw [shared]
      simp [sharedAttribute, slot]
    · intro index member
      have equal : pool.cards[index].row.attr = attrValue.val := congrArg Fin.val (uniformAt index member)
      simp only [allows, contained index member, decide_true, Bool.true_and, decide_eq_true_eq]
      exact Or.inr (congrArg (fun value => value + 1) equal)
  · refine ⟨⟨mask, 0⟩, key_member _ _ _ covered, maskExact, ?_, ?_⟩
    · have shared : PowerModel.sharesAttribute
          (fun index => MixedPower.legacy (PowerProduction.poolData pool valid index)) picks = false :=
        Bool.eq_false_iff.mpr (fun truth => uniform ((PowerModel.sharesAttribute_eq_true _ _).mp truth))
      rw [shared]
      rfl
    · intro index member
      simp [allows, contained index member]

theorem required_key_power (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (picks : List (Fin pool.cards.length)) (full : picks.length = 5) :
    ∃ key ∈ keys (poolMasks pool valid), (∀ index ∈ picks, allows pool valid key index = true) ∧
      ∀ index, MixedPower.cardPower mode (PowerProduction.poolData pool valid) picks index ≤
        (entry pool valid mode key index).power := by
  obtain ⟨key, member, units, attrExact, allowed⟩ := required_key pool valid picks
    (by intro empty; simp [empty] at full)
  refine ⟨key, member, allowed, ?_⟩
  intro index
  change MixedPower.cardPower mode (PowerProduction.poolData pool valid) picks index ≤
    ScenarioPower.sourceBound mode (PowerProduction.poolData pool valid index)
      (MixedPower.rawMembership key.units) (sharedAttribute key)
  rw [units, attrExact]
  exact ScenarioPower.source_full_deck_bound mode (PowerProduction.poolData pool valid) picks full index

theorem build_covers_deck (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (picks : List (Fin pool.cards.length)) (full : picks.length = 5)
    (unique : (picks.map (fun index => pool.cards[index].row.character)).Nodup) :
    ∃ scenario ∈ build pool valid mode,
      (∀ index ∈ picks, allows pool valid scenario.key index = true) ∧
      MixedPower.total mode (PowerProduction.poolData pool valid) picks ≤ scenario.ceiling := by
  obtain ⟨key, keyMember, allowed, bounded⟩ := required_key_power pool valid mode picks full
  have positions : ∃ order, (key, order) ∈ (keys (poolMasks pool valid)).zipIdx := by
    have mapped : key ∈ ((keys (poolMasks pool valid)).zipIdx).map Prod.fst := by
      simpa only [List.zipIdx_map_fst] using keyMember
    obtain ⟨⟨found, order⟩, member, same⟩ := List.mem_map.mp mapped
    exact ⟨order, same ▸ member⟩
  obtain ⟨order, position⟩ := positions
  have distinct : ((picks.map (entry pool valid mode key)).map Entry.character).Nodup := by
    apply List.Nodup.of_map (f := fun character : Fin 27 => character.val)
    simpa only [List.map_map, entry, Function.comp_def] using unique
  obtain ⟨scenario, found, sameKey, _, ceiling⟩ := viable_scene pool valid mode key order picks full distinct allowed
  refine ⟨scenario, ?_, ?_, ?_⟩
  · exact List.mem_filterMap.mpr ⟨(key, order), position, found⟩
  · simpa only [sameKey] using allowed
  · have total := List.sum_le_sum (fun index (_ : index ∈ picks) => bounded index)
    have upper : (picks.map (fun index => (entry pool valid mode key index).power)).sum ≤ scenario.ceiling := by
      simpa only [List.map_map, Function.comp_def] using ceiling
    exact total.trans upper

def decode (pool : Admission.GatherPipeline.Pool) (positive : 0 < pool.cards.length) (item : Entry) : Fin pool.cards.length :=
  ⟨item.dense % pool.cards.length, Nat.mod_lt _ positive⟩

theorem decode_entry (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (mode : MixedPower.Mode) (key : Key) (index : Fin pool.cards.length) :
    decode pool positive (entry pool valid mode key index) = index := by
  apply Fin.ext
  exact Nat.mod_eq_of_lt index.isLt

theorem raw_decoded_entry (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (mode : MixedPower.Mode) (key : Key) (item : Entry)
    (member : item ∈ rawEntries pool valid mode key) :
    entry pool valid mode key (decode pool positive item) = item := by
  obtain ⟨index, _, same⟩ := List.mem_map.mp member
  rw [← same, decode_entry]

theorem decoded_unique (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (mode : MixedPower.Mode) (key : Key) (items : List Entry)
    (members : ∀ item ∈ items, item ∈ rawEntries pool valid mode key)
    (unique : (items.map Entry.character).Nodup) : (items.map (decode pool positive)).Nodup := by
  have mapped : (items.map (decode pool positive)).map (fun index => (entry pool valid mode key index).character) =
      items.map Entry.character := by
    rw [List.map_map]
    apply List.map_congr_left
    intro item member
    exact congrArg Entry.character (raw_decoded_entry pool valid positive mode key item (members item member))
  apply List.Nodup.of_map (fun index => (entry pool valid mode key index).character)
  rwa [mapped]

theorem decoded_power_sum (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (mode : MixedPower.Mode) (key : Key) (items : List Entry)
    (members : ∀ item ∈ items, item ∈ rawEntries pool valid mode key) :
    ((items.map (decode pool positive)).map (fun index =>
      ScenarioPower.sourceBound mode (PowerProduction.poolData pool valid index)
        (MixedPower.rawMembership key.units) (sharedAttribute key))).sum = (items.map Entry.power).sum := by
  rw [List.map_map]
  congr 1
  apply List.map_congr_left
  intro item member
  exact congrArg Entry.power (raw_decoded_entry pool valid positive mode key item (members item member))

theorem built_entries_sum_limit (sources : List (PowerProduction.Input × Admission.GatherOrder.Card))
    (context : Admission.SourceNumeric.Context) (pool : Admission.GatherPipeline.Pool)
    (accepted : Admission.SourceNumeric.build (PowerProduction.sourceCards sources) context = .ok pool)
    (positive : 0 < pool.cards.length) (mode : MixedPower.Mode) (key : Key) (items : List Entry)
    (unique : (items.map Entry.character).Nodup) (count : items.length ≤ 5)
    (members : ∀ item ∈ items, item ∈ rawEntries pool
      (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1 mode key) :
    (items.map Entry.power).sum + context.honor.val ≤ 2 ^ 24 := by
  let valid := (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1
  have distinct := decoded_unique pool valid positive mode key items members unique
  have bound := PowerProduction.built_scenario_sum_limit sources context pool accepted
    (items.map (decode pool positive)) distinct (by simpa only [List.length_map] using count)
    mode (MixedPower.rawMembership key.units) (sharedAttribute key)
  dsimp only at bound
  rw [decoded_power_sum pool valid positive mode key items members] at bound
  exact bound

end SceneTable

namespace SmallIds

abbrev PublicId := Fin 65536

/-- Only addresses below five are observable; the occupied length, not an ID
value, distinguishes padding from data. -/
structure State where
  storage : Nat → PublicId
  length : Fin 6

def initial : State := ⟨fun _ => 0, 0⟩

def occupied (state : State) : List PublicId := (List.range state.length.val).map state.storage

def shiftStep (storage : Nat → PublicId) (index : Nat) : Nat → PublicId :=
  Function.update storage index (storage (index - 1))

def shiftIndices (position length : Nat) : List Nat := ((List.range length).drop (position + 1)).reverse

def insertAt (state : State) (position : Nat) (id : PublicId) : State :=
  let length := min (state.length.val + 1) 5
  let shifted := (shiftIndices position length).foldl shiftStep state.storage
  ⟨Function.update shifted position id, ⟨length, by omega⟩⟩

def insert (state : State) (id : PublicId) : State :=
  let position := ((occupied state).takeWhile (fun old => decide (old < id))).length
  if (occupied state)[position]? = some id ∨ position = 5 then state else insertAt state position id

def collect (ids : List PublicId) : State := ids.foldl insert initial

theorem shift_index_range (position length index : Nat) (member : index ∈ shiftIndices position length) :
    position < index ∧ index < length := by
  have belongs := List.mem_reverse.mp member
  have rangeMember := List.mem_of_mem_drop belongs
  have high := List.mem_range.mp rangeMember
  have lower : position + 1 ≤ index := by
    have value := List.getElem_of_mem belongs
    obtain ⟨offset, bound, same⟩ := value
    simp only [List.getElem_drop, List.getElem_range] at same
    omega
  omega

theorem shifts_untouched (indices : List Nat) (storage : Nat → PublicId) (address : Nat)
    (absent : address ∉ indices) : (indices.foldl shiftStep storage) address = storage address := by
  induction indices generalizing storage with
  | nil => rfl
  | cons first rest ih =>
      rw [List.foldl_cons, ih (shiftStep storage first) (fun member => absent (List.mem_cons_of_mem _ member))]
      exact Function.update_of_ne (fun same => absent (List.mem_cons.mpr (Or.inl same))) _ _

theorem shifts_spec (indices : List Nat) (storage : Nat → PublicId)
    (ordered : indices.Pairwise (fun left right => right < left)) (address : Nat) :
    (indices.foldl shiftStep storage) address =
      if address ∈ indices then storage (address - 1) else storage address := by
  induction indices generalizing storage with
  | nil => simp
  | cons first rest ih =>
      have ordering := List.pairwise_cons.mp ordered
      by_cases same : address = first
      · subst address
        have absent : first ∉ rest := fun member => Nat.lt_irrefl first (ordering.1 first member)
        rw [List.foldl_cons, shifts_untouched rest _ first absent]
        simp [shiftStep]
      · rw [List.foldl_cons, ih _ ordering.2]
        by_cases member : address ∈ rest
        · have smaller := ordering.1 address member
          have previous : address - 1 ≠ first := by omega
          simp only [member, if_pos, shiftStep, Function.update_of_ne previous, List.mem_cons, same, false_or]
        · simp [member, same, shiftStep]

theorem shift_indices_descending (position length : Nat) :
    (shiftIndices position length).Pairwise (fun left right => right < left) := by
  have ascending : ((List.range length).drop (position + 1)).Pairwise (· < ·) := List.pairwise_lt_range.drop
  simpa only [shiftIndices, List.pairwise_reverse] using ascending

theorem shifted_read (state : State) (position length address : Nat) :
    ((shiftIndices position length).foldl shiftStep state.storage) address =
      if address ∈ shiftIndices position length then state.storage (address - 1) else state.storage address :=
  shifts_spec _ state.storage (shift_indices_descending position length) address

theorem shift_index_iff (position length address : Nat) :
    address ∈ shiftIndices position length ↔ position < address ∧ address < length := by
  constructor
  · exact shift_index_range position length address
  · rintro ⟨afterPosition, beforeEnd⟩
    have bound : address - (position + 1) < ((List.range length).drop (position + 1)).length := by
      simp only [List.length_drop, List.length_range]
      omega
    have member := List.getElem_mem bound
    have same : ((List.range length).drop (position + 1))[address - (position + 1)] = address := by
      simp only [List.getElem_drop, List.getElem_range]
      omega
    rw [same] at member
    exact List.mem_reverse.mpr member

theorem insert_read (state : State) (id : PublicId) (address : Nat)
    (fresh : (occupied state)[((occupied state).takeWhile (fun old => decide (old < id))).length]? ≠ some id)
    (room : ((occupied state).takeWhile (fun old => decide (old < id))).length ≠ 5) :
    let position := ((occupied state).takeWhile (fun old => decide (old < id))).length
    let length := min (state.length.val + 1) 5
    (insert state id).storage address =
      if address = position then id else
        if position < address ∧ address < length then state.storage (address - 1) else state.storage address := by
  dsimp only
  unfold insert insertAt
  dsimp only
  rw [if_neg (not_or.mpr ⟨fresh, room⟩)]
  dsimp only
  by_cases same : address = ((occupied state).takeWhile (fun old => decide (old < id))).length
  · subst address
    simp
  · rw [Function.update_of_ne same, if_neg same, shifted_read]
    simp only [shift_index_iff]

theorem insert_at_projection (state : State) (position : Nat) (id : PublicId)
    (within : position ≤ state.length.val) :
    occupied (insertAt state position id) =
      ((occupied state).take position ++ id :: (occupied state).drop position).take 5 := by
  rcases state with ⟨storage, length⟩
  have bound := length.isLt
  fin_cases length <;> dsimp only at within
  all_goals interval_cases position
  all_goals simp [insertAt, occupied, shiftIndices, shiftStep, List.range_succ, List.foldl_cons, Function.update]

theorem ordered_insert_at_cut (ids : List PublicId) (id : PublicId) :
    ids.orderedInsert (· ≤ ·) id =
      ids.take ((ids.takeWhile (fun old => decide (old < id))).length) ++
        id :: ids.drop ((ids.takeWhile (fun old => decide (old < id))).length) := by
  induction ids with
  | nil => simp
  | cons first rest ih =>
      by_cases less : first < id
      · simp [List.orderedInsert, less, not_le.mpr less, ih]
      · have before : id ≤ first := le_of_not_gt less
        simp [List.orderedInsert, less, before]

theorem duplicate_at_cut (ids : List PublicId) (id : PublicId) (ordered : ids.Pairwise (· ≤ ·)) :
    ids[(ids.takeWhile (fun old => decide (old < id))).length]? = some id ↔ id ∈ ids := by
  constructor
  · exact List.mem_of_getElem?
  · intro member
    induction ids with
    | nil => simp at member
    | cons first rest ih =>
        have ordering := List.pairwise_cons.mp ordered
        rcases List.mem_cons.mp member with same | tail
        · subst id
          simp
        · have before := ordering.1 id tail
          rcases lt_or_eq_of_le before with less | same
          · simpa [less] using ih ordering.2 tail
          · subst id
            simp

def insertList (ids : List PublicId) (id : PublicId) : List PublicId :=
  if id ∈ ids then ids else ids.orderedInsert (· ≤ ·) id

theorem occupied_length (state : State) : (occupied state).length = state.length.val := by
  simp [occupied]

theorem insert_projection (state : State) (id : PublicId)
    (ordered : (occupied state).Pairwise (· ≤ ·)) :
    occupied (insert state id) = (insertList (occupied state) id).take 5 := by
  let position := ((occupied state).takeWhile (fun old => decide (old < id))).length
  have within : position ≤ state.length.val := by
    have part : ((occupied state).takeWhile (fun old => decide (old < id))).Sublist (occupied state) := List.takeWhile_sublist _
    simpa only [occupied_length] using part.length_le
  have lengthBound : (occupied state).length ≤ 5 := by rw [occupied_length]; exact Nat.le_of_lt_succ state.length.isLt
  by_cases present : id ∈ occupied state
  · have duplicate := (duplicate_at_cut (occupied state) id ordered).mpr present
    simp only [insert, duplicate, true_or, ↓reduceIte, insertList, if_pos present, List.take_of_length_le lengthBound]
  · have fresh := mt (duplicate_at_cut (occupied state) id ordered).mp present
    by_cases full : position = 5
    · have length : (occupied state).length = 5 := by rw [occupied_length]; omega
      have selected : ((occupied state).takeWhile (fun old => decide (old < id))).length = 5 := full
      rw [insert, if_pos (Or.inr selected), insertList, if_neg present, ordered_insert_at_cut, selected]
      simp [length]
    · rw [insert, if_neg (not_or.mpr ⟨fresh, full⟩), insert_at_projection state position id within,
        insertList, if_neg present, ordered_insert_at_cut]

theorem insert_length (state : State) (id : PublicId) : (insert state id).length.val ≤ 5 :=
  Nat.le_of_lt_succ (insert state id).length.isLt

theorem insert_list_ordered (ids : List PublicId) (id : PublicId) (ordered : ids.Pairwise (· ≤ ·)) :
    (insertList ids id).Pairwise (· ≤ ·) := by
  unfold insertList
  split
  · exact ordered
  · exact List.Sorted.orderedInsert id ids ordered

theorem insert_list_unique (ids : List PublicId) (id : PublicId) (unique : ids.Nodup) :
    (insertList ids id).Nodup := by
  unfold insertList
  split
  · exact unique
  · rename_i absent
    exact (List.perm_orderedInsert (· ≤ ·) id ids).nodup_iff.mpr (List.nodup_cons.mpr ⟨absent, unique⟩)

theorem insert_list_membership (ids : List PublicId) (id value : PublicId) :
    value ∈ insertList ids id ↔ value = id ∨ value ∈ ids := by
  by_cases present : id ∈ ids
  · rw [insertList, if_pos present]
    constructor
    · exact Or.inr
    · rintro (rfl | member)
      · exact present
      · exact member
  · simp [insertList, present]

theorem insert_list_cons (first id : PublicId) (rest : List PublicId)
    (ordered : (first :: rest).Pairwise (· ≤ ·)) :
    insertList (first :: rest) id =
      if first < id then first :: insertList rest id else
        if first = id then first :: rest else id :: first :: rest := by
  by_cases less : first < id
  · have different : id ≠ first := ne_of_gt less
    simp [insertList, less, different, List.orderedInsert, not_le.mpr less]
    split <;> rfl
  · by_cases same : first = id
    · subst id
      simp [insertList]
    · have smaller : id < first := lt_of_le_of_ne (le_of_not_gt less) (Ne.symm same)
      have absent : id ∉ rest := by
        intro member
        have bound := (List.pairwise_cons.mp ordered).1 id member
        exact (not_le_of_gt smaller) bound
      have different : id ≠ first := Ne.symm same
      simp [insertList, less, same, different, absent, List.orderedInsert, le_of_lt smaller]

theorem insert_take (ids : List PublicId) (id : PublicId) (count : Nat)
    (ordered : ids.Pairwise (· ≤ ·)) :
    (insertList (ids.take count) id).take count = (insertList ids id).take count := by
  induction ids generalizing count with
  | nil => simp
  | cons first rest ih =>
      cases count with
      | zero => simp
      | succ count =>
          have ordering := List.pairwise_cons.mp ordered
          have shorter : (first :: rest.take count).Pairwise (· ≤ ·) := by
            simpa only [List.take_succ_cons] using (ordered.take (i := count + 1))
          rw [List.take_succ_cons, insert_list_cons first id _ shorter, insert_list_cons first id _ ordered]
          by_cases less : first < id
          · simp [less, ih count ordering.2]
          · by_cases same : first = id
            · simp [same]
            · cases count with
              | zero => simp [less, same]
              | succ count => simp [less, same, List.take_take]

def sortedDistinct (ids : List PublicId) : List PublicId := ids.foldl insertList []

theorem distinct_fold_spec (ids initialIds : List PublicId)
    (ordered : initialIds.Pairwise (· ≤ ·)) (unique : initialIds.Nodup) :
    (ids.foldl insertList initialIds).Pairwise (· ≤ ·) ∧
      (ids.foldl insertList initialIds).Nodup ∧
      ∀ value, value ∈ ids.foldl insertList initialIds ↔ value ∈ initialIds ∨ value ∈ ids := by
  induction ids generalizing initialIds with
  | nil => exact ⟨ordered, unique, by simp⟩
  | cons first rest ih =>
      have later := ih (insertList initialIds first) (insert_list_ordered _ _ ordered) (insert_list_unique _ _ unique)
      refine ⟨later.1, later.2.1, ?_⟩
      intro value
      simp only [List.foldl_cons, later.2.2 value, insert_list_membership, List.mem_cons]
      tauto

theorem sorted_distinct_spec (ids : List PublicId) :
    (sortedDistinct ids).Pairwise (· ≤ ·) ∧ (sortedDistinct ids).Nodup ∧
      ∀ value, value ∈ sortedDistinct ids ↔ value ∈ ids := by
  have spec := distinct_fold_spec ids [] (by simp) (by simp)
  simpa only [sortedDistinct, List.not_mem_nil, false_or] using spec

theorem collect_fold_projection (ids : List PublicId) (state : State) (full : List PublicId)
    (ordered : full.Pairwise (· ≤ ·)) (projection : occupied state = full.take 5) :
    occupied (ids.foldl insert state) = (ids.foldl insertList full).take 5 := by
  induction ids generalizing state full with
  | nil => exact projection
  | cons first rest ih =>
      apply ih (insert state first) (insertList full first) (insert_list_ordered _ _ ordered)
      have current : (occupied state).Pairwise (· ≤ ·) := by rw [projection]; exact ordered.take
      rw [insert_projection state first current, projection, insert_take full first 5 ordered]

theorem collect_projection (ids : List PublicId) : occupied (collect ids) = (sortedDistinct ids).take 5 :=
  collect_fold_projection ids initial [] (by simp) rfl

theorem collect_ordered_unique (ids : List PublicId) :
    (occupied (collect ids)).Pairwise (· ≤ ·) ∧ (occupied (collect ids)).Nodup := by
  rw [collect_projection]
  have spec := sorted_distinct_spec ids
  exact ⟨spec.1.take, spec.2.1.take⟩

theorem sorted_prefix_lower (ids picks : List PublicId) (ordered : ids.Pairwise (· ≤ ·))
    (unique : picks.Nodup) (members : ∀ value ∈ picks, value ∈ ids) : ids.take picks.length ≤ picks := by
  induction ids generalizing picks with
  | nil =>
      cases picks with
      | nil => exact le_rfl
      | cons first rest => exact False.elim (by simpa using members first (by simp))
  | cons first rest ih =>
      cases picks with
      | nil => exact le_rfl
      | cons chosen tail =>
          have ordering := List.pairwise_cons.mp ordered
          have distinct := List.nodup_cons.mp unique
          have headMember := members chosen (by simp)
          have headBound : first ≤ chosen := by
            rcases List.mem_cons.mp headMember with same | later
            · simp [same]
            · exact ordering.1 chosen later
          rw [List.length_cons, List.take_succ_cons]
          by_cases same : first = chosen
          · subst chosen
            apply List.cons_le_cons first
            apply ih tail ordering.2 distinct.2
            intro value member
            rcases List.mem_cons.mp (members value (List.mem_cons_of_mem _ member)) with equal | later
            · exact False.elim (distinct.1 (equal ▸ member))
            · exact later
          · exact le_of_lt (List.Lex.rel (lt_of_le_of_ne headBound same))

theorem collected_prefix_lower (ids picks : List PublicId) (unique : picks.Nodup)
    (count : picks.length ≤ 5) (members : ∀ value ∈ picks, value ∈ ids) :
    (occupied (collect ids)).take picks.length ≤ picks := by
  have spec := sorted_distinct_spec ids
  have bound := sorted_prefix_lower (sortedDistinct ids) picks spec.1 unique
    (fun value member => (spec.2.2 value).mpr (members value member))
  simpa only [collect_projection, List.take_take, Nat.min_eq_left count] using bound

theorem collected_sufficient (ids picks : List PublicId) (unique : picks.Nodup)
    (count : picks.length ≤ 5) (members : ∀ value ∈ picks, value ∈ ids) :
    picks.length ≤ (occupied (collect ids)).length := by
  have spec := sorted_distinct_spec ids
  have included : picks.toFinset ⊆ (sortedDistinct ids).toFinset := by
    intro value member
    exact List.mem_toFinset.mpr ((spec.2.2 value).mpr (members value (List.mem_toFinset.mp member)))
  have cardBound := Finset.card_le_card included
  have lengthBound : picks.length ≤ (sortedDistinct ids).length := by
    simpa only [List.toFinset_card_of_nodup unique, List.toFinset_card_of_nodup spec.2.1] using cardBound
  rw [collect_projection, List.length_take]
  exact le_min count lengthBound

theorem ordered_insert_lex_lt (left right : List PublicId) (id : PublicId)
    (sameLength : left.length = right.length) (smaller : left < right) :
    left.orderedInsert (· ≤ ·) id < right.orderedInsert (· ≤ ·) id := by
  induction left generalizing right with
  | nil =>
      have empty : right = [] := List.length_eq_zero_iff.mp sameLength.symm
      subst right
      exact False.elim (lt_irrefl _ smaller)
  | cons first rest ih =>
      cases right with
      | nil => simp at sameLength
      | cons second tail =>
          change List.Lex (· < ·) (first :: rest) (second :: tail) at smaller
          change List.Lex (· < ·) _ _
          cases smaller with
          | rel headLess =>
              by_cases lowFirst : id ≤ first
              · have lowSecond := lowFirst.trans headLess.le
                simp only [List.orderedInsert, if_pos lowFirst, if_pos lowSecond]
                exact List.Lex.cons (List.Lex.rel headLess)
              · by_cases lowSecond : id ≤ second
                · simp only [List.orderedInsert, if_neg lowFirst, if_pos lowSecond]
                  exact List.Lex.rel (lt_of_not_ge lowFirst)
                · simp only [List.orderedInsert, if_neg lowFirst, if_neg lowSecond]
                  exact List.Lex.rel headLess
          | cons tailLess =>
              by_cases low : id ≤ first
              · simp only [List.orderedInsert, if_pos low]
                exact List.Lex.cons (List.Lex.cons tailLess)
              · simp only [List.orderedInsert, if_neg low]
                exact List.Lex.cons (ih tail (by simpa using sameLength) tailLess)

theorem ordered_insert_lex_le (left right : List PublicId) (id : PublicId)
    (sameLength : left.length = right.length) (smaller : left ≤ right) :
    left.orderedInsert (· ≤ ·) id ≤ right.orderedInsert (· ≤ ·) id := by
  rcases lt_or_eq_of_le smaller with less | same
  · exact (ordered_insert_lex_lt left right id sameLength less).le
  · rw [same]

def mergeSelected (selected tail : List PublicId) : List PublicId :=
  selected.foldr (List.orderedInsert (· ≤ ·)) tail

theorem merge_selected_length (selected tail : List PublicId) :
    (mergeSelected selected tail).length = selected.length + tail.length := by
  induction selected with
  | nil => simp [mergeSelected]
  | cons first rest ih =>
      simp only [mergeSelected, List.foldr_cons, List.orderedInsert_length, List.length_cons] at ih ⊢
      omega

theorem merge_selected_mono (selected left right : List PublicId)
    (sameLength : left.length = right.length) (smaller : left ≤ right) :
    mergeSelected selected left ≤ mergeSelected selected right := by
  induction selected with
  | nil => exact smaller
  | cons first rest ih =>
      apply ordered_insert_lex_le _ _ first
      · change (mergeSelected rest left).length = (mergeSelected rest right).length
        simp only [merge_selected_length, sameLength]
      · exact ih

def sortIds (ids : List PublicId) : List PublicId := ids.mergeSort (fun a b => decide (a ≤ b))

theorem sort_ids_ordered (ids : List PublicId) : (sortIds ids).Pairwise (· ≤ ·) := by
  simpa only [sortIds, decide_eq_true_eq] using
    List.sorted_mergeSort (le := fun a b : PublicId => decide (a ≤ b))
      (by intro a b c ab bc; simp only [decide_eq_true_eq] at ab bc ⊢; exact ab.trans bc)
      (by intro a b; simpa using le_total a b) ids

theorem sort_ids_perm (ids : List PublicId) : (sortIds ids).Perm ids := List.mergeSort_perm _ _

theorem merge_selected_ordered (selected tail : List PublicId) (ordered : tail.Pairwise (· ≤ ·)) :
    (mergeSelected selected tail).Pairwise (· ≤ ·) := by
  induction selected with
  | nil => exact ordered
  | cons first rest ih => exact List.Sorted.orderedInsert first _ ih

theorem merge_selected_perm (selected tail : List PublicId) :
    (mergeSelected selected tail).Perm (selected ++ tail) := by
  induction selected with
  | nil => exact List.Perm.refl _
  | cons first rest ih => exact (List.perm_orderedInsert _ first _).trans (ih.cons first)

theorem merge_selected_eq_sort (selected tail : List PublicId) (ordered : tail.Pairwise (· ≤ ·)) :
    mergeSelected selected tail = sortIds (selected ++ tail) :=
  List.eq_of_perm_of_sorted ((merge_selected_perm selected tail).trans (sort_ids_perm _).symm)
    (merge_selected_ordered selected tail ordered) (sort_ids_ordered _)

theorem sort_append (selected tail : List PublicId) :
    sortIds (selected ++ tail) = mergeSelected selected (sortIds tail) := by
  apply List.eq_of_perm_of_sorted (r := (· ≤ ·))
  · exact (sort_ids_perm _).trans
      ((merge_selected_perm selected (sortIds tail)).trans ((sort_ids_perm tail).append_left selected)).symm
  · exact sort_ids_ordered _
  · exact merge_selected_ordered _ _ (sort_ids_ordered tail)

theorem public_set_lower (selected ids picks : List PublicId) (unique : picks.Nodup)
    (count : picks.length ≤ 5) (members : ∀ value ∈ picks, value ∈ ids) :
    sortIds (selected ++ (occupied (collect ids)).take picks.length) ≤ sortIds (selected ++ picks) := by
  have permutation := sort_ids_perm picks
  have sortedUnique := permutation.nodup_iff.mpr unique
  have sortedCount : (sortIds picks).length ≤ 5 := by simpa only [permutation.length_eq] using count
  have lower := collected_prefix_lower ids (sortIds picks) sortedUnique sortedCount
    (fun value member => members value (permutation.mem_iff.mp member))
  rw [permutation.length_eq] at lower
  have sufficient := collected_sufficient ids picks unique count members
  have ordered := (collect_ordered_unique ids).1
  rw [← merge_selected_eq_sort selected _ ordered.take, sort_append selected picks]
  apply merge_selected_mono
  · simp only [List.length_take, Nat.min_eq_left sufficient, permutation.length_eq]
  · exact lower

end SmallIds

namespace TiePruning

def idFeed (entries : List Entry) (pos : Nat) (used : Finset (Fin 27))
    (selected : List SmallIds.PublicId) (least : Nat) : List SmallIds.PublicId :=
  (((entries.drop pos).takeWhile (fun entry => decide (least ≤ entry.power))).filter
    (fun entry => decide (entry.character ∉ used ∧ entry.publicId ∉ selected))).map Entry.publicId

def cannotEnter (entries : List Entry) (pos : Nat) (used : Finset (Fin 27))
    (selected : List SmallIds.PublicId) (slots sum least honor : Nat) (cap : Option Nat)
    (cutoff : Option Nat) (kth : Option (List SmallIds.PublicId)) : Bool :=
  match kth with
  | none => false
  | some publicSet =>
      let raw := Saturating.add Saturating.u32Max sum honor
      if PowerModel.clamp cap raw ≠ raw ∨ cutoff ≠ some raw then false else
        let ids := SmallIds.occupied (SmallIds.collect (idFeed entries pos used selected least))
        if ids.length < slots then true else
          decide (publicSet < SmallIds.sortIds (selected ++ ids.take slots))

theorem threshold_member (entries : List Entry)
    (ordered : entries.Pairwise (fun left right => right.power ≤ left.power))
    (entry : Entry) (member : entry ∈ entries) (least : Nat) (high : least ≤ entry.power) :
    entry ∈ entries.takeWhile (fun item => decide (least ≤ item.power)) := by
  induction entries with
  | nil => simp at member
  | cons first rest ih =>
      have ordering := List.pairwise_cons.mp ordered
      have firstHigh : least ≤ first.power := by
        rcases List.mem_cons.mp member with same | later
        · simpa only [same] using high
        · exact high.trans (ordering.1 entry later)
      simp only [List.takeWhile_cons, firstHigh, decide_true, ↓reduceIte]
      rcases List.mem_cons.mp member with same | later
      · exact List.mem_cons.mpr (Or.inl same)
      · exact List.mem_cons_of_mem _ (ih ordering.2 later)

theorem feed_contains_optimal (entries candidates : List Entry) (pos : Nat) (used : Finset (Fin 27))
    (selected : List SmallIds.PublicId) (slots sum least : Nat)
    (size : candidates.length = slots) (unique : (candidates.map Entry.character).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ (sortedEntries entries).drop pos ∧ entry.character ∉ used)
    (disjoint : ∀ entry ∈ candidates, entry.publicId ∉ selected)
    (found : bestCompletion (sortedEntries entries) pos used slots = some (sum, least))
    (optimal : (candidates.map Entry.power).sum = sum) :
    ∀ id ∈ candidates.map Entry.publicId,
      id ∈ idFeed (sortedEntries entries) pos used selected least := by
  obtain ⟨bound, minimum, result, _, high⟩ := best_completion_spec entries candidates pos used slots size unique available
  rw [found] at result
  have same := Prod.mk.inj (Option.some.inj result)
  obtain ⟨rfl, rfl⟩ := same
  have ordered : ((sortedEntries entries).drop pos).Pairwise (fun left right => right.power ≤ left.power) :=
    (sorted_entries_power entries).drop
  intro id member
  obtain ⟨entry, belongs, rfl⟩ := List.mem_map.mp member
  apply List.mem_map.mpr
  refine ⟨entry, List.mem_filter.mpr ⟨?_, ?_⟩, rfl⟩
  · exact threshold_member _ ordered entry (available entry belongs).1 least (high optimal entry belongs)
  · exact decide_eq_true ⟨(available entry belongs).2, disjoint entry belongs⟩

theorem rejected_public_set (entries candidates : List Entry) (pos : Nat) (used : Finset (Fin 27))
    (selected kth : List SmallIds.PublicId) (slots bound least sum honor : Nat) (cap : Option Nat) (cutoff : Option Nat)
    (size : candidates.length = slots) (count : slots ≤ 5)
    (characters : (candidates.map Entry.character).Nodup) (publicUnique : (candidates.map Entry.publicId).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ (sortedEntries entries).drop pos ∧ entry.character ∉ used)
    (disjoint : ∀ entry ∈ candidates, entry.publicId ∉ selected)
    (found : bestCompletion (sortedEntries entries) pos used slots = some (bound, least))
    (optimal : (candidates.map Entry.power).sum = bound)
    (pruned : cannotEnter (sortedEntries entries) pos used selected slots sum least honor cap cutoff (some kth) = true) :
    kth < SmallIds.sortIds (selected ++ candidates.map Entry.publicId) := by
  have members := feed_contains_optimal entries candidates pos used selected slots bound least size characters available disjoint found optimal
  have capacity : (candidates.map Entry.publicId).length ≤ 5 := by simpa only [List.length_map, size] using count
  have enough := SmallIds.collected_sufficient _ _ publicUnique capacity members
  have lower := SmallIds.public_set_lower selected _ _ publicUnique capacity members
  simp only [List.length_map, size] at enough lower
  unfold cannotEnter at pruned
  dsimp only at pruned
  split at pruned
  · contradiction
  · rw [if_neg (not_lt.mpr enough)] at pruned
    exact (of_decide_eq_true pruned).trans_le lower

theorem rejection_guards (entries : List Entry) (pos : Nat) (used : Finset (Fin 27))
    (selected kth : List SmallIds.PublicId) (slots sum least honor : Nat) (cap : Option Nat) (cutoff : Option Nat)
    (pruned : cannotEnter entries pos used selected slots sum least honor cap cutoff (some kth) = true) :
    PowerModel.clamp cap (Saturating.add Saturating.u32Max sum honor) = Saturating.add Saturating.u32Max sum honor ∧
      cutoff = some (Saturating.add Saturating.u32Max sum honor) := by
  unfold cannotEnter at pruned
  dsimp only at pruned
  split at pruned
  · contradiction
  · rename_i guards
    exact ⟨not_ne_iff.mp (not_or.mp guards).1, not_ne_iff.mp (not_or.mp guards).2⟩

theorem public_values_strict (left right : List SmallIds.PublicId) (smaller : left < right) :
    left.map Fin.val < right.map Fin.val := by
  change List.Lex (· < ·) left right at smaller
  change List.Lex (· < ·) _ _
  induction smaller with
  | nil => exact List.Lex.nil
  | rel headLess => exact List.Lex.rel headLess
  | cons tailLess ih => exact List.Lex.cons ih

theorem power_key_public_order (score : Nat) (left right : List SmallIds.PublicId)
    (leftOrder leftDense rightOrder rightDense : List Nat) (smaller : left < right) :
    Canonical.make score 0 (left.map Fin.val) leftOrder leftDense <
      Canonical.make score 0 (right.map Fin.val) rightOrder rightDense := by
  apply (Canonical.key_lt_iff _ _ _ _ _ _ _ _ _ _).mpr
  exact Or.inr ⟨rfl, Or.inr ⟨rfl, Or.inl (public_values_strict left right smaller)⟩⟩

theorem rejected_canonical_key (entries candidates : List Entry) (pos : Nat) (used : Finset (Fin 27))
    (selected kth : List SmallIds.PublicId) (slots bound least selectedPower actualPower honor : Nat)
    (cap : Option Nat) (cutoff : Option Nat) (kthOrder kthDense candidateOrder candidateDense : List Nat)
    (size : candidates.length = slots) (count : slots ≤ 5)
    (characters : (candidates.map Entry.character).Nodup) (publicUnique : (candidates.map Entry.publicId).Nodup)
    (available : ∀ entry ∈ candidates, entry ∈ (sortedEntries entries).drop pos ∧ entry.character ∉ used)
    (disjoint : ∀ entry ∈ candidates, entry.publicId ∉ selected)
    (found : bestCompletion (sortedEntries entries) pos used slots = some (bound, least))
    (actualUpper : actualPower ≤ selectedPower + (candidates.map Entry.power).sum)
    (fits : selectedPower + bound + honor ≤ Saturating.u32Max)
    (pruned : cannotEnter (sortedEntries entries) pos used selected slots (selectedPower + bound) least honor cap cutoff (some kth) = true) :
    Canonical.make (selectedPower + bound + honor) 0 (kth.map Fin.val) kthOrder kthDense <
      Canonical.make (PowerModel.clamp cap (actualPower + honor)) 0
        ((SmallIds.sortIds (selected ++ candidates.map Entry.publicId)).map Fin.val) candidateOrder candidateDense := by
  obtain ⟨computed, minimum, result, upper, _⟩ := best_completion_spec entries candidates pos used slots size characters available
  rw [found] at result
  obtain ⟨rfl, rfl⟩ := Prod.mk.inj (Option.some.inj result)
  have guards := rejection_guards _ pos used selected kth slots (selectedPower + bound) least honor cap cutoff pruned
  have raw : Saturating.add Saturating.u32Max (selectedPower + bound) honor = selectedPower + bound + honor := by
    simp only [Saturating.add, Saturating.clip, Nat.min_eq_left fits]
  rw [raw] at guards
  have capped : PowerModel.clamp cap (actualPower + honor) ≤ actualPower + honor := by
    cases cap with
    | none => exact le_rfl
    | some limit => exact min_le_left _ _
  have scoreBound : PowerModel.clamp cap (actualPower + honor) ≤ selectedPower + bound + honor := by omega
  rcases lt_or_eq_of_le scoreBound with lowerScore | tied
  · exact Canonical.score_order _ _ lowerScore
  · have optimal : (candidates.map Entry.power).sum = bound := by omega
    have publicOrder := rejected_public_set entries candidates pos used selected kth slots bound least
      (selectedPower + bound) honor cap cutoff size count characters publicUnique available disjoint found optimal pruned
    rw [tied]
    exact power_key_public_order _ _ _ _ _ _ _ publicOrder

end TiePruning

namespace LeafEvaluation

theorem support_steps_commute (state : DeckComposition.MixedState) (first second : DeckComposition.CardFacts) :
    DeckComposition.supportStep (DeckComposition.supportStep state first) second =
      DeckComposition.supportStep (DeckComposition.supportStep state second) first := by
  classical
  by_cases firstVirtual : first.original = DeckComposition.piapro
  · by_cases secondVirtual : second.original = DeckComposition.piapro
    · cases firstSupport : first.support with
      | none =>
          cases secondSupport : second.support with
          | none => simp [DeckComposition.supportStep, firstVirtual, secondVirtual, firstSupport, secondSupport]
          | some unit =>
              by_cases present : unit ∈ state.units <;>
                simp [DeckComposition.supportStep, firstVirtual, secondVirtual, firstSupport, secondSupport, present]
      | some unit =>
          cases secondSupport : second.support with
          | none =>
              by_cases present : unit ∈ state.units <;>
                simp [DeckComposition.supportStep, firstVirtual, secondVirtual, firstSupport, secondSupport, present]
          | some other =>
              by_cases same : unit = other
              · subst other
                simp [firstSupport, secondSupport, DeckComposition.supportStep, firstVirtual, secondVirtual]
              · by_cases unitPresent : unit ∈ state.units <;> by_cases otherPresent : other ∈ state.units <;>
                  simp [DeckComposition.supportStep, firstVirtual, secondVirtual, firstSupport, secondSupport,
                    same, Ne.symm same, unitPresent, otherPresent, Finset.insert_comm]
    · simp [DeckComposition.supportStep, secondVirtual]
  · simp [DeckComposition.supportStep, firstVirtual]

theorem support_fold_permutation (left right : List DeckComposition.CardFacts) (permutation : left.Perm right)
    (initial : DeckComposition.MixedState) :
    left.foldl DeckComposition.supportStep initial = right.foldl DeckComposition.supportStep initial := by
  induction permutation generalizing initial with
  | nil => rfl
  | cons first permutation ih => exact ih _
  | swap first second rest =>
      simp only [List.foldl_cons, support_steps_commute initial first second]
  | trans first second ihFirst ihSecond => exact (ihFirst initial).trans (ihSecond initial)

theorem mixed_activation_permutation (left right : List DeckComposition.CardFacts) (permutation : left.Perm right) :
    DeckComposition.isMultiUnit left = DeckComposition.isMultiUnit right := by
  have original : DeckComposition.nonVirtualUnits left = DeckComposition.nonVirtualUnits right := by
    ext unit
    simp only [DeckComposition.nonVirtualUnits, List.mem_toFinset]
    exact ((permutation.filter _).map DeckComposition.CardFacts.original).mem_iff
  have state : DeckComposition.mixedState left = DeckComposition.mixedState right := by
    unfold DeckComposition.mixedState
    rw [original]
    exact support_fold_permutation left right permutation _
  simp only [DeckComposition.isMultiUnit, DeckComposition.mixedUnits, state]

theorem common_units_permutation {Id : Type*} (data : Id → PowerModel.CardData)
    (left right : List Id) (permutation : left.Perm right) :
    PowerModel.commonUnits data left = PowerModel.commonUnits data right := by
  ext unit
  simp only [PowerModel.mem_commonUnits]
  constructor
  · intro all index member
    exact all index (permutation.mem_iff.mpr member)
  · intro all index member
    exact all index (permutation.mem_iff.mp member)

theorem shared_attribute_permutation {Id : Type*} (data : Id → PowerModel.CardData)
    (left right : List Id) (permutation : left.Perm right) :
    PowerModel.sharesAttribute data left = PowerModel.sharesAttribute data right := by
  apply Bool.eq_iff_iff.mpr
  rw [PowerModel.sharesAttribute_eq_true, PowerModel.sharesAttribute_eq_true]
  constructor
  · rintro ⟨attrValue, uniform⟩
    exact ⟨attrValue, fun index member => uniform index (permutation.mem_iff.mpr member)⟩
  · rintro ⟨attrValue, uniform⟩
    exact ⟨attrValue, fun index member => uniform index (permutation.mem_iff.mp member)⟩

theorem card_power_permutation {Id : Type*} (mode : MixedPower.Mode) (data : Id → MixedPower.Card)
    (left right : List Id) (permutation : left.Perm right) (index : Id) :
    MixedPower.cardPower mode data left index = MixedPower.cardPower mode data right index := by
  have mixed := mixed_activation_permutation _ _ (permutation.filterMap (fun index => MixedPower.activeFact (data index)))
  have units := common_units_permutation (fun index => MixedPower.legacy (data index)) left right permutation
  have attrSame := shared_attribute_permutation (fun index => MixedPower.legacy (data index)) left right permutation
  simp only [MixedPower.cardPower, MixedPower.deckFacts, mixed, units, attrSame, permutation.length_eq]

theorem total_permutation {Id : Type*} (mode : MixedPower.Mode) (data : Id → MixedPower.Card)
    (left right : List Id) (permutation : left.Perm right) :
    MixedPower.total mode data left = MixedPower.total mode data right := by
  unfold MixedPower.total
  calc
    (left.map (MixedPower.cardPower mode data left)).sum = (left.map (MixedPower.cardPower mode data right)).sum := by
      congr 1
      apply List.map_congr_left
      intro index _
      exact card_power_permutation mode data left right permutation index
    _ = (right.map (MixedPower.cardPower mode data right)).sum := (permutation.map _).sum_eq

def publicId (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (index : Fin pool.cards.length) : SmallIds.PublicId :=
  ⟨pool.cards[index].row.publicId.toNat, by
    have domain := Admission.GatherPipeline.row_domain pool valid index
    have lower := domain.1
    have upper := domain.2.1
    omega⟩

def reportCode (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (index : Fin pool.cards.length) : Nat := (publicId pool valid index).val * 65536 + index.val

theorem pool_count (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool) :
    pool.cards.length ≤ 65535 := by
  have count := ((Admission.CardTable.validate_iff _).mp valid.1).1
  simpa only [List.length_map] using count

theorem report_code_injective (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool) :
    Function.Injective (reportCode pool valid) := by
  intro left right equal
  have count := pool_count pool valid
  have leftBound := left.isLt
  have rightBound := right.isLt
  unfold reportCode at equal
  apply Fin.ext
  omega

theorem report_code_source (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (index : Fin pool.cards.length) :
    reportCode pool valid index = ((publicId pool valid index).val <<< 16) ||| index.val ∧
      reportCode pool valid index ≤ Saturating.u32Max := by
  have count := pool_count pool valid
  have indexBound : index.val < 2 ^ 16 := by have bounded := index.isLt; omega
  have publicBound := (publicId pool valid index).isLt
  constructor
  · unfold reportCode
    rw [← Nat.shiftLeft_add_eq_or_of_lt (i := 16) indexBound, Nat.shiftLeft_eq]
  · unfold reportCode Saturating.u32Max
    omega

theorem report_code_public_order (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (left right : Fin pool.cards.length) (ordered : reportCode pool valid left ≤ reportCode pool valid right) :
    publicId pool valid left ≤ publicId pool valid right := by
  have count := pool_count pool valid
  have leftBound := left.isLt
  have rightBound := right.isLt
  unfold reportCode at ordered
  change (publicId pool valid left).val ≤ (publicId pool valid right).val
  omega

def reported (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (deck : List (Fin pool.cards.length)) : List (Fin pool.cards.length) :=
  deck.insertionSort (fun left right => reportCode pool valid left ≤ reportCode pool valid right)

theorem reported_perm (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (deck : List (Fin pool.cards.length)) : (reported pool valid deck).Perm deck := List.perm_insertionSort _ _

theorem reported_ordered (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (deck : List (Fin pool.cards.length)) :
    (reported pool valid deck).Pairwise (fun left right => reportCode pool valid left ≤ reportCode pool valid right) := by
  letI : IsTotal (Fin pool.cards.length) (fun a b => reportCode pool valid a ≤ reportCode pool valid b) :=
    ⟨fun a b => le_total _ _⟩
  letI : IsTrans (Fin pool.cards.length) (fun a b => reportCode pool valid a ≤ reportCode pool valid b) :=
    ⟨fun a b c ab bc => ab.trans bc⟩
  exact List.sorted_insertionSort _ _

theorem reported_public_ids (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (deck : List (Fin pool.cards.length)) :
    (reported pool valid deck).map (publicId pool valid) = SmallIds.sortIds (deck.map (publicId pool valid)) := by
  have ordered : ((reported pool valid deck).map (publicId pool valid)).Pairwise (· ≤ ·) :=
    List.pairwise_map.mpr ((reported_ordered pool valid deck).imp (fun order => report_code_public_order pool valid _ _ order))
  exact List.eq_of_perm_of_sorted
    (((reported_perm pool valid deck).map _).trans (SmallIds.sort_ids_perm _).symm)
    ordered (SmallIds.sort_ids_ordered _)

theorem reported_permutation (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (left right : List (Fin pool.cards.length)) (permutation : left.Perm right) :
    reported pool valid left = reported pool valid right := by
  letI : IsAntisymm (Fin pool.cards.length) (fun a b => reportCode pool valid a ≤ reportCode pool valid b) :=
    ⟨fun a b ab ba => report_code_injective pool valid (le_antisymm ab ba)⟩
  exact List.eq_of_perm_of_sorted
    ((reported_perm pool valid left).trans (permutation.trans (reported_perm pool valid right).symm))
    (reported_ordered pool valid left) (reported_ordered pool valid right)

noncomputable def score (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (honor : Nat) (cap : Option Nat) (deck : List (Fin pool.cards.length)) : Nat :=
  PowerModel.clamp cap (MixedPower.total mode (PowerProduction.poolData pool valid) deck + honor)

noncomputable def resultKey (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (honor : Nat) (cap : Option Nat) (deck : List (Fin pool.cards.length)) : Canonical.Key :=
  let output := reported pool valid deck
  Canonical.make (score pool valid mode honor cap deck) 0
    ((SmallIds.sortIds (output.map (publicId pool valid))).map Fin.val)
    (output.map (fun index => (publicId pool valid index).val)) (output.map Fin.val)

theorem result_key_permutation (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (mode : MixedPower.Mode) (honor : Nat) (cap : Option Nat)
    (left right : List (Fin pool.cards.length)) (permutation : left.Perm right) :
    resultKey pool valid mode honor cap left = resultKey pool valid mode honor cap right := by
  simp only [resultKey, reported_permutation pool valid left right permutation, score,
    total_permutation mode (PowerProduction.poolData pool valid) left right permutation]

theorem built_actual_sum_limit (sources : List (PowerProduction.Input × Admission.GatherOrder.Card))
    (context : Admission.SourceNumeric.Context) (pool : Admission.GatherPipeline.Pool)
    (accepted : Admission.SourceNumeric.build (PowerProduction.sourceCards sources) context = .ok pool)
    (picks : List (Fin pool.cards.length)) (unique : picks.Nodup) (full : picks.length = 5) (mode : MixedPower.Mode) :
    let valid := (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1
    MixedPower.total mode (PowerProduction.poolData pool valid) picks + context.honor.val ≤ 2 ^ 24 := by
  dsimp only
  let valid := (Admission.SourceNumeric.concrete_success _ context pool accepted).2.1
  let data := PowerProduction.poolData pool valid
  let common := PowerModel.commonUnits (fun index => MixedPower.legacy (data index)) picks
  let attrFlag := PowerModel.sharesAttribute (fun index => MixedPower.legacy (data index)) picks
  have upper := List.sum_le_sum (fun index (_ : index ∈ picks) =>
    ScenarioPower.source_full_deck_bound mode data picks full index)
  have limit := PowerProduction.built_scenario_sum_limit sources context pool accepted picks unique
    (Nat.le_of_eq full) mode common attrFlag
  exact (Nat.add_le_add_right upper context.honor.val).trans limit

end LeafEvaluation

namespace BoundedTracker

def Valid (capacity : Nat) (keys : List Canonical.Key) : Prop :=
  keys.Pairwise (· ≤ ·) ∧ (keys.map Canonical.identity).Nodup ∧ keys.length ≤ capacity

def cutoff (capacity : Nat) (keys : List Canonical.Key) : Option Nat :=
  if keys.length < capacity then none else keys.getLast?.map Canonical.score

def publicSet (capacity : Nat) (keys : List Canonical.Key) : Option (List Nat) :=
  if keys.length < capacity then none else keys.getLast?.map Canonical.identity

def insertCore (capacity : Nat) (keys : List Canonical.Key) (candidate : Canonical.Key) : List Canonical.Key :=
  let position := (keys.takeWhile (fun old => decide (old < candidate))).length
  let next := keys.take position ++ candidate :: keys.drop position
  if capacity < next.length then next.dropLast else next

def insert (capacity : Nat) (keys : List Canonical.Key) (candidate : Canonical.Key) : List Canonical.Key :=
  if capacity = 0 then keys else
    if capacity ≤ keys.length ∧ (keys.getLast?.any (fun last => decide (last ≤ candidate))) then keys else
      match keys.find? (fun old => decide (Canonical.identity old = Canonical.identity candidate)) with
      | none => insertCore capacity keys candidate
      | some existing =>
          if existing ≤ candidate then keys else insertCore capacity (keys.erase existing) candidate

theorem identity_unique (keys : List Canonical.Key) (unique : (keys.map Canonical.identity).Nodup)
    (left : Canonical.Key) (leftMember : left ∈ keys) (right : Canonical.Key) (rightMember : right ∈ keys)
    (same : Canonical.identity left = Canonical.identity right) : left = right := by
  induction keys with
  | nil => simp at leftMember
  | cons first rest ih =>
      have distinct := List.nodup_cons.mp unique
      rcases List.mem_cons.mp leftMember with leftSame | leftTail
      · subst left
        rcases List.mem_cons.mp rightMember with rightSame | rightTail
        · exact rightSame.symm
        · exact False.elim (distinct.1 (List.mem_map.mpr ⟨right, rightTail, same.symm⟩))
      · rcases List.mem_cons.mp rightMember with rightSame | rightTail
        · subst right
          exact False.elim (distinct.1 (List.mem_map.mpr ⟨left, leftTail, same⟩))
        · exact ih distinct.2 leftTail rightTail

theorem covered_weaken (capacity : Nat) (retained : Finset Canonical.Key) (first second : Canonical.Key)
    (same : Canonical.identity first = Canonical.identity second) (better : first ≤ second)
    (covered : Allium.Covered Canonical.identity capacity retained first) :
    Allium.Covered Canonical.identity capacity retained second := by
  rcases covered with ⟨key, member, identity, order⟩ | count
  · exact Or.inl ⟨key, member, identity.trans same, order.trans better⟩
  · exact Or.inr (count.trans (Finset.card_le_card (Allium.earlierIds_mono_key _ retained better)))

theorem bounded_self_topK (capacity : Nat) (keys : List Canonical.Key)
    (unique : (keys.map Canonical.identity).Nodup) (count : keys.length ≤ capacity) :
    Allium.topK Canonical.identity capacity keys.toFinset = keys.toFinset := by
  let retained := keys.toFinset
  have membership : ∀ key, key ∈ retained ↔ key ∈ keys := fun _ => List.mem_toFinset
  have lengthBound : retained.card ≤ keys.length := List.toFinset_card_le keys
  change Allium.topK Canonical.identity capacity retained = retained
  classical
  apply Finset.Subset.antisymm (Allium.topK_subset _ _ _)
  intro candidate member
  apply (Allium.mem_topK _ _ _ _).mpr
  refine ⟨member, ?_, ?_⟩
  · intro other belongs same
    have equal := identity_unique keys unique other ((membership other).mp belongs) candidate
      ((membership candidate).mp member) same
    exact le_of_eq equal.symm
  · have missing : candidate ∉ retained.filter (fun other => other < candidate) := by simp
    have strict : retained.filter (fun other => other < candidate) ⊂ retained :=
      Finset.ssubset_iff_subset_ne.mpr ⟨Finset.filter_subset _ _, by intro equal; rw [equal] at missing; exact missing member⟩
    have smaller := Finset.card_lt_card strict
    have imageBound := Finset.card_image_le (s := retained.filter (fun other => other < candidate)) (f := Canonical.identity)
    unfold Allium.earlierIds
    omega

theorem last_upper (keys : List Canonical.Key) (ordered : keys.Pairwise (· ≤ ·))
    (last : Canonical.Key) (found : keys.getLast? = some last) (key : Canonical.Key) (member : key ∈ keys) : key ≤ last := by
  induction keys generalizing key with
  | nil => simp at member
  | cons first rest ih =>
      have ordering := List.pairwise_cons.mp ordered
      cases rest with
      | nil =>
          have same : first = last := Option.some.inj found
          have item : key = first := List.mem_singleton.mp member
          simp only [item, same, le_refl]
      | cons second tail =>
          rw [List.getLast?_cons_cons] at found
          rcases List.mem_cons.mp member with same | belongs
          · subst key
            exact ordering.1 last (List.mem_of_getLast? found)
          · exact ih ordering.2 found key belongs

theorem identity_card (keys : List Canonical.Key) (unique : (keys.map Canonical.identity).Nodup) :
    (keys.toFinset.image Canonical.identity).card = keys.length := by
  let retained := keys.toFinset
  have membership : ∀ key, key ∈ retained ↔ key ∈ keys := fun _ => List.mem_toFinset
  have keyUnique : keys.Nodup := List.Nodup.of_map _ unique
  have card : retained.card = keys.length := List.toFinset_card_of_nodup keyUnique
  change (retained.image Canonical.identity).card = keys.length
  rw [Finset.card_image_of_injOn, card]
  intro a am b bm same
  exact identity_unique keys unique a ((membership a).mp am) b ((membership b).mp bm) same

theorem last_witness (capacity : Nat) (keys : List Canonical.Key) (valid : Valid capacity keys)
    (full : capacity ≤ keys.length) (last candidate : Canonical.Key)
    (found : keys.getLast? = some last) (better : last < candidate) :
    Allium.Covered Canonical.identity capacity keys.toFinset candidate := by
  let retained := keys.toFinset
  have membership : ∀ key, key ∈ retained ↔ key ∈ keys := fun _ => List.mem_toFinset
  have count : (retained.image Canonical.identity).card = keys.length := identity_card keys valid.2.1
  change Allium.Covered Canonical.identity capacity retained candidate
  apply Or.inr
  have filtered : retained.filter (fun key => key < candidate) = retained := by
    apply Finset.filter_eq_self.mpr
    intro key member
    exact (last_upper keys valid.1 last found key ((membership key).mp member)).trans_lt better
  simpa only [Allium.earlierIds, filtered, count] using full

theorem ordered_insert_position (keys : List Canonical.Key) (candidate : Canonical.Key) :
    keys.orderedInsert (· ≤ ·) candidate =
      keys.take ((keys.takeWhile (fun old => decide (old < candidate))).length) ++
        candidate :: keys.drop ((keys.takeWhile (fun old => decide (old < candidate))).length) := by
  induction keys with
  | nil => simp
  | cons first rest ih =>
      by_cases less : first < candidate
      · simp [List.orderedInsert, less, not_le.mpr less, ih]
      · have before : candidate ≤ first := le_of_not_gt less
        simp [List.orderedInsert, less, before]

theorem core_projection (capacity : Nat) (keys : List Canonical.Key) (candidate : Canonical.Key)
    (count : keys.length ≤ capacity) :
    insertCore capacity keys candidate = (keys.orderedInsert (· ≤ ·) candidate).take capacity := by
  unfold insertCore
  dsimp only
  rw [← ordered_insert_position]
  have size := List.orderedInsert_length (· ≤ ·) keys candidate
  split
  · rename_i longer
    have length : (keys.orderedInsert (· ≤ ·) candidate).length = capacity + 1 := by omega
    rw [List.dropLast_eq_take, length]
    simp
  · rename_i short
    exact (List.take_of_length_le (Nat.le_of_not_gt short)).symm

theorem prefix_covers (capacity : Nat) (keys : List Canonical.Key) (ordered : keys.Pairwise (· ≤ ·))
    (unique : (keys.map Canonical.identity).Nodup) (candidate : Canonical.Key) (member : candidate ∈ keys) :
    Allium.Covered Canonical.identity capacity (keys.take capacity).toFinset candidate := by
  by_cases present : candidate ∈ keys.take capacity
  · exact Allium.covered_of_mem _ _ (List.mem_toFinset.mpr present)
  · have long : capacity ≤ keys.length := by
      by_contra short
      have all := List.take_of_length_le (Nat.le_of_lt (Nat.lt_of_not_ge short)) (l := keys)
      exact present (all.symm ▸ member)
    have tailMember : candidate ∈ keys.drop capacity := by
      have splitMember : candidate ∈ keys.take capacity ++ keys.drop capacity := by simpa using member
      exact (List.mem_append.mp splitMember).resolve_left present
    have divided : (keys.take capacity ++ keys.drop capacity).Pairwise (· ≤ ·) := by simpa using ordered
    have between := (List.pairwise_append.mp divided).2.2
    have keptUnique : ((keys.take capacity).map Canonical.identity).Nodup := by
      rw [List.map_take]
      exact unique.take
    have count := identity_card (keys.take capacity) keptUnique
    have length : (keys.take capacity).length = capacity := by simp [Nat.min_eq_left long]
    let retained := (keys.take capacity).toFinset
    have membership : ∀ key, key ∈ retained ↔ key ∈ keys.take capacity := fun _ => List.mem_toFinset
    change Allium.Covered Canonical.identity capacity retained candidate
    apply Or.inr
    have filtered : retained.filter (fun key => key < candidate) = retained := by
      apply Finset.filter_eq_self.mpr
      intro key belongs
      have kept := (membership key).mp belongs
      exact lt_of_le_of_ne (between key kept candidate tailMember) (fun same => present (same ▸ kept))
    have retainedCount : (retained.image Canonical.identity).card = capacity := count.trans length
    simpa only [Allium.earlierIds, filtered, retainedCount] using (le_refl capacity)

theorem core_properties (capacity : Nat) (keys : List Canonical.Key) (candidate : Canonical.Key)
    (valid : Valid capacity keys) (fresh : Canonical.identity candidate ∉ keys.map Canonical.identity) :
    Valid capacity (insertCore capacity keys candidate) ∧
      (∀ key ∈ insertCore capacity keys candidate, key = candidate ∨ key ∈ keys) ∧
      ∀ key ∈ candidate :: keys, Allium.Covered Canonical.identity capacity (insertCore capacity keys candidate).toFinset key := by
  rw [core_projection capacity keys candidate valid.2.2]
  have ordered : (keys.orderedInsert (· ≤ ·) candidate).Pairwise (· ≤ ·) := List.Sorted.orderedInsert _ _ valid.1
  have permutation := List.perm_orderedInsert (· ≤ ·) candidate keys
  have unique : ((keys.orderedInsert (· ≤ ·) candidate).map Canonical.identity).Nodup := by
    apply (permutation.map Canonical.identity).nodup_iff.mpr
    exact List.nodup_cons.mpr ⟨fresh, valid.2.1⟩
  refine ⟨⟨ordered.take, ?_, by simp⟩, ?_, ?_⟩
  · rw [List.map_take]
    exact unique.take
  · intro key member
    exact (List.mem_orderedInsert (· ≤ ·)).mp (List.mem_of_mem_take member)
  · intro key member
    exact prefix_covers capacity _ ordered unique key (permutation.mem_iff.mpr member)

theorem insert_properties (capacity : Nat) (keys : List Canonical.Key) (candidate : Canonical.Key)
    (valid : Valid capacity keys) :
    Valid capacity (insert capacity keys candidate) ∧
      (∀ key ∈ insert capacity keys candidate, key = candidate ∨ key ∈ keys) ∧
      ∀ key ∈ candidate :: keys, Allium.Covered Canonical.identity capacity (insert capacity keys candidate).toFinset key := by
  unfold insert
  by_cases zero : capacity = 0
  · simp only [if_pos zero]
    refine ⟨valid, fun key member => Or.inr member, ?_⟩
    intro key _
    exact Or.inr (by rw [zero]; exact Nat.zero_le _)
  · simp only [if_neg zero]
    by_cases late : capacity ≤ keys.length ∧ keys.getLast?.any (fun last => decide (last ≤ candidate)) = true
    · simp only [if_pos late]
      refine ⟨valid, fun key member => Or.inr member, ?_⟩
      intro key member
      rcases List.mem_cons.mp member with same | old
      · subst key
        obtain ⟨last, found, order⟩ : ∃ last, keys.getLast? = some last ∧ last ≤ candidate := by
          cases found : keys.getLast? with
          | none => simp [found] at late
          | some last => exact ⟨last, rfl, by simpa [found] using late.2⟩
        rcases lt_or_eq_of_le order with less | equal
        · exact last_witness capacity keys valid late.1 last candidate found less
        · exact Allium.covered_of_mem _ _ (List.mem_toFinset.mpr (equal ▸ List.mem_of_getLast? found))
      · exact Allium.covered_of_mem _ _ (List.mem_toFinset.mpr old)
    · simp only [if_neg late]
      cases existing : keys.find? (fun old => decide (Canonical.identity old = Canonical.identity candidate)) with
      | none =>
          have noMatch : ∀ old ∈ keys, Canonical.identity old ≠ Canonical.identity candidate := by
            simpa only [List.find?_eq_none, decide_eq_true_eq] using existing
          have fresh : Canonical.identity candidate ∉ keys.map Canonical.identity := by
            intro member
            obtain ⟨old, belongs, same⟩ := List.mem_map.mp member
            exact noMatch old belongs same
          exact core_properties capacity keys candidate valid fresh
      | some old =>
          have oldMember := List.mem_of_find?_eq_some existing
          have sameIdentity : Canonical.identity old = Canonical.identity candidate := by
            have tested := List.find?_some (p := fun key => decide (Canonical.identity key = Canonical.identity candidate)) existing
            exact of_decide_eq_true tested
          by_cases worse : old ≤ candidate
          · simp only [if_pos worse]
            refine ⟨valid, fun key member => Or.inr member, ?_⟩
            intro key member
            rcases List.mem_cons.mp member with same | belongs
            · subst key
              exact Or.inl ⟨old, List.mem_toFinset.mpr oldMember, sameIdentity, worse⟩
            · exact Allium.covered_of_mem _ _ (List.mem_toFinset.mpr belongs)
          · simp only [if_neg worse]
            have removed : (keys.erase old).Sublist keys := List.erase_sublist
            have erasedValid : Valid capacity (keys.erase old) :=
              ⟨valid.1.sublist removed, valid.2.1.sublist (removed.map Canonical.identity), removed.length_le.trans valid.2.2⟩
            have unique := ((List.perm_cons_erase oldMember).map Canonical.identity).nodup_iff.mp valid.2.1
            simp only [List.map_cons, List.nodup_cons] at unique
            have fresh : Canonical.identity candidate ∉ (keys.erase old).map Canonical.identity := by
              rw [← sameIdentity]
              exact unique.1
            have core := core_properties capacity (keys.erase old) candidate erasedValid fresh
            refine ⟨core.1, ?_, ?_⟩
            · intro key member
              rcases core.2.1 key member with same | belongs
              · exact Or.inl same
              · exact Or.inr (List.mem_of_mem_erase belongs)
            · intro key member
              rcases List.mem_cons.mp member with same | belongs
              · subst key
                exact core.2.2 candidate (by simp)
              · by_cases same : key = old
                · subst key
                  exact covered_weaken capacity _ candidate old sameIdentity.symm (lt_of_not_ge worse).le
                    (core.2.2 candidate (by simp))
                · exact core.2.2 key (List.mem_cons_of_mem _ ((List.mem_erase_of_ne same).mpr belongs))

theorem insert_exact (capacity : Nat) (keys : List Canonical.Key) (candidate : Canonical.Key)
    (valid : Valid capacity keys) :
    (insert capacity keys candidate).toFinset =
      Allium.topK Canonical.identity capacity (Insert.insert candidate keys.toFinset) := by
  have properties := insert_properties capacity keys candidate valid
  let output := (insert capacity keys candidate).toFinset
  let full : Finset Canonical.Key := Insert.insert candidate keys.toFinset
  have outputMember : ∀ key, key ∈ output ↔ key ∈ insert capacity keys candidate := fun _ => List.mem_toFinset
  have fullMember : ∀ key, key ∈ full ↔ key ∈ candidate :: keys := by
    intro key
    simp only [full, Finset.mem_insert, List.mem_toFinset, List.mem_cons]
  have sub : output ⊆ full := by
    intro key member
    exact (fullMember key).mpr (List.mem_cons.mpr (properties.2.1 key ((outputMember key).mp member)))
  have covered : ∀ key ∈ full, Allium.Covered Canonical.identity capacity output key := by
    intro key member
    exact properties.2.2 key ((fullMember key).mp member)
  have exactness := Allium.coverage_exactness Canonical.identity capacity sub covered
  have self : Allium.topK Canonical.identity capacity output = output :=
    bounded_self_topK capacity _ properties.1.2.1 properties.1.2.2
  rw [self] at exactness
  exact exactness

/-- Observed keys are a ghost trace, not additional runtime storage. The
bounded key vector follows the source insertion algorithm. -/
structure State where
  keys : List Canonical.Key
  observed : Finset Canonical.Key

def initial : State := ⟨[], ∅⟩

def step (capacity : Nat) (state : State) (candidate : Canonical.Key) : State :=
  ⟨insert capacity state.keys candidate, Insert.insert candidate state.observed⟩

def WellFormed (capacity : Nat) (state : State) : Prop :=
  Valid capacity state.keys ∧ state.keys.toFinset = Allium.topK Canonical.identity capacity state.observed

theorem initial_well_formed (capacity : Nat) : WellFormed capacity initial := by
  simp [WellFormed, Valid, initial, Allium.topK]

theorem step_well_formed (capacity : Nat) (state : State) (candidate : Canonical.Key)
    (wellFormed : WellFormed capacity state) : WellFormed capacity (step capacity state candidate) := by
  refine ⟨(insert_properties capacity state.keys candidate wellFormed.1).1, ?_⟩
  change (insert capacity state.keys candidate).toFinset =
    Allium.topK Canonical.identity capacity (Insert.insert candidate state.observed)
  rw [insert_exact capacity state.keys candidate wellFormed.1, wellFormed.2]
  simpa only [Finset.union_singleton] using
    Allium.topK_union_left Canonical.identity capacity state.observed {candidate}

theorem retained_observed (capacity : Nat) (state : State) (wellFormed : WellFormed capacity state) :
    state.keys.toFinset ⊆ state.observed := by
  rw [wellFormed.2]
  exact Allium.topK_subset _ _ _

theorem observed_last_witness (capacity : Nat) (state : State) (wellFormed : WellFormed capacity state)
    (full : capacity ≤ state.keys.length) (last candidate : Canonical.Key)
    (found : state.keys.getLast? = some last) (better : last < candidate) :
    Allium.Covered Canonical.identity capacity state.observed candidate :=
  Allium.covered_mono _ _ (retained_observed capacity state wellFormed)
    (last_witness capacity state.keys wellFormed.1 full last candidate found better)

end BoundedTracker

/-- The guards of the dedicated dispatch in solver/numeric.rs. -/
structure Dispatch where
  powerTarget : Bool
  minimize : Bool
  uniqueCharacters : Bool
  fixedCards : List Nat
  fixedCharacters : List Nat
  forcedLeader : Option Nat
  teammateSkillLower : Option Nat
  powerCap : Option Nat

def enabled (context : Dispatch) : Bool :=
  context.powerTarget && !context.minimize && context.uniqueCharacters &&
    context.fixedCards.isEmpty && context.fixedCharacters.isEmpty && context.forcedLeader.isNone &&
    context.teammateSkillLower.isNone && context.powerCap.isNone

namespace SourceDFS

structure Context where
  numeric : Admission.SourceNumeric.Context
  capacity : Nat
  mode : MixedPower.Mode
  minimize : Bool
  uniqueCharacters : Bool
  forcedLeader : Option Nat
  teammateLowerBound : Option Binary64.Value

def dispatch (context : Context) : Dispatch :=
  { powerTarget := decide (context.numeric.target = .power)
    minimize := context.minimize
    uniqueCharacters := context.uniqueCharacters
    fixedCards := context.numeric.fixedCards.map Fin.val
    fixedCharacters := context.numeric.fixedCharacters.map Fin.val
    forcedLeader := context.forcedLeader
    teammateSkillLower := context.teammateLowerBound.map (fun _ => 0)
    powerCap := context.numeric.powerCap.map Fin.val }

inductive PollKind where
  | immediate
  | sampled
  deriving DecidableEq

abbrev Budget := PollKind → Nat → Bool

structure Machine where
  tracker : BoundedTracker.State
  observations : Nat

structure Outcome where
  machine : Machine
  interrupted : Bool

def initial : Machine := ⟨BoundedTracker.initial, 0⟩

def tick (machine : Machine) : Machine := { machine with observations := machine.observations + 1 }

def stop (machine : Machine) : Outcome := ⟨machine, true⟩
def complete (machine : Machine) : Outcome := ⟨machine, false⟩

def belowCutoff (context : Context) (machine : Machine) (sum : Nat) : Bool :=
  match BoundedTracker.cutoff context.capacity machine.tracker.keys with
  | none => false
  | some threshold => decide (PowerModel.clamp (context.numeric.powerCap.map Fin.val)
      (Saturating.add Saturating.u32Max sum context.numeric.honor.val) < threshold)

def publicCutoff (context : Context) (machine : Machine) : Option (List SmallIds.PublicId) :=
  (BoundedTracker.publicSet context.capacity machine.tracker.keys).map
    (List.map (fun id => ⟨id % 65536, Nat.mod_lt _ (by decide)⟩))

def tieCannotEnter (context : Context) (machine : Machine) (entries selected : List Entry)
    (used : Finset (Fin 27)) (slots sum least : Nat) : Bool :=
  TiePruning.cannotEnter entries 0 used (selected.map Entry.publicId) slots sum least
    context.numeric.honor.val (context.numeric.powerCap.map Fin.val)
    (BoundedTracker.cutoff context.capacity machine.tracker.keys) (publicCutoff context machine)

abbrev Continuation := List Entry → List Entry → Finset (Fin 27) → Nat → Machine → Outcome

/-- The sibling loop polls before its repeated-value bound, then excludes
used characters and public IDs. An interrupted child stops its siblings. -/
noncomputable def loop (context : Context) (budget : Budget) (child : Continuation) (slots : Nat) : Continuation
  | [], _, _, _, machine => complete machine
  | item :: rest, selected, used, sum, machine =>
      let next := tick machine
      if budget .sampled machine.observations then stop next else
        if belowCutoff context next (sum + item.power * slots) then complete next else
          if item.character ∈ used ∨ item.publicId ∈ selected.map Entry.publicId then
            loop context budget child slots rest selected used sum next
          else
            let result := child rest (selected ++ [item]) (Insert.insert item.character used) (sum + item.power) next
            if result.interrupted then result else
              loop context budget child slots rest selected used sum result.machine

/-- Leaf evaluation recomputes the actual deck composition. No scenario sum
is passed into the leaf result key. -/
noncomputable def descend (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) : Nat → Continuation
  | 0 => fun _ selected _ _ machine =>
      let key := LeafEvaluation.resultKey pool valid context.mode context.numeric.honor.val
        (context.numeric.powerCap.map Fin.val) (selected.map (SceneTable.decode pool positive))
      complete { machine with tracker := BoundedTracker.step context.capacity machine.tracker key }
  | slots + 1 => fun entries selected used sum machine =>
      match bestCompletion entries 0 used (slots + 1) with
      | none => complete machine
      | some (completion, least) =>
          if belowCutoff context machine (sum + completion) ||
              tieCannotEnter context machine entries selected used (slots + 1) (sum + completion) least then
            complete machine
          else loop context budget (descend context budget pool valid positive slots) (slots + 1)
            entries selected used sum machine

def sceneKey (scene : SceneTable.Scenario) : List Int := [-Int.ofNat scene.ceiling, Int.ofNat scene.order]

def sortedScenes (scenes : List SceneTable.Scenario) : List SceneTable.Scenario :=
  scenes.mergeSort (fun left right => decide (sceneKey left ≤ sceneKey right))

noncomputable def runScenes (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) : List SceneTable.Scenario → Machine → Outcome
  | [], machine => complete machine
  | scene :: rest, machine =>
      let next := tick machine
      if budget .immediate machine.observations then stop next else
        if belowCutoff context next scene.ceiling then runScenes context budget pool valid positive rest next else
          let result := descend context budget pool valid positive 5 scene.entries [] ∅ 0 next
          if result.interrupted then result else runScenes context budget pool valid positive rest result.machine

/-- None selects a different numeric solver. A completed empty result is
returned for the same zero-capacity and short-pool guards as the dispatcher. -/
noncomputable def search (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool) : Option Outcome :=
  if context.capacity = 0 then some (complete initial) else
    if short : pool.cards.length < 5 then some (complete initial) else
      if enabled (dispatch context) then
        some (runScenes context budget pool valid (by omega)
          (sortedScenes (SceneTable.build pool valid context.mode)) initial)
      else none

structure SelectionValid (origin entries selected : List Entry)
    (used : Finset (Fin 27)) (slots sum : Nat) : Prop where
  size : selected.length + slots = 5
  used_exact : used = (selected.map Entry.character).toFinset
  sum_exact : sum = (selected.map Entry.power).sum
  characters : (selected.map Entry.character).Nodup
  public_ids : (selected.map Entry.publicId).Nodup
  selected_source : ∀ item ∈ selected, item ∈ origin
  suffix : entries.Sublist origin

theorem selection_root (origin : List Entry) :
    SelectionValid origin origin [] ∅ 5 0 := by
  constructor <;> simp

theorem selection_skip (origin rest selected : List Entry) (item : Entry)
    (used : Finset (Fin 27)) (slots sum : Nat)
    (state : SelectionValid origin (item :: rest) selected used slots sum) :
    SelectionValid origin rest selected used slots sum := by
  exact { state with suffix := (List.sublist_cons_self item rest).trans state.suffix }

theorem selection_choose (origin rest selected : List Entry) (item : Entry)
    (used : Finset (Fin 27)) (slots sum : Nat)
    (state : SelectionValid origin (item :: rest) selected used (slots + 1) sum)
    (fresh : ¬ (item.character ∈ used ∨ item.publicId ∈ selected.map Entry.publicId)) :
    SelectionValid origin rest (selected ++ [item]) (insert item.character used)
      slots (sum + item.power) := by
  have charFresh : item.character ∉ selected.map Entry.character := by
    simpa [state.used_exact] using (not_or.mp fresh).1
  have idFresh := (not_or.mp fresh).2
  constructor
  · simp only [List.length_append, List.length_singleton]
    have size := state.size
    omega
  · rw [state.used_exact]
    ext character
    simp only [Finset.mem_insert, List.mem_toFinset, List.map_append,
      List.map_singleton, List.mem_append, List.mem_singleton]
    tauto
  · simp [List.map_append, state.sum_exact]
  · simp only [List.map_append, List.map_singleton, List.nodup_append,
      List.nodup_singleton, state.characters, true_and]
    intro character member other same
    have equal : other = item.character := by simpa using same
    subst other
    exact fun equal => charFresh (equal ▸ member)
  · simp only [List.map_append, List.map_singleton, List.nodup_append,
      List.nodup_singleton, state.public_ids, true_and]
    intro id member other same
    have equal : other = item.publicId := by simpa using same
    subst other
    exact fun equal => idFresh (equal ▸ member)
  · intro entry member
    rcases List.mem_append.mp member with old | added
    · exact state.selected_source entry old
    · have equal : entry = item := by simpa using added
      subst entry
      exact state.suffix.subset (List.mem_cons_self ..)
  · exact (List.sublist_cons_self item rest).trans state.suffix

def LeafProduced (context : Context) (pool : Admission.GatherPipeline.Pool)
    (valid : Admission.GatherPipeline.Valid pool) (positive : 0 < pool.cards.length)
    (origin : List Entry) (key : Canonical.Key) : Prop :=
  ∃ selected : List Entry,
    selected.length = 5 ∧ (selected.map Entry.character).Nodup ∧
    (selected.map Entry.publicId).Nodup ∧ (∀ item ∈ selected, item ∈ origin) ∧
    key = LeafEvaluation.resultKey pool valid context.mode context.numeric.honor.val
      (context.numeric.powerCap.map Fin.val) (selected.map (SceneTable.decode pool positive))

def ObservedWithin (allowed : Canonical.Key → Prop) (machine : Machine) : Prop :=
  ∀ key ∈ machine.tracker.observed, allowed key

theorem loop_preserves_observed (context : Context) (budget : Budget)
    (child : Continuation) (slots : Nat) (origin : List Entry) (allowed : Canonical.Key → Prop)
    (childSafe : ∀ entries selected used sum machine,
      SelectionValid origin entries selected used slots sum → ObservedWithin allowed machine →
      ObservedWithin allowed (child entries selected used sum machine).machine)
    (entries selected : List Entry) (used : Finset (Fin 27)) (sum : Nat) (machine : Machine)
    (state : SelectionValid origin entries selected used (slots + 1) sum)
    (observed : ObservedWithin allowed machine) :
    ObservedWithin allowed (loop context budget child (slots + 1) entries selected used sum machine).machine := by
  induction entries generalizing machine with
  | nil => exact observed
  | cons item rest ih =>
      rw [loop]
      dsimp only
      by_cases expired : budget .sampled machine.observations = true
      · simp only [if_pos expired]
        exact observed
      · simp only [if_neg expired]
        by_cases cut : belowCutoff context (tick machine) (sum + item.power * (slots + 1)) = true
        · simp only [if_pos cut]
          exact observed
        · simp only [if_neg cut]
          have skipped := selection_skip origin rest selected item used (slots + 1) sum state
          by_cases conflict : item.character ∈ used ∨ item.publicId ∈ selected.map Entry.publicId
          · simp only [if_pos conflict]
            exact ih (tick machine) skipped observed
          · simp only [if_neg conflict]
            have chosen := selection_choose origin rest selected item used slots sum state conflict
            have first := childSafe rest (selected ++ [item]) (insert item.character used)
              (sum + item.power) (tick machine) chosen observed
            by_cases stopped : (child rest (selected ++ [item]) (insert item.character used)
                (sum + item.power) (tick machine)).interrupted = true
            · simp only [if_pos stopped]
              exact first
            · simp only [if_neg stopped]
              exact ih (child rest (selected ++ [item]) (insert item.character used)
                (sum + item.power) (tick machine)).machine skipped first

theorem descend_preserves_observed (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (origin : List Entry) (allowed : Canonical.Key → Prop)
    (leafAllowed : ∀ key, LeafProduced context pool valid positive origin key → allowed key)
    (slots : Nat) (entries selected : List Entry) (used : Finset (Fin 27))
    (sum : Nat) (machine : Machine)
    (state : SelectionValid origin entries selected used slots sum)
    (observed : ObservedWithin allowed machine) :
    ObservedWithin allowed (descend context budget pool valid positive slots entries selected used sum machine).machine := by
  induction slots generalizing entries selected used sum machine with
  | zero =>
      intro key member
      change key ∈ insert (LeafEvaluation.resultKey pool valid context.mode context.numeric.honor.val
        (context.numeric.powerCap.map Fin.val) (selected.map (SceneTable.decode pool positive)))
        machine.tracker.observed at member
      rcases Finset.mem_insert.mp member with same | old
      · apply leafAllowed key
        exact ⟨selected, by simpa using state.size, state.characters, state.public_ids,
          state.selected_source, same⟩
      · exact observed key old
  | succ slots ih =>
      rw [descend]
      dsimp only
      cases completion : bestCompletion entries 0 used (slots + 1) with
      | none => exact observed
      | some result =>
          rcases result with ⟨bound, least⟩
          dsimp only
          split
          · exact observed
          · exact loop_preserves_observed context budget _ slots origin allowed ih
              entries selected used sum machine state observed

theorem descend_observed_origin (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (origin : List Entry) (slots : Nat)
    (entries selected : List Entry) (used : Finset (Fin 27)) (sum : Nat) (machine : Machine)
    (state : SelectionValid origin entries selected used slots sum) :
    ∀ key ∈ (descend context budget pool valid positive slots entries selected used sum machine).machine.tracker.observed,
      key ∈ machine.tracker.observed ∨ LeafProduced context pool valid positive origin key := by
  exact descend_preserves_observed context budget pool valid positive origin
    (fun key => key ∈ machine.tracker.observed ∨ LeafProduced context pool valid positive origin key)
    (fun _ produced => Or.inr produced) slots entries selected used sum machine state
    (fun _ member => Or.inl member)

def Evolves (capacity : Nat) (before after : Machine) : Prop :=
  BoundedTracker.WellFormed capacity after.tracker ∧ before.tracker.observed ⊆ after.tracker.observed

theorem loop_evolves (context : Context) (budget : Budget) (child : Continuation) (slots : Nat)
    (childSafe : ∀ entries selected used sum machine, BoundedTracker.WellFormed context.capacity machine.tracker →
      Evolves context.capacity machine (child entries selected used sum machine).machine)
    (entries selected : List Entry) (used : Finset (Fin 27)) (sum : Nat) (machine : Machine)
    (wellFormed : BoundedTracker.WellFormed context.capacity machine.tracker) :
    Evolves context.capacity machine (loop context budget child slots entries selected used sum machine).machine := by
  induction entries generalizing machine with
  | nil => exact ⟨wellFormed, Finset.Subset.refl _⟩
  | cons item rest ih =>
      rw [loop]
      dsimp only
      by_cases expired : budget .sampled machine.observations = true
      · simp only [if_pos expired]
        exact ⟨wellFormed, Finset.Subset.refl _⟩
      · simp only [if_neg expired]
        by_cases cut : belowCutoff context (tick machine) (sum + item.power * slots) = true
        · simp only [if_pos cut]
          exact ⟨wellFormed, Finset.Subset.refl _⟩
        · simp only [if_neg cut]
          by_cases conflict : item.character ∈ used ∨ item.publicId ∈ selected.map Entry.publicId
          · simp only [if_pos conflict]
            exact ih (tick machine) wellFormed
          · simp only [if_neg conflict]
            have first := childSafe rest (selected ++ [item]) (Insert.insert item.character used) (sum + item.power)
              (tick machine) wellFormed
            by_cases stopped : (child rest (selected ++ [item]) (Insert.insert item.character used) (sum + item.power)
                (tick machine)).interrupted = true
            · simp only [if_pos stopped]
              exact first
            · simp only [if_neg stopped]
              have later := ih (child rest (selected ++ [item]) (Insert.insert item.character used) (sum + item.power)
                (tick machine)).machine first.1
              exact ⟨later.1, first.2.trans later.2⟩

theorem descend_evolves (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (slots : Nat)
    (entries selected : List Entry) (used : Finset (Fin 27)) (sum : Nat) (machine : Machine)
    (wellFormed : BoundedTracker.WellFormed context.capacity machine.tracker) :
    Evolves context.capacity machine (descend context budget pool valid positive slots entries selected used sum machine).machine := by
  induction slots generalizing entries selected used sum machine with
  | zero =>
      exact ⟨BoundedTracker.step_well_formed _ _ _ wellFormed, Finset.subset_insert _ _⟩
  | succ slots ih =>
      rw [descend]
      dsimp only
      cases completion : bestCompletion entries 0 used (slots + 1) with
      | none => exact ⟨wellFormed, Finset.Subset.refl _⟩
      | some result =>
          rcases result with ⟨bound, least⟩
          dsimp only
          split
          · exact ⟨wellFormed, Finset.Subset.refl _⟩
          · exact loop_evolves context budget _ (slots + 1) ih entries selected used sum machine wellFormed

theorem scenes_evolve (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool)
    (positive : 0 < pool.cards.length) (scenes : List SceneTable.Scenario) (machine : Machine)
    (wellFormed : BoundedTracker.WellFormed context.capacity machine.tracker) :
    Evolves context.capacity machine (runScenes context budget pool valid positive scenes machine).machine := by
  induction scenes generalizing machine with
  | nil => exact ⟨wellFormed, Finset.Subset.refl _⟩
  | cons scene rest ih =>
      rw [runScenes]
      dsimp only
      by_cases expired : budget .immediate machine.observations = true
      · simp only [if_pos expired]
        exact ⟨wellFormed, Finset.Subset.refl _⟩
      · simp only [if_neg expired]
        by_cases cut : belowCutoff context (tick machine) scene.ceiling = true
        · simp only [if_pos cut]
          exact ih (tick machine) wellFormed
        · simp only [if_neg cut]
          have first := descend_evolves context budget pool valid positive 5 scene.entries [] ∅ 0 (tick machine) wellFormed
          by_cases stopped : (descend context budget pool valid positive 5 scene.entries [] ∅ 0 (tick machine)).interrupted = true
          · simp only [if_pos stopped]
            exact first
          · simp only [if_neg stopped]
            have later := ih (descend context budget pool valid positive 5 scene.entries [] ∅ 0 (tick machine)).machine first.1
            exact ⟨later.1, first.2.trans later.2⟩

theorem search_well_formed (context : Context) (budget : Budget)
    (pool : Admission.GatherPipeline.Pool) (valid : Admission.GatherPipeline.Valid pool) (result : Outcome)
    (returned : search context budget pool valid = some result) :
    BoundedTracker.WellFormed context.capacity result.machine.tracker := by
  unfold search at returned
  split at returned
  · have same := Option.some.inj returned
    subst result
    exact BoundedTracker.initial_well_formed _
  · split at returned
    · have same := Option.some.inj returned
      subst result
      exact BoundedTracker.initial_well_formed _
    · split at returned
      · have same := Option.some.inj returned
        subst result
        exact (scenes_evolve context budget pool valid _ _ initial (BoundedTracker.initial_well_formed _)).1
      · contradiction

end SourceDFS

end Allium.PowerScenarios
