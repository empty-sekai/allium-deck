#!/usr/bin/env python3
"""Resolve shared WASM build inputs for CI."""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
from pathlib import Path

from http_retry import read_url


def env_first(*names: str) -> str:
    for name in names:
        value = os.environ.get(name, "").strip()
        if value:
            return value
    return ""


def git_value(*args: str) -> str:
    try:
        return subprocess.check_output(["git", *args], text=True, stderr=subprocess.DEVNULL).strip()
    except Exception:
        return ""


def infer_publish_version(cdn_base: str, region: str) -> str:
    manifest_url = f"{cdn_base.rstrip('/')}/deck-wasm/{region}/latest/manifest.json"
    data, _headers = read_url(
        manifest_url,
        headers={"User-Agent": "allium-deck-wasm-ci/1.0"},
        timeout=30,
        label="deck wasm latest manifest",
    )
    manifest = json.loads(data)
    return str(manifest.get("masterdata_version") or "").strip()


def sanitize_version(version: str) -> str:
    version = re.sub(r"[^A-Za-z0-9._-]+", "-", version).strip("-")
    if not version:
        raise RuntimeError("masterdata version became empty after sanitization")
    return version


def write_env(path: Path, values: dict[str, str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [f"export {key}={shlex.quote(value)}" for key, value in values.items()]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=["publish", "release"], required=True)
    parser.add_argument("--env-out", default=".tmp/cnb-wasm.env")
    args = parser.parse_args()

    region = env_first("REGION") or "cn"
    if region != "cn":
        raise RuntimeError(f"unsupported region: {region}")

    cdn_base = (env_first("CDN_BASE") or "https://cdn.emptysekai.com").rstrip("/")
    version = env_first("MASTERDATA_VERSION")
    if not version:
        if args.mode == "publish":
            version = infer_publish_version(cdn_base, region)
        else:
            version = infer_publish_version(cdn_base, region)
    version = sanitize_version(version)

    source_repository = env_first("SOURCE_REPOSITORY", "GITHUB_REPOSITORY") or "empty-sekai/allium-deck"
    source_revision = env_first("SOURCE_REVISION", "CNB_COMMIT", "GITHUB_SHA") or git_value("rev-parse", "HEAD")
    tag = env_first("CNB_BRANCH", "GITHUB_REF_NAME") or version

    values = {
        "REGION": region,
        "CDN_BASE": cdn_base,
        "MASTERDATA_VERSION": version,
        "MASTERDATA_DIR": f".tmp/masterdata/{region}",
        "MUSIC_METAS": f".tmp/music_metas/{region}/music_metas.json",
        "MASTERDATA_MANIFEST": ".tmp/masterdata-manifest.json",
        "DIST_DIR": "dist/deck-wasm",
        "SOURCE_REPOSITORY": source_repository,
        "SOURCE_REVISION": source_revision,
        "RELEASE_TAG": tag,
        "WASM_ZIP": f"allium-deck-wasm-{tag}-{version}-{region}.zip",
    }
    write_env(Path(args.env_out), values)
    print(f"region={region}")
    print(f"masterdata_version={version}")
    print(f"source_revision={source_revision}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
