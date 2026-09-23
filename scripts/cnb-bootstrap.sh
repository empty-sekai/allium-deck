#!/usr/bin/env bash
# Prepare the standard CNB Workspace image for this repository's Rust checks.
set -euo pipefail

if [[ ! -f /etc/debian_version ]]; then
  echo "This bootstrap expects the CNB Debian development image" >&2
  exit 1
fi

apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  build-essential pkg-config ca-certificates curl

if [[ ! -x /root/.cargo/bin/rustup ]]; then
  installer=$(mktemp)
  trap 'rm -f "$installer"' EXIT
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$installer"
  sh "$installer" -y --profile minimal --default-toolchain 1.98.0 \
    --component rustfmt --component clippy
fi

. /root/.cargo/env
rustup toolchain install 1.98.0 --profile minimal --component rustfmt --component clippy
rustup default 1.98.0
rustc -Vv
cargo -V
