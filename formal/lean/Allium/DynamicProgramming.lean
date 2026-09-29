import Allium.FiniteBounds

/-!
# Group choices, reachability and componentwise-max DP compression

A group offers exactly the choices listed in its finite set. Optional groups
explicitly include None; mandatory groups do not. A state key is preserved
exactly while independent feature maxima may come from different paths.

Source: bonus_tiers.rs key/count tables; Challenge bound-state frontier;
Final attribute-mask DP. Mapping a particular solver's groups into this model
is a separate obligation, not an implicit completeness assumption.
-/
namespace Allium.DP

variable {Choice State : Type*} [DecidableEq Choice] [DecidableEq State]

def assignments : List (Finset Choice) → Finset (List Choice)
  | [] => {[]}
  | options :: rest => options.biUnion (fun choice => (assignments rest).image (choice :: ·))

/-- Full Cartesian choice enumeration, preserving group/role positions. -/
theorem mem_assignments (groups : List (Finset Choice)) (path : List Choice) :
    path ∈ assignments groups ↔
      List.Forall₂ (fun options choice => choice ∈ options) groups path := by
  induction groups generalizing path with
  | nil => simp [assignments]
  | cons options rest ih =>
      constructor
      · intro h
        rcases Finset.mem_biUnion.mp h with ⟨choice, hc, ht⟩
        rcases Finset.mem_image.mp ht with ⟨tail, hm, heq⟩
        subst path
        exact List.Forall₂.cons hc ((ih tail).mp hm)
      · intro h
        cases h with
        | cons hc ht =>
            exact Finset.mem_biUnion.mpr ⟨_, hc, Finset.mem_image.mpr ⟨_, ih _ |>.mpr ht, rfl⟩⟩

def optional (cards : Finset Choice) : Finset (Option Choice) := insert none (cards.image some)
def mandatory (cards : Finset Choice) : Finset (Option Choice) := cards.image some

@[simp] theorem optional_skip (cards : Finset Choice) : none ∈ optional cards := by
  simp [optional]

@[simp] theorem mandatory_no_skip (cards : Finset Choice) : none ∉ mandatory cards := by
  simp [mandatory]

@[simp] theorem optional_take (cards : Finset Choice) (card : Choice) :
    some card ∈ optional cards ↔ card ∈ cards := by simp [optional]

@[simp] theorem mandatory_take (cards : Finset Choice) (card : Choice) :
    some card ∈ mandatory cards ↔ card ∈ cards := by simp [mandatory]

def step (update : State → Choice → State) (states : Finset State)
    (options : Finset Choice) : Finset State :=
  states.biUnion (fun state => options.image (update state))

def reachable (update : State → Choice → State) :
    List (Finset Choice) → Finset State → Finset State
  | [], states => states
  | options :: rest, states => reachable update rest (step update states options)

/-- The DP recurrence is exactly the fold of all legal group choices. -/
theorem mem_reachable (update : State → Choice → State) (groups : List (Finset Choice))
    (states : Finset State) (result : State) :
    result ∈ reachable update groups states ↔
      ∃ initial ∈ states, ∃ path ∈ assignments groups, path.foldl update initial = result := by
  induction groups generalizing states with
  | nil => simp [reachable, assignments]
  | cons options rest ih =>
      rw [reachable, ih]
      constructor
      · rintro ⟨middle, hm, tail, ht, heval⟩
        rcases Finset.mem_biUnion.mp hm with ⟨initial, hi, hc⟩
        rcases Finset.mem_image.mp hc with ⟨choice, hchoice, heq⟩
        subst middle
        exact ⟨initial, hi, choice :: tail,
          Finset.mem_biUnion.mpr ⟨choice, hchoice, Finset.mem_image.mpr ⟨tail, ht, rfl⟩⟩,
          heval⟩
      · rintro ⟨initial, hi, path, hp, heval⟩
        rcases Finset.mem_biUnion.mp hp with ⟨choice, hc, htail⟩
        rcases Finset.mem_image.mp htail with ⟨tail, ht, heq⟩
        subst path
        exact ⟨update initial choice,
          Finset.mem_biUnion.mpr ⟨initial, hi, Finset.mem_image.mpr ⟨choice, hc, rfl⟩⟩,
          tail, ht, heval⟩

/-- Reachability is not replaced by the all-zero feature vector. -/
theorem unreachable_excludes_paths (update : State → Choice → State)
    (groups : List (Finset Choice)) (states : Finset State)
    (accept : State → Prop) [DecidablePred accept]
    (hempty : (reachable update groups states).filter accept = ∅) :
    ∀ initial ∈ states, ∀ path ∈ assignments groups,
      ¬ accept (path.foldl update initial) := by
  intro initial hi path hp ha
  have hmem : path.foldl update initial ∈ reachable update groups states :=
    (mem_reachable update groups states _).mpr ⟨initial, hi, path, hp, rfl⟩
  have hbad := Finset.mem_filter.mpr ⟨hmem, ha⟩
  rw [hempty] at hbad
  exact Finset.notMem_empty _ hbad

section Compression
variable {Key : Type*} [DecidableEq Key] {n : ℕ}

abbrev Features (n : ℕ) := Fin n → ℕ
abbrev Cell (Key : Type*) (n : ℕ) := Key × Features n

def CellLE (a b : Cell Key n) : Prop := a.1 = b.1 ∧ ∀ i, a.2 i ≤ b.2 i

def Covers (concrete abstract : Finset (Cell Key n)) : Prop :=
  ∀ cell ∈ concrete, ∃ upper ∈ abstract, CellLE cell upper

/-- Exact keys and independent maxima of every feature in that key's cell. -/
def compress (cells : Finset (Cell Key n)) : Finset (Cell Key n) :=
  (cells.image Prod.fst).image (fun key =>
    (key, fun i => (cells.filter (fun cell => cell.1 = key)).sup (fun cell => cell.2 i)))

theorem compression_covers (cells : Finset (Cell Key n)) : Covers cells (compress cells) := by
  intro cell hc
  refine ⟨(cell.1, fun i =>
    (cells.filter (fun other => other.1 = cell.1)).sup (fun other => other.2 i)), ?_, rfl, ?_⟩
  · exact Finset.mem_image.mpr
      ⟨cell.1, Finset.mem_image.mpr ⟨cell, hc, rfl⟩, rfl⟩
  · intro i
    exact Finset.le_sup (f := fun other : Cell Key n => other.2 i)
      (Finset.mem_filter.mpr ⟨hc, rfl⟩)

omit [DecidableEq Key] in
theorem covers_refl (cells : Finset (Cell Key n)) : Covers cells cells := by
  intro cell hc
  exact ⟨cell, hc, rfl, fun _ => le_rfl⟩

omit [DecidableEq Key] in
theorem covers_trans {a b c : Finset (Cell Key n)} (hab : Covers a b) (hbc : Covers b c) :
    Covers a c := by
  intro cell hc
  rcases hab cell hc with ⟨mid, hm, hkey, hfeat⟩
  rcases hbc mid hm with ⟨upper, hu, hkey', hfeat'⟩
  exact ⟨upper, hu, hkey.trans hkey', fun i => (hfeat i).trans (hfeat' i)⟩

omit [DecidableEq Choice] in
/-- Key updates must depend only on the exact key/choice, while feature updates
must be monotone. Additive, maximum and non-negative affine updates qualify. -/
theorem step_covers (update : Cell Key n → Choice → Cell Key n)
    (hmono : ∀ a b choice, CellLE a b → CellLE (update a choice) (update b choice))
    (options : Finset Choice) {concrete abstract : Finset (Cell Key n)}
    (hcover : Covers concrete abstract) :
    Covers (step update concrete options) (step update abstract options) := by
  intro cell hc
  rcases Finset.mem_biUnion.mp hc with ⟨prior, hp, hm⟩
  rcases Finset.mem_image.mp hm with ⟨choice, hchoice, heq⟩
  subst cell
  rcases hcover prior hp with ⟨upper, hu, hle⟩
  exact ⟨update upper choice,
    Finset.mem_biUnion.mpr ⟨upper, hu, Finset.mem_image.mpr ⟨choice, hchoice, rfl⟩⟩,
    hmono prior upper choice hle⟩

def compressedReachable (update : Cell Key n → Choice → Cell Key n) :
    List (Finset Choice) → Finset (Cell Key n) → Finset (Cell Key n)
  | [], cells => cells
  | options :: rest, cells =>
      compressedReachable update rest (compress (step update cells options))

omit [DecidableEq Choice] in
/-- Cell compression after EVERY group cannot lose an actual state's key or
underestimate any of its features. Correlation loss is only an overestimate. -/
theorem compressed_reachable_covers (update : Cell Key n → Choice → Cell Key n)
    (hmono : ∀ a b choice, CellLE a b → CellLE (update a choice) (update b choice))
    (groups : List (Finset Choice)) (concrete abstract : Finset (Cell Key n))
    (hcover : Covers concrete abstract) :
    Covers (reachable update groups concrete) (compressedReachable update groups abstract) := by
  induction groups generalizing concrete abstract with
  | nil => exact hcover
  | cons options rest ih =>
      exact ih (step update concrete options) (compress (step update abstract options))
        (covers_trans (step_covers update hmono options hcover) (compression_covers _))

omit [DecidableEq Key] in
/-- A queried interval/class of keys includes every actual hit in that class. -/
theorem query_bound (concrete abstract : Finset (Cell Key n)) (hcover : Covers concrete abstract)
    (accept : Key → Prop) [DecidablePred accept]
    (objective : Features n → ℕ) (hmono : Monotone objective)
    (cell : Cell Key n) (hc : cell ∈ concrete) (ha : accept cell.1) :
    objective cell.2 ≤ (abstract.filter (fun c => accept c.1)).sup (fun c => objective c.2) := by
  rcases hcover cell hc with ⟨upper, hu, hk, hf⟩
  exact (hmono hf).trans (Finset.le_sup
    (s := abstract.filter (fun c => accept c.1)) (f := fun c : Cell Key n => objective c.2)
    (Finset.mem_filter.mpr ⟨hu, by simpa only [← hk] using ha⟩))

omit [DecidableEq Key] in
theorem query_empty (concrete abstract : Finset (Cell Key n)) (hcover : Covers concrete abstract)
    (accept : Key → Prop) [DecidablePred accept]
    (hempty : abstract.filter (fun c => accept c.1) = ∅) :
    ∀ cell ∈ concrete, ¬ accept cell.1 := by
  intro cell hc ha
  rcases hcover cell hc with ⟨upper, hu, hk, _⟩
  have hm : upper ∈ abstract.filter (fun c => accept c.1) :=
    Finset.mem_filter.mpr ⟨hu, by simpa only [← hk] using ha⟩
  rw [hempty] at hm
  exact Finset.notMem_empty _ hm

end Compression
end Allium.DP
