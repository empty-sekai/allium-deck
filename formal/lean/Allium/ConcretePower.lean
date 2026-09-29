import Allium.Composition
import Allium.ScenarioSearch
import Allium.Enumeration

/-!
# A concrete end-to-end Power instance

Ordered five-slot legality -> all 49 regimes -> admitted pools -> arbitrary
8-entry table bounds -> character-aware / unrestricted top-five sums ->
optional cap and honor -> shared-threshold search -> canonical exhaustive Top-K.

The theorem does not take SearchTree.Sound, a score upper bound, scene coverage,
or equality with exhaustive search as a premise. Those obligations are built
and proved below. The search is a mathematical scenario-level implementation;
it does not instantiate every production DFS optimization or the other scoring
targets. Enumeration.Roles remains the explicitly stated input specification.
-/
namespace Allium.ConcretePower
open PowerModel Composition ScenarioSearch

variable {Card : Type*}

-- Keep concrete key equality aligned with the search kernel's LinearOrder.
local instance : DecidableEq Canonical.Key :=
  (inferInstance : LinearOrder Canonical.Key).toDecidableEq

structure Input (Card : Type*) where
  pool : Finset Card
  publicId : Card → ℕ
  denseId : Card → ℕ
  character : Card → ℕ
  roles : Enumeration.Roles
  uniqueCharacters : Bool
  data : Card → CardData
  honor : ℕ
  cap : Option ℕ

noncomputable def decks (input : Input Card) : Finset (List Card) :=
  Enumeration.allValidDecks input.pool input.publicId input.character input.roles input.uniqueCharacters

noncomputable def value (input : Input Card) : List Card → ℕ :=
  objective input.data input.honor input.cap

noncomputable def keyOf (input : Input Card) : List Card → Canonical.Key :=
  Canonical.ofDeck input.publicId input.denseId (value input) (value input)

noncomputable def legalKeys (input : Input Card) : Finset Canonical.Key :=
  (decks input).image (keyOf input)

noncomputable def admittedPool (input : Input Card) (r : Regime) : Finset Card := by
  classical
  exact input.pool.filter (fun card => Admits (input.data card) r)

noncomputable def planPower (input : Input Card) (r : Regime) : ℕ :=
  if input.uniqueCharacters then
    FiniteBounds.characterBound 5 (admittedPool input r) input.character
      (fun card => powerBound (input.data card) r)
  else
    FiniteBounds.maxSum 5 (admittedPool input r) (fun card => powerBound (input.data card) r)

noncomputable def ceiling (input : Input Card) (r : Regime) : ℕ :=
  clamp input.cap (planPower input r + input.honor)

theorem member_admitted (input : Input Card) (r : Regime) (deck : List Card)
    (hd : deck ∈ decks input) (hr : Matches input.data deck r) :
    ∀ card ∈ deck, card ∈ admittedPool input r := by
  classical
  have hv := (Enumeration.exhaustive_complete _ _ _ _ _ deck).mp hd
  intro card hc
  exact Finset.mem_filter.mpr
    ⟨Enumeration.valid_card_in_pool _ _ _ _ _ deck hv card hc,
      matches_admits input.data deck r hr card hc⟩

/-- This is the actual per-character versus per-card split of a regime plan,
not a maximum obtained by evaluating all completed decks. -/
theorem planPower_sound (input : Input Card) (r : Regime) (deck : List Card)
    (hd : deck ∈ decks input) (hr : Matches input.data deck r) :
    total input.data deck ≤ planPower input r := by
  classical
  have hv := (Enumeration.exhaustive_complete _ _ _ _ _ deck).mp hd
  have hlen := Enumeration.valid_length _ _ _ _ _ deck hv
  have hcards := member_admitted input r deck hd hr
  have hnd : deck.Nodup := List.Nodup.of_map input.publicId hv.2.1
  let weight := fun card => powerBound (input.data card) r
  have hsum : total input.data deck ≤ (deck.map weight).sum := deck_sum_bound input.data deck r hr
  by_cases hu : input.uniqueCharacters = true
  · have hchars : (deck.map input.character).Nodup := hv.2.2.1 hu
    simp only [planPower, hu, ↓reduceIte, FiniteBounds.characterBound]
    let perChar := FiniteBounds.characterMax (admittedPool input r) input.character weight
    calc
      total input.data deck ≤ (deck.map weight).sum := hsum
      _ ≤ (deck.map (fun card => perChar (input.character card))).sum :=
        List.sum_le_sum (fun card hc =>
          FiniteBounds.le_characterMax _ _ _ card (hcards card hc))
      _ = ((deck.map input.character).map perChar).sum := by simp only [List.map_map, Function.comp_def]
      _ = ∑ char ∈ (deck.map input.character).toFinset, perChar char :=
        (List.sum_toFinset perChar hchars).symm
      _ ≤ FiniteBounds.maxSum 5 ((admittedPool input r).image input.character) perChar := by
        apply FiniteBounds.sum_le_maxSum
        · intro char hc
          obtain ⟨card, hc, heq⟩ := List.mem_map.mp (List.mem_toFinset.mp hc)
          exact Finset.mem_image.mpr ⟨card, hcards card hc, heq⟩
        · have hn := List.toFinset_card_le (deck.map input.character)
          simpa [hlen] using hn
  · simp only [planPower, if_neg hu]
    calc
      total input.data deck ≤ (deck.map weight).sum := hsum
      _ = ∑ card ∈ deck.toFinset, weight card := (List.sum_toFinset weight hnd).symm
      _ ≤ FiniteBounds.maxSum 5 (admittedPool input r) weight := by
        apply FiniteBounds.sum_le_maxSum
        · intro card hc
          exact hcards card (List.mem_toFinset.mp hc)
        · exact (List.toFinset_card_le deck).trans hlen.le

theorem ceiling_sound (input : Input Card) (r : Regime) (deck : List Card)
    (hd : deck ∈ decks input) (hr : Matches input.data deck r) :
    value input deck ≤ ceiling input r :=
  clamp_monotone input.cap (Nat.add_le_add_right (planPower_sound input r deck hd hr) input.honor)

noncomputable def admittedDecks (input : Input Card) (r : Regime) : Finset (List Card) := by
  classical
  exact (decks input).filter (fun deck => ∀ card ∈ deck, Admits (input.data card) r)

noncomputable def requiredDecks (input : Input Card) (r : Regime) : Finset (List Card) :=
  deckClass input.data (decks input) r

noncomputable def admittedKeys (input : Input Card) (r : Regime) : Finset Canonical.Key :=
  (admittedDecks input r).image (keyOf input)

noncomputable def requiredKeys (input : Input Card) (r : Regime) : Finset Canonical.Key :=
  (requiredDecks input r).image (keyOf input)

noncomputable def scene (input : Input Card) (r : Regime) : Scene Canonical.Key :=
  ⟨boundedLeaves (ceiling input r) (admittedKeys input r).toList, requiredKeys input r⟩

/-- Incidental decks are genuinely retained as leaves. Their scores need not
satisfy the bound of this scene, but they must be legal globally. -/
theorem scene_valid (input : Input Card) (r : Regime) :
    (scene input r).Valid Canonical.score (legalKeys input) := by
  classical
  refine ⟨?_, ?_, ?_⟩
  · change requiredKeys input r ⊆ (boundedLeaves (ceiling input r) (admittedKeys input r).toList).leaves
    rw [boundedLeaves_leaves, Finset.toList_toFinset]
    apply Finset.image_subset_image
    intro deck hd
    rcases Finset.mem_filter.mp hd with ⟨hlegal, hmatch⟩
    exact Finset.mem_filter.mpr ⟨hlegal, matches_admits input.data deck r hmatch⟩
  · change (boundedLeaves (ceiling input r) (admittedKeys input r).toList).leaves ⊆ legalKeys input
    rw [boundedLeaves_leaves, Finset.toList_toFinset]
    exact Finset.image_subset_image (Finset.filter_subset _ _)
  · apply boundedLeaves_sound
    intro key hk
    obtain ⟨deck, hd, heq⟩ := Finset.mem_image.mp hk
    rcases Finset.mem_filter.mp hd with ⟨hlegal, hmatch⟩
    subst key
    exact ceiling_sound input r deck hlegal hmatch

noncomputable def scenes (input : Input Card) : List (Scene Canonical.Key) :=
  (Finset.univ : Finset Regime).toList.map (scene input)

theorem scenes_cover (input : Input Card) :
    ∀ key ∈ legalKeys input, ∃ s ∈ scenes input, key ∈ s.required := by
  classical
  intro key hk
  obtain ⟨deck, hd, heq⟩ := Finset.mem_image.mp hk
  obtain ⟨r, hr⟩ := regimes_cover input.data deck
  refine ⟨scene input r, ?_, ?_⟩
  · exact List.mem_map.mpr ⟨r, Finset.mem_toList.mpr (Finset.mem_univ r), rfl⟩
  · exact Finset.mem_image.mpr ⟨deck, Finset.mem_filter.mpr ⟨hd, hr⟩, heq⟩

/-- End-to-end instance: no abstract bound/coverage assumptions remain.
All cultivation and placement variants stay until canonical public-set Top-K. -/
theorem power_search_exact (input : Input Card) (k : ℕ) (seeds : Finset Canonical.Key)
    (hseeds : seeds ⊆ legalKeys input) :
    topK Canonical.identity k
      (runScenes Canonical.identity Canonical.score k seeds (scenes input)) =
    topK Canonical.identity k
      (Enumeration.exhaustiveKeys input.pool input.publicId input.denseId input.character
        input.roles input.uniqueCharacters (value input) (value input)) := by
  apply runScenes_exact Canonical.identity Canonical.score Canonical.score_order k
    (legalKeys input) (scenes input) _ (scenes_cover input) seeds hseeds
  intro s hs
  obtain ⟨r, _, heq⟩ := List.mem_map.mp hs
  subst s
  exact scene_valid input r

end Allium.ConcretePower
