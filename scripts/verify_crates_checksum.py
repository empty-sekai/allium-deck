#!/usr/bin/env python3
"""Verify that a crates.io version exists with an expected package checksum."""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request


NOT_FOUND = 3


def query_checksum(crate: str, version: str, timeout: int = 30) -> str | None:
    request = urllib.request.Request(
        f"https://crates.io/api/v1/crates/{crate}/{version}",
        headers={"User-Agent": "allium-deck-release-ci/1.0"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = json.load(response)
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return None
        raise RuntimeError(f"crates.io version lookup failed with HTTP {exc.code}") from exc
    return str(payload["version"]["checksum"]).lower()


def wait_for_matching_checksum(
    crate: str,
    version: str,
    expected: str,
    *,
    attempts: int,
    delay: float,
) -> None:
    expected = expected.strip().lower()
    for attempt in range(1, attempts + 1):
        remote = query_checksum(crate, version)
        if remote is not None:
            if remote != expected:
                raise RuntimeError(
                    f"crates.io {crate} {version} checksum mismatch: "
                    f"expected {expected}, remote {remote}"
                )
            print(f"crates.io {crate} {version} matches local package")
            return
        if attempt < attempts:
            print(
                f"crates.io {crate} {version} is not visible yet "
                f"({attempt}/{attempts}); retrying in {delay:g}s",
                file=sys.stderr,
            )
            time.sleep(delay)
    raise FileNotFoundError(f"crates.io {crate} {version} is not published")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--crate", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--checksum", required=True)
    parser.add_argument("--attempts", type=int, default=1)
    parser.add_argument("--delay", type=float, default=0)
    args = parser.parse_args()
    if args.attempts < 1:
        raise RuntimeError("attempts must be at least 1")
    try:
        wait_for_matching_checksum(
            args.crate,
            args.version,
            args.checksum,
            attempts=args.attempts,
            delay=args.delay,
        )
    except FileNotFoundError as exc:
        print(f"not found: {exc}", file=sys.stderr)
        return NOT_FOUND
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
