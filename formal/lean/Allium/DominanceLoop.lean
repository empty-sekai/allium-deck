import Allium.RootForest
import Allium.PowerDominance

/-!
# Checked power substitutions and retained roots

Each accepted deletion checks the concrete public/dense ordering, character,
fixed public IDs, and all legacy/mixed power states. The parent forest and
its power and character invariants are constructed by that transition. They
are not supplied as an objective-soundness premise.

These are the power and identity projections of dominance. Skill, event bonus,
support compensation, placement, and complete result-key ordering are separate
obligations. An arbitrary schedule below means any finite sequence of these
checked transitions, not an unverified arbitrary replacement relation.
-/
namespace Allium.DominanceLoop
open RootForest PowerDominance

structure State (count : Nat) where
  forest : Forest count
  cards : Fin count → MixedPower.Card
  character : Fin count → Nat
  fixedIds : Finset Nat
  parentCharacter : ∀ card, forest.keep card = false →
    character (forest.parent card) = character card
  parentPower : ∀ card, forest.keep card = false →
    Dominates (cards card) (cards (forest.parent card))
  fixedIdsKept : ∀ card, forest.publicId card ∈ fixedIds → forest.keep card = true

variable {count : Nat}

def initial (publicId character : Fin count → Nat) (cards : Fin count → MixedPower.Card)
    (fixedIds : Finset Nat) : State count :=
  { forest :=
      { publicId := publicId
        keep := fun _ => true
        parent := id
        guarded := by intro _ impossible; cases impossible }
    cards := cards
    character := character
    fixedIds := fixedIds
    parentCharacter := by intro _ impossible; cases impossible
    parentPower := by intro _ impossible; cases impossible
    fixedIdsKept := by intro _ _; rfl }

structure Accept (state : State count) (winner loser : Fin count) : Prop where
  winnerKept : state.forest.keep winner = true
  loserKept : state.forest.keep loser = true
  order : ParentGuard state.forest.publicId winner loser
  sameCharacter : state.character winner = state.character loser
  power : Dominates (state.cards loser) (state.cards winner)
  unfixedIds : state.forest.publicId loser ∉ state.fixedIds

/-- A deletion constructs every parent invariant from the checked dimensions.
A rejected pair leaves the complete state unchanged. -/
noncomputable def step (state : State count) (winner loser : Fin count) : State count := by
  classical
  exact if accepted : Accept state winner loser then
    { forest :=
        { publicId := state.forest.publicId
          keep := Function.update state.forest.keep loser false
          parent := Function.update state.forest.parent loser winner
          guarded := by
            intro card removed
            by_cases same : card = loser
            · subst card
              simpa only [Function.update_self] using accepted.order
            · simp only [Function.update_of_ne same] at removed ⊢
              exact state.forest.guarded card removed }
      cards := state.cards
      character := state.character
      fixedIds := state.fixedIds
      parentCharacter := by
        intro card removed
        by_cases same : card = loser
        · subst card
          simpa only [Function.update_self] using accepted.sameCharacter
        · simp only [Function.update_of_ne same] at removed ⊢
          exact state.parentCharacter card removed
      parentPower := by
        intro card removed
        by_cases same : card = loser
        · subst card
          simpa only [Function.update_self] using accepted.power
        · simp only [Function.update_of_ne same] at removed ⊢
          exact state.parentPower card removed
      fixedIdsKept := by
        intro card fixed
        have distinct : card ≠ loser := by
          intro same
          subst card
          exact accepted.unfixedIds fixed
        simp only [Function.update_of_ne distinct]
        exact state.fixedIdsKept card fixed }
  else state

@[simp] theorem step_cards (state : State count) (winner loser : Fin count) :
    (step state winner loser).cards = state.cards := by
  unfold step
  split <;> rfl

@[simp] theorem step_character (state : State count) (winner loser : Fin count) :
    (step state winner loser).character = state.character := by
  unfold step
  split <;> rfl

@[simp] theorem step_public_ids (state : State count) (winner loser : Fin count) :
    (step state winner loser).forest.publicId = state.forest.publicId := by
  unfold step
  split <;> rfl

@[simp] theorem step_fixedIds (state : State count) (winner loser : Fin count) :
    (step state winner loser).fixedIds = state.fixedIds := by
  unfold step
  split <;> rfl

noncomputable def scan : State count → List (Fin count × Fin count) → State count
  | state, [] => state
  | state, pair :: rest => scan (step state pair.1 pair.2) rest

@[simp] theorem scan_cards (state : State count) (schedule : List (Fin count × Fin count)) :
    (scan state schedule).cards = state.cards := by
  induction schedule generalizing state with
  | nil => rfl
  | cons pair rest ih => simp only [scan, ih, step_cards]

@[simp] theorem scan_character (state : State count) (schedule : List (Fin count × Fin count)) :
    (scan state schedule).character = state.character := by
  induction schedule generalizing state with
  | nil => rfl
  | cons pair rest ih => simp only [scan, ih, step_character]

@[simp] theorem scan_public_ids (state : State count) (schedule : List (Fin count × Fin count)) :
    (scan state schedule).forest.publicId = state.forest.publicId := by
  induction schedule generalizing state with
  | nil => rfl
  | cons pair rest ih => simp only [scan, ih, step_public_ids]

/-- Following an arbitrary number of checked parent edges preserves the
character, including edges to cards that were removed by later transitions. -/
theorem root_character (state : State count) (card : Fin count) :
    state.character (root state.forest card) = state.character card := by
  induction card using (measure (rank state.forest.publicId)).wf.induction with
  | h card ih =>
      rw [root]
      split
      next => rfl
      next removed =>
        have deleted := Bool.eq_false_iff.mpr removed
        exact (ih (state.forest.parent card)
          (parent_rank_lt _ _ _ (state.forest.guarded card deleted))).trans
          (state.parentCharacter card deleted)

theorem root_power (state : State count) (card : Fin count) :
    Dominates (state.cards card) (state.cards (root state.forest card)) := by
  induction card using (measure (rank state.forest.publicId)).wf.induction with
  | h card ih =>
      rw [root]
      split
      next => exact dominates_refl _
      next removed =>
        have deleted := Bool.eq_false_iff.mpr removed
        exact dominates_trans _ _ _ (state.parentPower card deleted)
          (ih (state.forest.parent card)
            (parent_rank_lt _ _ _ (state.forest.guarded card deleted)))

theorem fixedIds_root (state : State count) (card : Fin count)
    (fixed : state.forest.publicId card ∈ state.fixedIds) : root state.forest card = card :=
  root_of_kept state.forest card (state.fixedIdsKept card fixed)

theorem deck_root_related (state : State count) (deck : List (Fin count)) :
    List.Forall₂ Dominates (deck.map state.cards)
      ((deck.map (root state.forest)).map state.cards) := by
  induction deck with
  | nil => exact List.Forall₂.nil
  | cons card rest ih => exact List.Forall₂.cons (root_power state card) ih

/-- The effective selector is evaluated on each complete deck, not on the old
composition of a substituted slot. Character facts and all activation flags
are preserved through every parent edge. -/
theorem rooted_power_monotone (state : State count) (deck : List (Fin count))
    (mode : MixedPower.Mode) (honor : Nat) (cap : Option Nat) :
    PowerModel.clamp cap (MixedPower.total mode id (deck.map state.cards) + honor) ≤
      PowerModel.clamp cap
        (MixedPower.total mode id ((deck.map (root state.forest)).map state.cards) + honor) :=
  capped_objective_mono mode honor cap _ _ (deck_root_related state deck)

/-- Starting with all cards retained, any finite sequence of checked deletions
produces the power-monotone root deck. No parent-power invariant is an input. -/
theorem checked_scan_power (publicId character : Fin count → Nat)
    (cards : Fin count → MixedPower.Card) (fixedIds : Finset Nat)
    (schedule : List (Fin count × Fin count)) (deck : List (Fin count))
    (mode : MixedPower.Mode) (honor : Nat) (cap : Option Nat) :
    let state := scan (initial publicId character cards fixedIds) schedule
    PowerModel.clamp cap (MixedPower.total mode id (deck.map cards) + honor) ≤
      PowerModel.clamp cap
        (MixedPower.total mode id ((deck.map (root state.forest)).map cards) + honor) := by
  dsimp only
  have bound := rooted_power_monotone (scan (initial publicId character cards fixedIds) schedule)
    deck mode honor cap
  simpa only [scan_cards, initial] using bound

end Allium.DominanceLoop
