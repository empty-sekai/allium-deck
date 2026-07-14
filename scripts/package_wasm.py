#!/usr/bin/env python3
"""Package wasm-pack output for CDN publication."""

from __future__ import annotations

import argparse
import hashlib
import json
import mimetypes
import os
import shutil
from datetime import datetime, timezone
from pathlib import Path


def utc_now() -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    instant = datetime.fromtimestamp(int(epoch), timezone.utc) if epoch else datetime.now(timezone.utc)
    return instant.isoformat(timespec="seconds").replace("+00:00", "Z")


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def content_type(path: Path) -> str:
    if path.suffix == ".wasm":
        return "application/wasm"
    if path.suffix == ".js":
        return "text/javascript; charset=utf-8"
    if path.suffix == ".json":
        return "application/json; charset=utf-8"
    if path.suffix == ".ts":
        return "text/plain; charset=utf-8"
    guessed, _ = mimetypes.guess_type(path.name)
    return guessed or "application/octet-stream"


def copy_if_exists(src: Path, dst: Path) -> None:
    if src.exists():
        shutil.copy2(src, dst)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pkg-dir", default="pkg")
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--region", default="cn")
    parser.add_argument("--version", required=True)
    parser.add_argument("--cdn-base", default="https://cdn.emptysekai.com")
    parser.add_argument("--source-repository", default="")
    parser.add_argument("--source-revision", default="")
    parser.add_argument("--masterdata-manifest", default="")
    args = parser.parse_args()

    pkg_dir = Path(args.pkg_dir)
    out_dir = Path(args.out_dir)
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)

    required = [
        pkg_dir / "allium_deck.js",
        pkg_dir / "allium_deck_bg.wasm",
    ]
    for path in required:
        if not path.exists():
            raise RuntimeError(f"missing wasm-pack output: {path}")
        shutil.copy2(path, out_dir / path.name)

    for optional in [
        pkg_dir / "allium_deck.d.ts",
        pkg_dir / "package.json",
        Path("LICENSE-MIT"),
        Path("LICENSE-APACHE"),
    ]:
        copy_if_exists(optional, out_dir / optional.name)

    if args.masterdata_manifest:
        copy_if_exists(Path(args.masterdata_manifest), out_dir / "masterdata.json")

    files = []
    for file in sorted(path for path in out_dir.iterdir() if path.is_file()):
        files.append(
            {
                "name": file.name,
                "bytes": file.stat().st_size,
                "sha256": sha256(file),
                "content_type": content_type(file),
            }
        )

    cdn_base = args.cdn_base.rstrip("/")
    manifest = {
        "name": "allium-deck-wasm",
        "region": args.region,
        "masterdata_version": args.version,
        "built_at": utc_now(),
        "source": {
            "repository": args.source_repository,
            "revision": args.source_revision,
        },
        "cdn": {
            "latest_base_url": f"{cdn_base}/deck-wasm/{args.region}/latest",
            "versioned_base_url": f"{cdn_base}/deck-wasm/{args.region}/{args.version}",
        },
        "entrypoint": "allium_deck.js",
        "wasm": "allium_deck_bg.wasm",
        "files": files,
    }
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"[package] {out_dir}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}")
        raise SystemExit(1)
