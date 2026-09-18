# allium-deck containers

English | [简体中文](./README.md)

## Files

| File | Purpose |
|------|---------|
| `Dockerfile` | Engine verification environment: cached dependency layer, build, run the unit tests |
| `Dockerfile.wasm-ci` | WASM release builder: Rust 1.94 + wasm-pack + the Python upload SDK |
| [`../server/Dockerfile`](../server/Dockerfile) | HTTP service image, see "Service image" below |

The CLI itself lives in `src/bin/recommend_cli.rs` (`cargo install allium-deck` installs it
as `recommend_cli`); `docker/` holds no source.

---

# Verification environment

Only `serde / serde_json / thiserror`, so a clean release build takes about 30s and an
incremental one a few seconds.

## Option A: run it directly

```bash
# Unit tests, self-contained, no external data:
cargo test --lib --release

# CLI:
cargo run --bin recommend_cli --release -- \
  --masterdata <masterdata dir> \
  --music-metas <music_metas.json> \
  --user <user.json> \
  --params <params.json>
```

Output (timings on stderr, results on stdout):
```
[load] masterdata+music_metas: 120.3ms
[build_pool] 68.5ms  pool=187 candidates
[search] 4.2ms  leaf=1234 ub_prunes=...
[total] build+search = 72.7ms
 1. score=12345678     cards=[123, 456, 789, 234, 567]
 ...
```

## Option B: Docker

```bash
docker build -f docker/Dockerfile -t allium-deck-dev .

# Unit tests (the default CMD, self-contained):
docker run --rm allium-deck-dev

# CLI, with data mounted in:
MSYS_NO_PATHCONV=1 docker run --rm \
  -v /abs/masterdata:/data/md \
  -v /abs/music_metas.json:/data/mm.json \
  -v /abs/user.json:/data/user.json \
  -v /abs/params.json:/data/params.json \
  allium-deck-dev \
  cargo run --release --bin recommend_cli -- \
    --masterdata /data/md --music-metas /data/mm.json \
    --user /data/user.json --params /data/params.json
```

## End-to-end regression

`tests/e2e_regression.rs` needs external masterdata and testdata, injected through the
environment:

```bash
ALLIUM_MASTERDATA_CN=/abs/masterdata_cn \
ALLIUM_MASTERDATA_JP=/abs/masterdata_jp \
ALLIUM_MUSIC_METAS=/abs/music_metas.json \
ALLIUM_TESTDATA=/abs/testdata/real \
  cargo test --release --test e2e_regression
```

## Ground rules

- Do not change `Cargo.toml`'s dependencies casually: it invalidates the cached
  dependency layer and rebuilds everything.
- Performance numbers must be taken under `--release`.
- After a refactor, run the e2e suite; the output must be byte-for-byte identical.

---

# Service image

The service is a straightforward implementation that makes the engine reachable over
HTTP; it is not a tuned architecture, and what it leaves out is listed in
[`../server/README.en.md`](../server/README.en.md#scope). The image is the same: a
multi-stage build and a minimal non-root runtime, nothing more.

The build context is the repository root, because the service depends on the engine crate
by path:

```bash
docker build -f server/Dockerfile -t allium-deck-server .

docker run --rm -p 8080:8080 -v /path/to/data:/data:ro allium-deck-server \
  --masterdata cn=/data/masterdata --music-metas cn=/data/music_metas.json
```

## Three independent dimensions

The libc, the allocator and the runtime base are separate build arguments, not a bundle:

| Argument | Values | Default |
|---|---|---|
| `LIBC` | `gnu` \| `musl` | `gnu` |
| `ALLOC` | empty (the libc's own) \| `jemalloc` \| `mimalloc` | empty |
| `RUNTIME` | `distroless` \| `scratch` | `distroless` |

`musl` links statically, so it can ship on `scratch` with no libc in the image; `gnu`
needs `distroless` to supply one. Both allocators compile C, while the empty value keeps
the dependency tree pure Rust.

```bash
# Default: gnu + the libc's allocator + distroless
docker build -f server/Dockerfile -t allium-deck-server .

# Smallest image: static musl on scratch
docker build -f server/Dockerfile --build-arg LIBC=musl --build-arg RUNTIME=scratch .

# A different allocator
docker build -f server/Dockerfile --build-arg ALLOC=mimalloc .
```

## Measurements

The defaults were measured. The headline: **only one of the six combinations is
separable by this measurement**.

| Image | Size | A throughput req/s, median (range) | A p99 ms | B p99 ms | RSS steady MiB |
|---|---:|---:|---:|---:|---:|
| musl + libc allocator + scratch | 7.71 MB | **35.1** (30–37) | 324 | 510 | **18** |
| musl + jemalloc + scratch | 8.49 MB | 156.1 (134–185) | 113 | 470 | 119 |
| musl + mimalloc + scratch | 7.97 MB | 191.8 (145–240) | 69 | 439 | 155 |
| **gnu + libc allocator + distroless (default)** | 50.4 MB | 159.0 (103–192) | 108 | 453 | 36 |
| gnu + jemalloc + distroless | 51.2 MB | 176.5 (161–212) | 72 | 435 | 96 |
| gnu + mimalloc + distroless | 50.7 MB | 218.3 (157–232) | 79 | 439 | 146 |

**A**: an event multi/score request, `{"eventId":1,"eventType":"marathon","liveType":"multi","target":"score","limit":8}`, concurrency 8.
**B**: the World Bloom final chapter (`worldBloomFinaleTurn:3` + `attrFilter`) with `timeoutMs:200`, so every sample costs the same CPU budget, concurrency 8. That budget pins B's throughput at 16–17 req/s, so only its p99 is listed.

What this says:

- **musl with the libc's own allocator is clearly the worst of the six for this
  workload.** Its A-load throughput median of 35.1 is between a quarter and a sixth of
  the other five (156–218), and its range does not overlap theirs at all. Its B-load p99
  is the highest too.
- **The other five cannot be told apart.** Their per-round ranges overlap heavily — the
  widest spans 103–240 — so the gaps between them are smaller than the measurement's own
  variation. **Do not read a ranking off this table.**
- **The RSS difference is stable, though**: 18–36 MiB without a third-party allocator,
  96–155 MiB with one, a factor of 3 to 9.
- The difference is not mainly in the search. Searching accounts for 0.2–0.4 ms of an
  A-load request; the bulk is deserializing the player's collection (1300 cards, about
  361 KB here) into structures, which is a great many small allocations.
- The default is `gnu` with the libc's allocator: among the group that cannot be told
  apart, it has the cleanest dependency tree (pure Rust, no C) and the lowest RSS.
  Switching to `ALLOC=mimalloc` or `jemalloc` will not make things worse, but this table
  cannot show that it makes them better either — measure that on your own traffic.
  The 7.7 MB image and 18 MiB RSS of `LIBC=musl RUNTIME=scratch` remain attractive, but
  do not put it in front of frequent small requests.

### Conditions

These numbers are comparable only **with each other, on one machine, within one batch**;
they are not an absolute performance expectation:

- 3 rounds, with the image order shuffled inside each round from a fixed seed. The table
  reports the median across rounds, with the range in parentheses. A single pass produces
  a seemingly precise ranking, but the last five combinations swap places when the order
  is shuffled — hence medians and ranges rather than places.
- The data is this repository's deterministic synthetic set (26 characters, 1300 cards, a
  fully levelled account), generated by
  `cargo run --release --manifest-path server/Cargo.toml --bin export_synth_masterdata`.
  It contains no game data. The
  synthetic account is larger than a real one, so B's absolute timings do not stand in
  for a real World Bloom request.
- Host: Docker Desktop (WSL2), container limited to 4 CPUs and 4 GiB, `--workers 4`.
- The load generator shares the machine with the service and competes with it for CPU.

## Measuring your own traffic

The service already emits the breakdown; no extra probe is needed:

```bash
# Per-response timing: queue, pool build, search, total
curl -s localhost:8080/v1/recommend -d @request.json -H 'content-type: application/json' \
  | jq .timing

# /metrics has histograms of the same stages, plus queue rejections and worker panics
curl -s localhost:8080/metrics | grep -E 'alliumdeck_(search|build_pool|queue_wait)_seconds'
```

Drive it with your own request mix at your own concurrency, and compare throughput, p99
and `docker stats` RSS across `ALLOC` values. Microbenchmark `malloc`/`free` figures say
little about this workload: what decides it is the allocation pattern of request-body
parsing and the distribution of search durations.
