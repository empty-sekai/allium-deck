# allium-deck-server

English | [简体中文](./README.md)

An HTTP service around the [`allium-deck`](..) recommendation engine.

The engine is a pure computation library. This crate adds what a network service needs
around it: masterdata resident in memory, a bounded pool of search threads, per-request
ceilings, and metrics that show where each request spent its time.

It is not published to crates.io. Build it from this repository, or run the container.

## Scope

This is a straightforward implementation. It makes the engine reachable over HTTP and
gives it the rails a shared service needs, but it is not a tuned architecture and does
not try to be a complete deployment. What it deliberately leaves out:

- **No TLS, no authentication, no per-caller quotas.** Only `/admin/reload` is guarded,
  by a static token. Put a reverse proxy or gateway in front of it for anything else.
- **No caching.** Every request rebuilds the candidate pool and runs the search again,
  even for inputs it has just seen.
- **No reuse of parsed player data.** A collection is parsed from the request body every
  time. On a large collection that parse, not the search, is the bulk of a routine
  request — see the measurements in [`docker/README.en.md`](../docker/README.en.md).
- **No cancellation.** A client that disconnects does not stop the search; the thread
  finishes the work and discards it. The per-request deadline is the only bound.
- **One search per request, single-threaded.** Requests run concurrently with each
  other, but a single search never spreads across cores.
- **Single process.** Nothing coordinates across instances; each one holds its own copy
  of masterdata and its own queue.

Each of these is a reasonable thing to add. None of them is here.

## Quick start

The repository carries no game data. To try the service without any, export the
deterministic synthetic set the benchmarks use — 26 characters, 1300 cards, a fully
levelled account:

```bash
cargo run --manifest-path server/Cargo.toml --release --bin export-synth-masterdata -- ./synth

cd server
cargo run --release -- \
  --masterdata synth=../synth/masterdata \
  --music-metas synth=../synth/music_metas.json
```

Then ask for a deck. `user` takes the player's collection either as an object or as its
JSON text, and `params` is the engine's own parameter contract, documented in
[`docs/parameters.md`](../docs/parameters.md):

```bash
curl localhost:8080/v1/recommend -H 'content-type: application/json' -d "{
  \"user\": $(cat ../synth/user.json),
  \"params\": {\"liveType\": \"multi\", \"target\": \"score\", \"eventId\": 1, \"limit\": 5}
}"
```

```json
{
  "region": "synth",
  "decks": [
    {
      "rank": 1,
      "targetValue": 3659312905881,
      "cards": [{ "cardId": 80, "powerTotal": 35810, "eventBonus": 25.0, "skillScoreUp": 80.0 }],
      "totalPower": 175929,
      "liveScore": 769689,
      "eventPoint": 852
    }
  ],
  "diagnostics": { "poolSize": 156, "effectiveLiveType": "multi", "leafNodes": 145 },
  "timing": { "queueWaitMs": 0.03, "buildPoolMs": 1.12, "searchMs": 0.38, "totalMs": 10.49 },
  "timedOut": false
}
```

With real data, point `--masterdata` at a directory of flat masterdata `*.json` tables
and `--music-metas` at a `music_metas.json`. Sources for both are listed in
[`src/bin/recommend_cli.rs`](../src/bin/recommend_cli.rs).

## Endpoints

| Method | Path | What it does |
| --- | --- | --- |
| POST | `/v1/recommend` | Build decks. Every target and live type, including World Bloom chapters and the final chapter. |
| POST | `/v1/recommend/challenge-all` | The best challenge deck for each of the 26 characters, ranked. |
| POST | `/v1/world-bloom/support-cards` | Per-card support bonus for a World Bloom chapter. |
| POST | `/v1/music/recommend` | Rank every song and difficulty for an already-chosen deck. |
| POST | `/v1/live/exact-score` | Walk a chart note by note for a given power and skill set. |
| POST | `/v1/area-items/recommend` | Rank area item upgrades by power gained per coin. |
| GET | `/v1/regions` | Loaded regions, their table counts, and the effective ceilings. |
| POST | `/admin/reload` | Re-read masterdata from disk. Needs `--admin-token`. |
| GET | `/healthz` `/readyz` | Liveness and readiness. |
| GET | `/metrics` | Prometheus text exposition. |
| GET | `/openapi.json` | The full request and response schemas. |

Requests name a region with `"region"`; omitting it uses the default region, which is
the first `--masterdata` given unless `--default-region` says otherwise.

## Configuration

Every flag `--some-name` also reads `ALLIUM_DECK_SOME_NAME`, and the flag wins.

| Flag | Default | Notes |
| --- | --- | --- |
| `--masterdata <region>=<dir>` | required | Repeatable. Comma-separated in the environment variable. |
| `--music-metas <region>=<file>` | required | Repeatable. One per region. |
| `--bind <addr>` | `0.0.0.0:8080` | |
| `--default-region <name>` | first `--masterdata` | Used when a request omits `region`. |
| `--workers <n>` | available parallelism | Search threads. |
| `--max-queue <n>` | `workers * 8` | Queued requests before 503. |
| `--queue-timeout-ms <ms>` | `1000` | How long a request may wait unclaimed before 504. |
| `--max-search-timeout-ms <ms>` | `2000` | Ceiling for a request's `timeoutMs`. |
| `--max-limit <n>` | `30` | Ceiling for a request's `limit`. |
| `--max-body-bytes <n>` | `8388608` | Request body cap. |
| `--admin-token <token>` | unset | Enables `POST /admin/reload`. |
| `--log-format <text\|json>` | `text` | `ALLIUM_DECK_LOG` sets the filter, e.g. `debug`. |

## Concurrency and backpressure

Deck search is CPU-bound, and World Bloom and final-chapter requests run for hundreds of
milliseconds. Running that on the async runtime would starve connection handling, so the
service keeps a fixed number of dedicated threads and a bounded queue in front of them:

```text
HTTP (async)  --try_send-->  queue (--max-queue)  -->  --workers search threads
                 full: 503                              one request at a time
```

- **Queue full** → `503` immediately with `Retry-After`, rather than absorbing the
  request into an unbounded backlog. A caller learns the service is saturated.
- **Waited past `--queue-timeout-ms` without starting** → `504`.
- Once a thread starts a request it runs to completion; the search enforces its own
  deadline from there.
- A panicking request becomes a `500` and the thread keeps serving. It does not shrink
  the configured concurrency.

## Request ceilings

The engine accepts `limit` up to 100 and `timeoutMs` up to 300000 — long enough for one
request to hold a thread for five minutes. The service lowers both to its own ceilings
instead of rejecting the request, so asking for more returns a smaller answer rather
than an error. `GET /v1/regions` reports the ceilings in force.

When a search reaches its deadline it returns the best decks found so far and the
response sets `"timedOut": true`. Those decks are not a proven optimum — see the
exactness matrix in [`docs/parameters.md`](../docs/parameters.md). A World Bloom final
chapter request against a large collection is the shape most likely to hit this; raise
`--max-search-timeout-ms` if your deployment would rather wait than approximate.

## Errors

Every failure returns the same shape, so a client can branch on `code`:

```json
{ "error": { "code": "overloaded", "message": "the search queue is full; retry shortly" } }
```

| Status | `code` | Meaning |
| --- | --- | --- |
| 400 | `invalid_request` | Malformed body, or parameters the engine rejected. |
| 404 | `unknown_region` | No masterdata is loaded for that region. |
| 503 | `overloaded` | The queue is full. Retryable. |
| 504 | `queue_timeout` | The request waited longer than the queue budget. |
| 500 | `internal` | A search thread failed. |

## Reloading masterdata

```bash
curl -X POST localhost:8080/admin/reload -H "authorization: Bearer $TOKEN"
```

Every configured region is re-read and the snapshot is swapped atomically. Requests
already in flight keep the snapshot they started with, and a failed reload leaves the
running one in place.

## Observability

`/metrics` carries request counts by endpoint and outcome, and histograms for the total
duration **and each stage separately** — queue wait, pool build, and search. The split is
the useful part: it says whether a slow request was queued, built, or searched, which is
also what you need to size `--workers` and `--max-queue` for your own traffic.

The same split is in every response's `timing` object.

## Container

```bash
# From the repository root: the build context includes the engine crate.
docker build -f server/Dockerfile -t allium-deck-server .

docker run --rm -p 8080:8080 -v /path/to/data:/data:ro allium-deck-server \
  --masterdata cn=/data/masterdata --music-metas cn=/data/music_metas.json
```

The libc, the allocator and the runtime base are separate build arguments. See
[`docker/README.en.md`](../docker/README.en.md) for the combinations, what they measure out at,
and how to repeat the measurement on your own traffic.

There is no shell in the runtime image, so health checking belongs to whatever runs the
container: probe `/healthz` for liveness and `/readyz` for readiness.

## Tests

```bash
cargo test --release
```

The integration tests drive the router in process against the synthetic masterdata and
require the deck sequence to match `engine::recommend` exactly, order included — the
HTTP layer is not allowed to change what the engine answers.
