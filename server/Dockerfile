# Build context is the repository root, because the server depends on the engine crate
# by path:
#
#   docker build -f server/Dockerfile -t allium-deck-server .
#
# Three independent build arguments, so the libc, the allocator and the runtime base can
# be chosen separately rather than as a bundle:
#
#   --build-arg LIBC=musl|gnu          which C library the binary is built against
#   --build-arg ALLOC=|jemalloc|mimalloc   Rust's global allocator (empty = the libc's)
#   --build-arg RUNTIME=scratch|distroless the base the binary is shipped on
#
# The defaults are measured rather than assumed; docker/README.en.md records the comparison
# and how to repeat it against your own traffic.

ARG RUST_VERSION=1.94
ARG LIBC=gnu
ARG RUNTIME=distroless

# --- build bases -------------------------------------------------------------------
# Alpine's host target is *-linux-musl, which Rust links statically by default, so the
# result runs on any kernel without a libc on the image.
FROM rust:${RUST_VERSION}-alpine AS build-musl
RUN apk add --no-cache musl-dev
# jemalloc and mimalloc compile C, which needs a toolchain that is not in the base image.
RUN apk add --no-cache build-base

FROM rust:${RUST_VERSION}-slim AS build-gnu
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

# --- build -------------------------------------------------------------------------
FROM build-${LIBC} AS build
ARG ALLOC=""
ENV CARGO_TERM_COLOR=always
WORKDIR /build

# The engine crate, then the server crate. Both manifests are copied before the sources
# so that an unchanged dependency set can be served from the cache mounts below.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY data ./data
# The engine's manifest declares its bench target explicitly, so cargo checks that the
# file exists while resolving the path dependency, even though it is never built here.
COPY benches ./benches
COPY server/Cargo.toml server/Cargo.lock ./server/
COPY server/src ./server/src
COPY server/openapi.json ./server/

# The cache mounts make rebuilds incremental without a helper tool. The binary is copied
# out of the cached target directory, which does not survive the layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/server/target \
    cd server \
    && cargo build --release --locked ${ALLOC:+--features ${ALLOC}} \
    && cp target/release/allium-deck-server /allium-deck-server

# --- runtime bases -----------------------------------------------------------------
FROM scratch AS runtime-scratch
FROM gcr.io/distroless/cc-debian13 AS runtime-distroless

# --- runtime -----------------------------------------------------------------------
FROM runtime-${RUNTIME} AS runtime
COPY --from=build /allium-deck-server /allium-deck-server

# The same uid distroless uses for its nonroot user, so both bases behave alike.
USER 65532:65532
EXPOSE 8080

# Masterdata is mounted, never baked in: this repository carries no game data.
#
#   docker run --rm -p 8080:8080 -v /path/to/data:/data:ro allium-deck-server \
#     --masterdata cn=/data/masterdata --music-metas cn=/data/music_metas.json
#
# There is no shell on these bases, so health checks go through the orchestrator's own
# HTTP probe against /healthz and /readyz rather than a container command.
ENTRYPOINT ["/allium-deck-server"]
CMD ["--bind", "0.0.0.0:8080"]
