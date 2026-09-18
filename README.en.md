# allium-deck

English | [简体中文](./README.md)

A Rust implementation of a Project Sekai deck recommendation engine, focused on **exact DFS / branch-and-bound (B&B) search**.

Given a player's card collection, event bonuses, and an objective (power / skill / event points / MySekai, etc.), it searches a huge combinatorial space for the optimal 5-card deck. The core data structures are organized as SoA (structure of arrays) plus bit manipulation, combined with character-aware suffix upper bounds and dominance pruning. Conventional 260-card Top-1 / Top-8 hot paths are now sub-millisecond; heavier Top-K and adversarial cases are shown in the measurements below.

## About the implementation

Some in-game values and logic (power, skill bonuses, event points, support decks, the WL3 simulated finale, etc.) come from the following open-source implementations; source comments keep the cross-references:

- https://github.com/Team-Haruki/sekai-deck-recommend-cpp
- https://github.com/StarMoe-org/sekai-deck-recommend-cpp

See the individual commit messages for exactly what was ported and corrected.

On top of that, this implementation is not a line-by-line translation: the **low-level hot paths and search pruning have been thoroughly reworked in Rust**, with all core data structures aligned to cache lines:

- **Pool building**: builds by-id indexes over masterdata once, turning per-card O(N) linear scans into O(1) lookups.
- **Search**: SoA card pool + 512-bit candidate bitmaps, power packed into u18×8 slots plus lookup tables, per-character dominance pruning, character-aware suffix upper bounds, and a greedy + 1-swap warm-start lower bound, so branch and bound prunes as early as possible.

`CardPool` uses a columnar SoA layout with each column aligned to 64 bytes. A typical candidate pool (130–260 cards) is **~7–12 KB** in total; together with search-time structures such as `SearchContext` and `SuffixBound`, the hot-path data fits in the L1 data cache of a modern server CPU (EPYC 9K85: 48 KiB L1d per core). Leaf evaluation walks the deck in column order, minimizing traffic to unrelated cache lines and TLB pressure.

## Performance

The table below is measured from the current final code on an **AMD EPYC 9K85**, release build, pinned to CPU 2 with an 80% CPU quota. Every row uses a synthetic 260-card pool. `Pool build` covers construction of `CardPool` and the search context from already-parsed user data; `search` includes dominance, bound construction, warm start, the main solver, and Top-K alternative recovery, while excluding fixture I/O and JSON parsing.

| Scenario | Top-K | Pool build | Search p50 | Search p95 |
| --- | ---: | ---: | ---: | ---: |
| balanced / Solo / no event | 1 | 0.390 ms | **0.632 ms** | 0.656 ms |
| balanced / Solo / no event | 8 | 0.359 ms | **0.747 ms** | 0.767 ms |
| balanced / Multi / no event | 8 | 0.371 ms | **0.365 ms** | 0.401 ms |
| balanced / Solo / event | 8 | 0.382 ms | **1.468 ms** | 2.057 ms |
| tradeoff / Solo / no event | 1 | 0.338 ms | **0.994 ms** | 1.012 ms |
| tradeoff / Solo / no event | 8 | 0.340 ms | **4.378 ms** | 21.329 ms |
| tradeoff / Solo / no event | 100 | 0.345 ms | **13.467 ms** | 34.460 ms |

`balanced260` represents a conventional developed collection. `tradeoff260` deliberately makes power and skill strongly anti-correlated within a character and is used to expose search-tail behavior. Real latency depends on the account, event rules, objective, and Top-K, so the table reports distributions rather than a single best run.

A separate, heavier release A/B suite shows ordinary stress-search p50 improving over the previous exact baseline by about **20.2% / 14.8% / 16.7% / 17.7%** at Top-1 / 8 / 30 / 100. Final-auto improves by about **20.6% / 17.0% / 15.2% / 7.7%** at the same K values. Every paired run that completed on both sides returned identical results, and the number of 2-second stress timeouts did not increase.

On x86-64, AVX-512F/BW is selected at runtime; unsupported CPUs and other architectures fall back to scalar code.

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

The response is `{"decks": [...], "completion": "complete", "stats": {...}}`. `cards` holds game card ids in deck order, leader first; `score` is the search ordering key. Panel details such as total power and live score are not included: build the pool with `handler::build_card_pool` and summarize results with `search::summarize_deck` (`src/bin/recommend_cli.rs` is a worked example).

The complete parameter reference is in [docs/parameters.md](docs/parameters.md). Every supported mode returns exact Top-K when the search completes; a deadline is reported explicitly as `timed_out` rather than being presented as a complete result. See [docs/exactness-proof.md](docs/exactness-proof.md) for end-to-end exactness and [docs/pruning-proof.md](docs/pruning-proof.md) for the formal proof of every search-space pruning rule.

## Module map

| Module | Responsibility |
| --- | --- |
| `engine` | Public entry points (`recommend_json` / `recommend`), masterdata loading (`OwnedGameData`), JSON parameter parsing |
| `types` | Shared identifiers and enums (`Unit` / `Attr` / `LiveType` / `ScoreTarget`), plus the per-card resolved power and skill values |
| `handler` | Pool-building layer: candidate pruning, precomputation of power / skill / event bonus, WL support deck, search context construction |
| `pool` | SoA card pool: columnar storage, bitmaps, aligned layout, read-only once frozen |
| `search` | Search layer: dominance pruning, suffix upper bounds, warm start, B&B / DP / specialized solvers dispatched by objective/scenario, exact leaf evaluation |

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
          ├─ hard filtering + proven-safe preprocessing (no quality-prefix truncation)
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
  https://github.com/empty-sekai/allium-deck/releases/download/v0.0.15/recommend_cli-v0.0.15-linux-x86_64
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
  "completion": "complete",
  "timed_out": false,
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

## HTTP server

`server/` is an HTTP service around the engine, in its own crate and not published to
crates.io. Masterdata stays resident, searches run on a fixed number of dedicated
threads, and the queue in front of them is bounded — a full queue answers 503 rather
than absorbing the request into an unbounded backlog.

It is a straightforward implementation: it makes the engine reachable over HTTP with the
rails a shared service needs, but it is **not a tuned architecture and not a complete
deployment**. There is no TLS, no authentication or quotas, no caching, no request
cancellation, no cross-instance coordination, and a single search never spreads across
cores. The full list of what is left out is in
[`server/README.en.md`](./server/README.en.md#scope).

```bash
# This repository carries no game data; export a synthetic set to try the service.
cargo run --manifest-path server/Cargo.toml --release --bin export-synth-masterdata -- ./synth

cd server
cargo run --release -- \
  --masterdata synth=../synth/masterdata \
  --music-metas synth=../synth/music_metas.json
```

```bash
curl localhost:8080/v1/recommend -H 'content-type: application/json' -d "{
  \"user\": $(cat ../synth/user.json),
  \"params\": {\"liveType\": \"multi\", \"target\": \"score\", \"eventId\": 1, \"limit\": 5}
}"
```

`params` is the engine's own parameter contract (see `docs/parameters.md`); the service
adds no dialect of its own.

| Method | Path | What it does |
| --- | --- | --- |
| POST | `/v1/recommend` | Build decks; every target and live type, including World Bloom chapters and the final chapter |
| POST | `/v1/recommend/challenge-all` | The best challenge deck for each of the 26 characters, ranked |
| POST | `/v1/world-bloom/support-cards` | Per-card support bonus for a World Bloom chapter |
| POST | `/v1/music/recommend` | Rank every song and difficulty for an already-chosen deck |
| POST | `/v1/live/exact-score` | Walk a chart note by note for a given power and skill set |
| POST | `/v1/area-items/recommend` | Rank area item upgrades by power gained per coin |
| GET | `/v1/regions` `/healthz` `/readyz` `/metrics` `/openapi.json` | Region inventory and operational endpoints |

Each response carries a `timing` split across queue, pool build and search, and
`/metrics` exposes histograms of the same stages — which is what sizing `--workers` and
`--max-queue` for your own traffic depends on.

```bash
# The build context is the repository root: the service depends on the engine by path.
docker build -f server/Dockerfile -t allium-deck-server .
docker run --rm -p 8080:8080 -v /path/to/data:/data:ro allium-deck-server   --masterdata cn=/data/masterdata --music-metas cn=/data/music_metas.json
```

Full configuration, backpressure and timeout semantics, and the error codes are in
[`server/README.en.md`](./server/README.en.md). The image's libc, allocator and runtime base
are separate build arguments, measured in [`docker/README.en.md`](./docker/README.en.md).

## Static data

`data/` embeds the 3 World Bloom support-deck bonus tables. In the reference implementations these tables ship as static repository assets and are not updated with masterdata, so they are embedded here via `include_str!` and used as a fallback when masterdata lacks the corresponding files.

## Exactness

Here, “exact” has a specific meaning: **when the search finishes with `Complete`, the returned decks are the true Top-K of the full supported feasible set under one deterministic ordering**. This applies to Score, event score, MySekai, World Bloom, Final Chapter, Challenge, Power, Skill, and exact bonus tiers.

Warm starts, beams, one-swap neighborhoods, and similar heuristics are still useful, but only to find incumbents earlier or choose visit order; they never delete a candidate that has not been ruled out by a proved bound. Unconstrained Power uses an exact 49-scenario DP, and the remaining Power / Skill cases search the full candidate set with admissible bounds.

Search deadlines are explicit. If a deadline is observed, the result is `TimedOut`: every returned deck is legal and exactly evaluated, but Top-K completeness is **not** claimed. Likewise, if hard filtering leaves more than the current 512-card representation can hold, or compact metadata cannot be encoded losslessly, the engine returns a capacity error instead of silently dropping cards.

The correctness argument is split into two documents:

- [**Exactness proof**](docs/exactness-proof.md) defines the feasible set, deterministic Top-K order, solver completeness, and the timeout / capacity boundary.
- [**Pruning proof**](docs/pruning-proof.md) proves every rule that actually removes search space and maps each proof back to the implementation. Heuristics that only reorder work or seed an incumbent are explicitly separated from pruning.

Tests are used to find counterexamples and prevent regressions, not as a substitute for those proofs. The fixed release gates currently include **3,840** independent cross-scenario comparisons (256 × 15), dedicated historical-counterexample and boundary tests, and cross-runtime validation across native, server, Rust 1.89, WASM, Node, and Chrome.

## License

[MIT](./LICENSE-MIT) OR [Apache-2.0](./LICENSE-APACHE).
