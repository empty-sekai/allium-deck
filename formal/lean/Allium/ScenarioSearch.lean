import Allium.Search
import Allium.Collection

/-!
# Search with overlapping required classes and incidental leaves

A composition pool admits more decks than belong to its semantic regime.
Requiring its bound to dominate ALL admitted leaves would be false for an
arbitrary nonmonotone power table. Here bounds cover only required leaves;
all visited leaves must still be globally legal. A shared tracker retains
coverage witnesses across scenes. Required classes cover the global space.

This proves the scenario coordinator without assuming local Top-K exactness
under an externally supplied threshold. In particular, seeds need not belong
to the current scene, witnesses are counted by public identity, and K=0 works.
-/
namespace Allium.ScenarioSearch

variable {Candidate Identity : Type*} [LinearOrder Candidate] [DecidableEq Identity]

/-- A node owes its bound to required leaves, not incidental legal leaves. -/
def RequiredSound (score : Candidate → ℕ) (required : Finset Candidate) :
    SearchTree Candidate → Prop
  | .empty => True
  | .leaf _ => True
  | .branch upper left right =>
      (∀ candidate ∈ left.leaves ∪ right.leaves,
        candidate ∈ required → score candidate ≤ upper) ∧
      RequiredSound score required left ∧ RequiredSound score required right

/-- The existing pruning algorithm remains unchanged. Only the soundness
contract is weakened to the semantic class which this scene must cover. -/
theorem required_search_spec (identity : Candidate → Identity) (score : Candidate → ℕ)
    (horder : ∀ a b, score b < score a → a < b) (k : ℕ)
    (required : Finset Candidate) (tree : SearchTree Candidate)
    (hsound : RequiredSound score required tree) (retained : Finset Candidate) :
    retained ⊆ search identity score k retained tree ∧
    search identity score k retained tree ⊆ retained ∪ tree.leaves ∧
    ∀ candidate ∈ tree.leaves, candidate ∈ required →
      Covered identity k (search identity score k retained tree) candidate := by
  classical
  induction tree generalizing retained with
  | empty =>
      simp only [search, SearchTree.leaves, Finset.union_empty]
      exact ⟨(fun _ h => h), (fun _ h => h), by simp⟩
  | leaf candidate =>
      simp only [search, SearchTree.leaves]
      refine ⟨Finset.subset_insert _ _, ?_, ?_⟩
      · intro x hx
        simpa only [Finset.mem_insert, Finset.mem_union, Finset.mem_singleton, or_comm] using hx
      · intro x hx _
        have heq : x = candidate := Finset.mem_singleton.mp hx
        subst x
        exact covered_of_mem identity k (Finset.mem_insert_self _ _)
  | branch upper left right ihLeft ihRight =>
      rcases hsound with ⟨hub, hl, hr⟩
      by_cases hprune : k ≤ (strictWitnesses identity score retained upper).card
      · simp only [search, if_pos hprune, SearchTree.leaves]
        refine ⟨(fun _ h => h), Finset.subset_union_left, ?_⟩
        intro candidate hc hrequired
        exact strict_upper_prune identity score horder k retained candidate upper
          (hub candidate hc hrequired) hprune
      · simp only [search, if_neg hprune, SearchTree.leaves]
        rcases ihLeft hl retained with ⟨hlgrow, hlvalid, hlcover⟩
        rcases ihRight hr (search identity score k retained left) with
          ⟨hrgrow, hrvalid, hrcover⟩
        refine ⟨hlgrow.trans hrgrow, ?_, ?_⟩
        · intro candidate hc
          rcases Finset.mem_union.mp (hrvalid hc) with hmid | hright
          · rcases Finset.mem_union.mp (hlvalid hmid) with hseed | hleft
            · exact Finset.mem_union_left _ hseed
            · exact Finset.mem_union_right _ (Finset.mem_union_left _ hleft)
          · exact Finset.mem_union_right _ (Finset.mem_union_right _ hright)
        · intro candidate hc hrequired
          rcases Finset.mem_union.mp hc with hleft | hright
          · exact covered_mono identity k hrgrow (hlcover candidate hleft hrequired)
          · exact hrcover candidate hright hrequired

/-- A simple finite tree with a data-derived uniform scene bound. Building
this tree is not a claim about the production DFS's runtime complexity. -/
def boundedLeaves (upper : ℕ) : List Candidate → SearchTree Candidate
  | [] => .empty
  | candidate :: rest => .branch upper (.leaf candidate) (boundedLeaves upper rest)

@[simp] theorem boundedLeaves_leaves (upper : ℕ) (candidates : List Candidate) :
    (boundedLeaves upper candidates).leaves = candidates.toFinset := by
  induction candidates with
  | nil => simp [boundedLeaves, SearchTree.leaves]
  | cons candidate rest ih => simp [boundedLeaves, SearchTree.leaves, ih]

theorem boundedLeaves_sound (score : Candidate → ℕ) (required : Finset Candidate)
    (upper : ℕ) (candidates : List Candidate)
    (hbound : ∀ candidate ∈ required, score candidate ≤ upper) :
    RequiredSound score required (boundedLeaves upper candidates) := by
  induction candidates with
  | nil => trivial
  | cons candidate rest ih =>
      exact ⟨(fun x _ hx => hbound x hx), trivial, ih⟩

structure Scene (Candidate : Type*) where
  tree : SearchTree Candidate
  required : Finset Candidate

def Scene.Valid (score : Candidate → ℕ) (legal : Finset Candidate)
    (scene : Scene Candidate) : Prop :=
  scene.required ⊆ scene.tree.leaves ∧
  scene.tree.leaves ⊆ legal ∧ RequiredSound score scene.required scene.tree

def runScenes (identity : Candidate → Identity) (score : Candidate → ℕ) (k : ℕ)
    (retained : Finset Candidate) : List (Scene Candidate) → Finset Candidate
  | [] => retained
  | scene :: rest =>
      runScenes identity score k (search identity score k retained scene.tree) rest

/-- Coverage is transferred through the shared tracker, including witnesses
from earlier scenes which are not admitted by the currently searched scene. -/
theorem runScenes_spec (identity : Candidate → Identity) (score : Candidate → ℕ)
    (horder : ∀ a b, score b < score a → a < b) (k : ℕ)
    (legal : Finset Candidate) (scenes : List (Scene Candidate))
    (hscenes : ∀ scene ∈ scenes, scene.Valid score legal)
    (retained : Finset Candidate) (hretained : retained ⊆ legal) :
    retained ⊆ runScenes identity score k retained scenes ∧
    runScenes identity score k retained scenes ⊆ legal ∧
    ∀ scene ∈ scenes, ∀ candidate ∈ scene.required,
      Covered identity k (runScenes identity score k retained scenes) candidate := by
  induction scenes generalizing retained with
  | nil => exact ⟨(fun _ h => h), hretained, by simp⟩
  | cons scene rest ih =>
      have hv := hscenes scene List.mem_cons_self
      rcases required_search_spec identity score horder k scene.required scene.tree hv.2.2 retained with
        ⟨hgrow, hvalid, hcover⟩
      have hmid : search identity score k retained scene.tree ⊆ legal := by
        intro candidate hc
        rcases Finset.mem_union.mp (hvalid hc) with hs | hl
        · exact hretained hs
        · exact hv.2.1 hl
      rcases ih (fun s hs => hscenes s (List.mem_cons_of_mem _ hs)) _ hmid with
        ⟨hrgrow, hrvalid, hrcover⟩
      refine ⟨hgrow.trans hrgrow, hrvalid, ?_⟩
      intro s hs candidate hc
      rcases List.mem_cons.mp hs with heq | hrest
      · subst s
        exact covered_mono identity k hrgrow (hcover candidate (hv.1 hc) hc)
      · exact hrcover s hrest candidate hc

/-- Exact global canonical Top-K from overlapping scenes, with no assertion
that an external threshold leaves a scene's LOCAL Top-K unchanged. -/
theorem runScenes_exact (identity : Candidate → Identity) (score : Candidate → ℕ)
    (horder : ∀ a b, score b < score a → a < b) (k : ℕ)
    (legal : Finset Candidate) (scenes : List (Scene Candidate))
    (hscenes : ∀ scene ∈ scenes, scene.Valid score legal)
    (hcover : ∀ candidate ∈ legal, ∃ scene ∈ scenes, candidate ∈ scene.required)
    (seeds : Finset Candidate) (hseeds : seeds ⊆ legal) :
    topK identity k (runScenes identity score k seeds scenes) = topK identity k legal := by
  rcases runScenes_spec identity score horder k legal scenes hscenes seeds hseeds with
    ⟨_, hvalid, hcovered⟩
  apply coverage_exactness identity k hvalid
  intro candidate hc
  obtain ⟨scene, hs, hm⟩ := hcover candidate hc
  exact hcovered scene hs candidate hm

end Allium.ScenarioSearch
