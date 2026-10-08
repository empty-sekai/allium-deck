#!/usr/bin/env python3
"""Validate the complete release asset set and write portable SHA256SUMS."""

from __future__ import annotations

import argparse
import hashlib
import re
from pathlib import Path


def expected_assets(version: str) -> set[str]:
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("invalid release version")
    return {
        *(f"recommend_cli-v{version}-{platform}" for platform in (
            "linux-x86_64",
            "linux-x86_64-musl",
            "linux-aarch64",
            "windows-x86_64.exe",
            "macos-aarch64",
        )),
        f"allium-deck-wasm-v{version}-cn.zip",
        f"allium-deck-wasm-v{version}.tgz",
    }


def create_checksums(directory: Path, version: str) -> Path:
    expected = expected_assets(version)
    actual = {path.name for path in directory.iterdir() if path.name != "SHA256SUMS"}
    if actual != expected:
        raise ValueError(
            f"incomplete release asset set: missing={sorted(expected - actual)}, "
            f"unexpected={sorted(actual - expected)}"
        )
    rows = []
    for name in sorted(expected):
        path = directory / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"release asset must be a nonempty regular file: {name}")
        with path.open("rb") as handle:
            digest = hashlib.file_digest(handle, "sha256").hexdigest()
        rows.append(f"{digest}  {name}\n")
    output = directory / "SHA256SUMS"
    output.write_text("".join(rows), encoding="utf-8", newline="\n")
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    print(create_checksums(args.directory, args.version))


if __name__ == "__main__":
    main()
