# Reproducible search validation

Three kinds of evidence answer different questions:

| Evidence | What it establishes | Limit |
| --- | --- | --- |
| `validation_oracle` | Complete canonical Top-K equals independently enumerated legal ordered decks | Small pools; the leaf scoring implementation is shared |
| `validation_performance` | Paired implementations agree on every returned row and proof-work counter | Both implementations could share a semantic error |
| `validation_real` | Real account data and masterdata exercise pool construction and search together | Requests and optional small card subsets are derived, not captured traffic |

An exact comparison includes every score, public card set, leader, member order
and cultivation variant. A returned `TimedOut` result is an incumbent set, not a
complete ranking. Unsupported capacity and empty feasible inputs are reported
separately. Repeating one request measures timing variability; it does not add
another input to correctness coverage.

## Exhaustive matrix

```sh
cargo test --release validation_oracle -- --nocapture
ALLIUM_VALIDATION_ORACLE_SEEDS=16 cargo test --release validation_oracle_extended -- --ignored --nocapture --test-threads=1
```

Each seed checks 288 semantic contexts and 1,152 production searches. The axes
are ordinary WL / Final Chapter, Multi / Solo, four skill orders, six constraint
shapes and K=1/8/30/100. Random, same-public-ID variants and tied boxes exercise
support exclusion, nonmonotone diversity bonuses, full Top-100 and fewer than K
feasible sets. The printed JSON includes actual enumeration and comparison
counts. Dedicated regressions cover forced-leader reconstruction and generic
Final DFS with leader-specific support.

The oracle enumerates every legal ordered assignment with its own canonical
comparison and set deduplication. It does not call production pruning, dominance,
placement or Top-K tracking. Sharing the leaf evaluator means this is a search
equivalence check, not independent validation of game formulas.

## Paired timing matrix

Build two test executables with identical Rust versions and flags. Both must
contain `validation_performance_matrix`. A performance comparison should include
the same correctness fixes on both sides, isolating the optimization being timed.

```sh
python3 scripts/compare_search_matrix.py \
  --baseline /absolute/path/to/baseline-tests \
  --candidate /absolute/path/to/candidate-tests \
  --source-manifest /absolute/path/to/frozen-source-manifest.json \
  --output /absolute/path/to/new-results-directory \
  --cpu 2 --rounds 6 --repeats 5 --timeout-ms 0
```

The default uses 24 generated pools and 288 configurations: two families, three
sizes, four deterministic seeds, three scenes and four K values. Different sizes
from one seed may share a prefix; they are not independent random draws. The
runner alternates A/B order between rounds, records warmups, hashes binaries and
raw JSONL, and compares complete ordered results plus traversal diagnostics.
Use an otherwise idle machine and retain compiler, CPU and source identities.

Reports show empirical p50/p95/p99/max, completion counts and paired ratios for
fully completed cases. Thirty repeats provide only a coarse tail estimate;
these synthetic distributions are not production P99. A separate bounded stress
run can use `--sizes 260,416,512 --timeout-ms 1000 --allow-incomplete`; its deadline rate is part of
the result and must not be hidden by reporting completed samples alone.

## External real-data matrix

```sh
ALLIUM_REAL_MASTERDATA=/absolute/path/to/masterdata \
ALLIUM_REAL_MUSIC_METAS=/absolute/path/to/music_metas.json \
ALLIUM_REAL_CORPUS=/absolute/path/to/corpus \
ALLIUM_REAL_OUTPUT=/absolute/path/to/new-output.jsonl \
ALLIUM_REAL_MODE=oracle ALLIUM_REAL_ACCOUNT_LIMIT=48 \
ALLIUM_REAL_TOPKS=1,8,30,100 ALLIUM_REAL_REPEATS=1 \
cargo test --release --test validation_real -- --ignored --nocapture --test-threads=1
```

`MODE=oracle` creates an explicitly labeled small subset and compares every
result to exhaustive enumeration. `MODE=full` retains the entire account card
collection, including pools above 512 cards; inputs exceeding the dense-index
capacity report `TooManyCards` without reducing the input. Optional
`ACCOUNT_START`, `LIVE_TYPES`, `TIMEOUT_MS`, `WL_EVENT_ID` and `FINALE_EVENT_ID`
use the `ALLIUM_REAL_` prefix. Missing data or an unexpected solver route fails
the run. Accounts with fewer than five distinct characters are recorded as not
applicable rather than counted as successful searches.

Inventory records identify actual supported events, coverage exclusions and
anonymous source ordinals. Preserve the input/masterdata hashes privately with
the run. Never publish account fixtures or identifying corpus filenames as part
of benchmark artifacts.
