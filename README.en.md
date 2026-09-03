# allium-deck

English | [简体中文](./README.md)

A Rust implementation of a Project Sekai deck recommendation engine, focused on **exact DFS / branch-and-bound (B&B) search**.

Given a player's card collection, event bonuses, and an objective (power / skill / event points / MySekai, etc.), it searches a huge combinatorial space for the optimal 5-card deck. The core data structures are organized as SoA (structure of arrays) plus bit manipulation, combined with character-aware suffix upper bounds and dominance pruning, bringing a single search down to the sub-millisecond range.

## About the implementation

Some in-game values and logic (power, skill bonuses, event points, support decks, the WL3 simulated finale, etc.) come from the following open-source implementations; source comments keep the cross-references:

- https://github.com/Team-Haruki/sekai-deck-recommend-cpp
- https://github.com/StarMoe-org/sekai-deck-recommend-cpp

See the individual commit messages for exactly what was ported and corrected.

On top of that, this implementation is not a line-by-line translation: the **low-level hot paths and search pruning have been thoroughly reworked in Rust**, with all core data structures aligned to cache lines:

- **Pool building**: builds by-id indexes over masterdata once, turning per-card O(N) linear scans into O(1) lookups.
- **Search**: SoA card pool + 512-bit candidate bitmaps, power packed into u18×8 slots plus lookup tables, per-character dominance pruning, character-aware suffix upper bounds, and a greedy + 1-swap warm-start lower bound, so branch and bound prunes as early as possible.

`CardPool` uses a columnar SoA layout with each column aligned to 64 bytes. A typical candidate pool (130–260 cards) is **~7–12 KB** in total; together with search-time structures such as `SearchContext` and `SuffixBound`, the hot-path data fits in the L1 data cache of a modern server CPU (EPYC 9K85: 48 KiB L1d per core). Leaf evaluation walks the deck in column order, minimizing traffic to unrelated cache lines and TLB pressure.

> Performance (AMD EPYC 9K85, pinned to a single core, release profile: `opt-level=3` / `lto="fat"` / `codegen-units=1` / `target-cpu=znver5`): with masterdata resident in memory, a full pool build on cache miss (including preparation of the current user and parameters) typically takes about **0.6 ms**. Across multiplayer event Top-8 searches on 10 different accounts, the plain arithmetic mean of the per-account averages is **0.3270 ms**, with a range of **0.1017–0.8369 ms**. In this typical scenario, `v0.0.6` builds pools about **20×** faster and searches about **5–10×** faster than `v0.0.5`.
>
> Pool-build numbers exclude masterdata file reads and JSON parsing. On x86-64, AVX-512F/BW is detected at runtime; unsupported CPUs and other architectures automatically fall back to scalar code. Actual timings vary with account size, event rules, objective, and candidate pool; the 20× pool-build and 5–10× search figures describe typical multiplayer event deck building only, not a performance guarantee for every mode.

## Public API

The main entry point is `engine::recommend_json` — pure JSON in, JSON out:

```rust
use allium_deck::engine::recommend_json;

let result_json = recommend_json(
    masterdata_json,   // game masterdata
    music_metas_json,  // music metadata
    user_data_json,    // player collection (camelCase)
    params_json,       // build parameters (target / event / card_configs, ...)
)?;
```

Internally it runs two stages: `handler::build_card_pool` (pool building) → `search::search` (search). The typed entry point `engine::recommend` bypasses JSON serialization.

The complete parameter contract (all fields, defaults, value ranges) and the **per-mode exactness matrix** (which modes are exact against brute force and which are heuristic) are in [docs/parameters.md](docs/parameters.md).

## Module map

| Module | Responsibility |
| --- | --- |
| `engine` | Public entry points (`recommend_json` / `recommend`), masterdata loading (`OwnedGameData`), JSON parameter parsing |
| `types` | Shared types and enums (`Unit` / `Attr` / `LiveType` / `ScoreTarget`), power/skill lookup tables |
| `handler` | Pool-building layer: candidate pruning, precomputation of power / skill / event bonus, WL support deck, search context construction |
| `pool` | SoA card pool: columnar storage, bitmaps, aligned layout, read-only once frozen |
| `search` | Search layer: dominance pruning, suffix upper bounds, warm start, B&B dispatched by objective/scenario, exact leaf evaluation |

## Data flow

```
masterdata JSON ─┐
                 ├─→ OwnedGameData::load        // loaded once, cacheable
music_metas ─────┘        │ as_ref()
                          ▼
                    GameData (read-only borrowed view)
user JSON ──→ parse_user_profile_json ──→ UserProfile
params JSON ─→ parse_build_params_json ──→ BuildParams
                          │
                          ▼
        build_card_pool (handler)              // pool building
          ├─ per user card: precompute power / skill / event bonus
          ├─ candidate pruning (by event points / per character)
          ├─ sorted insertion into the SoA CardPool
          └─ build SearchContext (incl. WL support deck)
                          │
                          ▼  (CardPool, SearchContext)
        search (search)                        // search
          ├─ per-character dominance pruning
          ├─ character-aware suffix upper bound (core of B&B pruning)
          ├─ warm-start lower bound (greedy + 1-swap)
          └─ DFS dispatched by target / scenario:
               Power / Skill / Score / MySekai / challenge / final chapter
                          │
                          ▼
                  Vec<DeckResult> → JSON
```

## Dependencies and build

Depends only on `serde` / `serde_json` / `thiserror` — no graphics, async, or system library dependencies — and builds standalone in seconds:

```bash
cargo build --release
```

Performance numbers should be measured under the release profile. `src/bin/recommend_cli.rs` provides a standalone CLI (installed as `recommend_cli` via `cargo install allium-deck`) that prints per-stage timings (pool building vs search) for quick iteration.

## Language bindings

| Language | Location | Notes |
| --- | --- | --- |
| Rust | this repository (crates.io `allium-deck`) | the engine itself |
| JavaScript / browser | [`wasm/`](wasm) (npm `@empty-sekai/allium-deck-wasm`) | WASM bindings; see the Chinese README for the full export table |
| Python | [`allium-deck-python`](https://github.com/empty-sekai/allium-deck-python) (PyPI `allium-sekai-deck`) | prebuilt abi3 wheels with the `allium_deck` API and a LunaBot-compatible facade; no local Rust toolchain required |

## CLI

`recommend_cli` is a standalone deck recommendation command-line tool. Prebuilt binaries are available from [GitHub Releases](https://github.com/empty-sekai/allium-deck/releases), or install from source.

Run a full recommendation from the command line, printing pool-build/search timings and the Top-K decks:

```bash
# Option 1: download a prebuilt binary (linux-x86_64 shown)
curl -L -o recommend_cli \
  https://github.com/empty-sekai/allium-deck/releases/download/v0.0.12/recommend_cli-v0.0.12-linux-x86_64
chmod +x recommend_cli
./recommend_cli [OPTIONS]

# Option 2: install from git (no clone needed)
cargo install --git https://github.com/empty-sekai/allium-deck --bin recommend_cli
recommend_cli [OPTIONS]

# Option 3: clone and build locally
git clone https://github.com/empty-sekai/allium-deck.git
cd allium-deck
cargo build --release --bin recommend_cli
./target/release/recommend_cli [OPTIONS]
```

**Usage:**

```bash
recommend_cli \
  --masterdata <masterdata-dir> \
  --music-metas <music_metas.json> \
  --user <user.json> \
  --target score \
  --live-type multi \
  --event-id 170 \
  --music-id 74 \
  --music-diff expert \
  --boost 10 \
  --event-unit ln \
  --event-attr cool \
  --unit-filter ln \
  --multi-teammate-power 250000 \
  --multi-teammate-score-up 200 \
  --top-k 5
```

Flags:

| Flag | Type | Description |
| --- | --- | --- |
| `--masterdata` | directory | Game masterdata directory containing `cards.json`, `events.json`, `skills.json`, `cardRarities.json`, `gameCharacterUnits.json`, etc. |
| `--music-metas` | file | Music metadata JSON file. |
| `--user` | file | Player data JSON; must contain at least `userCards`. Area items, character ranks, honors, MySekai fields, etc. participate in scoring. |
| `--params` | file | Compatibility entry: reads recommendation parameters from a JSON file; direct flags override same-named JSON fields. |
| `--target` | enum | `score` / `power` / `skill` / `mysekai`. |
| `--live-type` | enum | `solo` / `multi` / `cheerful` / `auto` / `challenge` / `challenge_auto` / `mysekai`. |
| `--event-id` / `--music-id` / `--music-diff` | value | Event, song, and difficulty; difficulty is `easy` / `normal` / `hard` / `expert` / `master` / `append`. |
| `--boost` | int | Boost count `0..10`: `0` = no boost, `1..5` = `5/10/15/20/25x`, `6..10` = `27/29/31/33/35x`. |
| `--fixed-cards` / `--fixed-characters` / `--excluded-cards` | list | Comma-separated card ID / character ID constraints. |
| `--event-unit` / `--event-attr` | enum | Simulated event unit and attribute; units: `ln/mmj/vbs/wxs/25ji/vs`, attributes: `cool/cute/happy/pure/mysterious`. |
| `--unit-filter` / `--attr-filter` | enum | Hard filters on the candidate pool; VS dual-unit cards match the unit filter via `support_unit`. |
| `--world-bloom-character-id` / `--world-bloom-event-turn` / `--challenge-live-character-id` | value | World Bloom / Challenge Live special parameters. |
| `--skill-reference-strategy` / `--live-skill-order` / `--specific-skill-order` | value | Skill reference and activation order; a specific order is given as `0,1,2,3,4`. |
| `--multi-teammate-power` / `--multi-teammate-score-up` / `--multi-live-score-up-lower-bound` | value | Teammate power, effective skill score-up, and total skill lower bound for multi / Cheerful lives. |
| `--other-score` / `--life` | value | Cheerful opponent score and life. |
| `--rarity4-config` / `--single-card-config` | value | Card training configs, e.g. `level_max,skill_max,master_max,episode_read,canvas` and `123:level_max,skill_max`. |

Example output:

`stderr` carries only progress and timings; `stdout` always emits structured JSON, convenient for regression and performance comparison:

```text
[load] masterdata+music_metas: 135.0ms
[build_pool] 1.4ms  pool=78 effective_live=Multi
[search] 0.4ms  leaf=84 ub_prunes=278 ep_explored=18 mono_break=12
[total] 136.8ms
```

```json
{
  "effective_params": { "target": "Score", "live_type": "Multi", "boost": 10 },
  "diagnostics": { "pool_size": 78, "effective_live_type": "Multi" },
  "timing": { "build_pool_ms": 1.4, "search_ms": 0.4 },
  "decks": [
    {
      "rank": 1,
      "event_point": 1234567,
      "cards": [
        { "card_id": 111, "power_total": 35210, "skill_score_up": 120.0, "has_canvas_bonus": true, "canvas_power": 600 }
      ]
    }
  ]
}
```

## Static data

`data/` embeds the 3 World Bloom support-deck bonus tables. In the reference implementations these tables ship as static repository assets and are not updated with masterdata, so they are embedded here via `include_str!` and used as a fallback when masterdata lacks the corresponding files.

## Testing

- Unit tests: `#[cfg(test)]` modules throughout (pool / search / handler).
- Eval fixtures: `tests/fixtures/eval` pins the scoring rules — boost multipliers, multi/Cheerful, skill order, World Bloom, MySekai, etc. — with small auditable data.
- Search-result correctness is verified against **brute-force enumeration** by these unit tests:
  - `search_dfs_matches_bruteforce_for_best_deck`
  - `search_dfs_bonus_noevent_matches_bruteforce_with_suffix_max_break`
  - `search_dfs_mysekai_matches_bruteforce_with_suffix_max_break`
  - `search_suffix_bound_is_sound_and_zero_pool_is_zero`
  - `search_dominance_preserves_best_score`
- `tests/benchmark_proof.rs` cross-checks the production search against pruning-free brute-force enumeration, with three brute-force comparison tests and one corpus validation test:
  - `rust_bruteforce_matches_exact_on_full_testdata_pools` (brute-force cross-check): samples the small fixtures in this repository, builds the complete card pool from the original inputs, and compares the production search against brute-force enumeration. Only fixtures whose full-pool combination count stays within `ALLIUM_BF_CANDIDATE_LIMIT` are selected.
  - `rust_bruteforce_matches_exact_top_k_on_issue2_fixture` (Top-K regression): pins the [issue #2](https://github.com/empty-sekai/allium-deck/issues/2) fixture `real/mass_392500_score_multi_ev` and verifies the Top-K dominance-substitution expansion on the ordinary main search path. The pool has roughly 70 million combinations; run with `ALLIUM_BF_TOP_K=3 ALLIUM_BF_CANDIDATE_LIMIT=100000000`.
  - `rust_bruteforce_matches_exact_on_large_filtered_pools` (brute-force cross-check): targets large pools of highly developed accounts. It first drops 1★/2★ cards (`ALLIUM_BF_MIN_RARITY`), then keeps the per-character top N cards in each of the power / skill / event-bonus dimensions (`ALLIUM_BF_PER_CHAR_KEEP`), compressing the pool to a brute-forceable size before cross-checking. This is a stress subset covering the high-value candidate region of developed accounts, not a proof for the full large pool: pruned low-value cards may still appear in some Top-K runner-up decks.
  - `testdata_corpus_layers_are_classified` (corpus check): validates the layering and target distribution of the current fixture inventory, writing `target/benchmark-proof/report.md` plus JSON details.
- Related environment variables: `ALLIUM_BF_TOP_K`, `ALLIUM_BF_CASE_LIMIT` / `ALLIUM_BF_LARGE_CASE_LIMIT`, `ALLIUM_BF_CANDIDATE_LIMIT` / `ALLIUM_BF_LARGE_CANDIDATE_LIMIT`, `ALLIUM_BF_MIN_RARITY`, `ALLIUM_BF_PER_CHAR_KEEP`. These cross-check tests are skipped when masterdata is absent.

## Soundness

Every pruning mechanism has been verified by code audit and brute-force enumeration tests.

**Dominance pruning (`dominance.rs`) — Sound ✅**

Only compares two cards belonging to the **same character** (`pool.char_id(a) == pool.char_id(b)`). B is eliminated only when A is ≥ B in all of the following dimensions:

- power for all 8 formation combinations (compared after per-slot u18 decoding)
- skill (only comparable within the same type; Score Up compares the value, Unit Count compares the per-count bonuses for the same unit, Diff compares base and increment, Ref compares rate and max)
- event bonus (base and limited compared separately)
- same attribute (otherwise their contributions to the diff-attr bonus differ, and B cannot be declared harmless)
- unit mask is a superset (rhs_mask ⊆ lhs_mask, so no candidate formation is lost)

Substitution safety: replacing B with A never lowers the score under any objective. World Bloom events go through the same dominance pruning; the support deck is stored separately in `SearchContext`, so support candidates are not lost when the main search pool is compressed, and dominance requires equal attributes, so the diff-attr bonus cannot be broken by a different-attribute substitution.

Under Top-K (`top_k > 1`), combinations containing dominated cards may themselves be legitimate runner-up decks; pruning alone would lose ranks (formerly [issue #2](https://github.com/empty-sekai/allium-deck/issues/2)). The ordinary main search path now records a dominance map at pruning time (chains compressed to their surviving roots) and, after the search, performs a **substitution back-expansion** on every result: suppose the true Top-K contains a deck D that uses pruned cards; replacing each pruned card with its dominating root yields D', and by dominance score(D') ≥ score(D) ≥ the K-th threshold, so D' is necessarily in the exact Top-K of the pruned pool. Starting from D', substituting the dominating roots back with the pruned cards slot by slot (including multi-slot combinations), re-evaluating and merging, recovers the runner-up decks lost along this path. Back-substitution scores are monotonically non-increasing and are pruned against the current K-th threshold; `top_k = 1` skips the expansion entirely, so the main search path pays zero overhead. The final chapter has additional pruning on the member side; its Top-K substitution expansion is tracked in [issue #7](https://github.com/empty-sekai/allium-deck/issues/7).

**Suffix upper bound (`suffix.rs`) — Sound ✅**

The core of the upper bound is **character-aware aggregation**: for each of the three dimensions power / skill / bonus, take the per-character single-card maximum over the 27 characters, then sum the top-N over unused characters. Since at most one card per character can be picked, the per-dimension total of any actual formation cannot exceed the sum of that dimension's top-N per-character maxima. The three dimensions may pick different characters — a conservative overestimate that never loses a solution.

Several tightening layers sit on top:

- **Exclusion delta**: once a card is chosen, its character is excluded from the suffix and demoted to the next available character. The exclusion delta is obtained branch-free, using a compact bit index + popcount to locate it directly.
- **Dense suffix tail**: scans from the right end of the SoA leftwards, monotonically narrowing based on the characters actually present. As the DFS position advances, `ceiling(i+1) ≤ ceiling(i)`, so once it drops below the threshold the whole level can safely break.
- **World Bloom extra bound**: an extra ceiling layer over the support deck and the diff-attr cap, taking the tightest value per dimension.
- **Leaf evaluation** (`evaluate.rs`) uses the actual same-unit member count `unit_counts[unit].clamp(1, 5)` for table lookups; the 1–5-member effects are exact values, not approximations.

**Power / Skill paths — no optimality guarantee ❌**

`search_instrumented` (`mod.rs:37-38`) does not run the full B&B for the Power and Skill objectives; it enumerates within a sorted prefix instead (Power: 28 cards with ≤6 per character; Skill: 20 cards with ≤3 per character). Cards outside the prefix are simply discarded, with no upper-bound proof that this is safe — a pure performance trade-off.

The practical risk is low: Power / Skill are purely additive objectives with no cross-card skill synergy, and each character's best card is simply the one with the highest power_max / skill_max. A character having more than 4 skill cards where the optimum must use the 4th is extremely rare. But this is not a formal guarantee.

**Summary**

| Objective | Algorithm | Sound | Notes |
| --- | --- | --- | --- |
| Score / Mysekai | full B&B | ✅ | dominance pruning + character-aware suffix bound + tightening layers |
| Final chapter | character grouping + B&B | ✅ | leader × member two-stage DFS |
| Challenge | brute-force enumeration | ✅ | no pruning, only game_id dedup |
| Power / Skill | prefix DFS | ❌ | 28/20-card limit, ≤6/≤3 per character |

## License

[MIT](./LICENSE-MIT) OR [Apache-2.0](./LICENSE-APACHE).
