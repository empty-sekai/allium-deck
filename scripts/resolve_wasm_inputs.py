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


def load_release_inputs(root: Path, tag: str) -> dict[str, str]:
    if not re.fullmatch(r"v[0-9A-Za-z._-]+", tag):
        raise RuntimeError(f"invalid release tag: {tag!r}")
    path = root / "release-inputs" / f"{tag}.json"
    if not path.is_file():
        raise RuntimeError(f"missing pinned release inputs: {path}")
    payload = json.loads(path.read_text(encoding="utf-8"))
    version = str(payload.get("masterdata_version") or "").strip()
    music_checksum = str(payload.get("music_metas_sha256") or "").strip().lower()
    if not version:
        raise RuntimeError(f"masterdata_version is missing from {path}")
    if not re.fullmatch(r"[0-9a-f]{64}", music_checksum):
        raise RuntimeError(f"music_metas_sha256 is invalid in {path}")
    return {
        "masterdata_version": version,
        "music_metas_sha256": music_checksum,
    }


def resolve_source_date_epoch(source_revision: str) -> str:
    epoch = env_first("SOURCE_DATE_EPOCH")
    if not epoch and source_revision:
        epoch = git_value("show", "-s", "--format=%ct", source_revision)
    if not epoch.isdigit():
        raise RuntimeError(f"could not resolve commit timestamp for {source_revision!r}")
    return epoch


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
    tag = env_first("CNB_BRANCH", "GITHUB_REF_NAME")
    version = env_first("MASTERDATA_VERSION")
    music_checksum = ""
    snapshot_mode = "latest"
    if args.mode == "release":
        pinned = load_release_inputs(Path(__file__).resolve().parents[1], tag)
        pinned_version = pinned["masterdata_version"]
        if version and version != pinned_version:
            raise RuntimeError(
                f"MASTERDATA_VERSION {version!r} does not match {tag} pinned version {pinned_version!r}"
            )
        version = pinned_version
        music_checksum = pinned["music_metas_sha256"]
        snapshot_mode = "pinned"
    elif not version:
        version = infer_publish_version(cdn_base, region)
    version = sanitize_version(version)

    source_repository = env_first("SOURCE_REPOSITORY", "GITHUB_REPOSITORY") or "empty-sekai/allium-deck"
    source_revision = env_first("SOURCE_REVISION", "CNB_COMMIT", "GITHUB_SHA") or git_value("rev-parse", "HEAD")
    source_date_epoch = resolve_source_date_epoch(source_revision)
    tag = tag or version

    values = {
        "REGION": region,
        "CDN_BASE": cdn_base,
        "MASTERDATA_VERSION": version,
        "SNAPSHOT_MODE": snapshot_mode,
        "MUSIC_METAS_SHA256": music_checksum,
        "MASTERDATA_DIR": f".tmp/masterdata/{region}",
        "MUSIC_METAS": f".tmp/music_metas/{region}/music_metas.json",
        "MASTERDATA_MANIFEST": ".tmp/masterdata-manifest.json",
        "DIST_DIR": "dist/deck-wasm",
        "SOURCE_REPOSITORY": source_repository,
        "SOURCE_REVISION": source_revision,
        "SOURCE_DATE_EPOCH": source_date_epoch,
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
