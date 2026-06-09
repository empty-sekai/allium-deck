#!/usr/bin/env python3
"""Download CN masterdata from the public Empty Sekai CDN for wasm builds."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable


REQUIRED_TABLES = [
    "gameCharacterUnits",
    "cards",
    "events",
    "cardRarities",
    "cardEpisodes",
    "masterLessons",
    "skills",
    "areaItemLevels",
    "characterRanks",
    "cardMysekaiCanvasBonuses",
    "eventCards",
    "eventDeckBonuses",
    "eventCardBonusLimits",
    "eventHonorBonuses",
    "worldBloomDifferentAttributeBonuses",
    "eventSkillScoreUpLimits",
    "eventRarityBonusRates",
]

OPTIONAL_TABLES = [
    "worldBlooms",
    "worldBloomSupportDeckBonusesWL1",
    "worldBloomSupportDeckBonusesWL2",
    "worldBloomSupportDeckBonusesWL3",
    "worldBloomSupportDeckBonuses",
    "worldBloomSupportDeckUnitEventLimitedBonuses",
    "eventMysekaiFixtureGameCharacterPerformanceBonusLimits",
    "honors",
    "bondsHonors",
]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str, timeout: int) -> tuple[bytes, dict[str, str]]:
    request = urllib.request.Request(url, headers={"User-Agent": "allium-deck-wasm-ci/1.0"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read(), dict(response.headers.items())


def parse_json(data: bytes, label: str) -> object:
    try:
        return json.loads(data)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{label} is not valid JSON: {exc}") from exc


def table_url(cdn_base: str, region: str, table: str) -> str:
    return f"{cdn_base.rstrip('/')}/masterdata/{region}/latest/{table}.json"


def music_url(cdn_base: str, region: str) -> str:
    return f"{cdn_base.rstrip('/')}/music_metas/{region}/latest/music_metas.json"


def download_table(
    table: str,
    *,
    required: bool,
    cdn_base: str,
    region: str,
    out_dir: Path,
    timeout: int,
) -> dict[str, object] | None:
    url = table_url(cdn_base, region, table)
    target = out_dir / f"{table}.json"
    try:
        data, headers = fetch(url, timeout)
    except urllib.error.HTTPError as exc:
        if not required and exc.code == 404:
            print(f"[skip] optional {table}.json not found on CDN", file=sys.stderr)
            return None
        raise RuntimeError(f"download failed for {url}: HTTP {exc.code}") from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(f"download failed for {url}: {exc}") from exc

    parse_json(data, f"{table}.json")
    target.write_bytes(data)
    print(f"[ok] {table}.json {len(data)} bytes", file=sys.stderr)
    return {
        "name": f"{table}.json",
        "url": url,
        "required": required,
        "bytes": len(data),
        "sha256": sha256(data),
        "etag": headers.get("ETag") or headers.get("etag"),
    }


def download_tables(
    tables: Iterable[str],
    *,
    required: bool,
    cdn_base: str,
    region: str,
    out_dir: Path,
    timeout: int,
) -> list[dict[str, object]]:
    entries = []
    for table in tables:
        entry = download_table(
            table,
            required=required,
            cdn_base=cdn_base,
            region=region,
            out_dir=out_dir,
            timeout=timeout,
        )
        if entry is not None:
            entries.append(entry)
    return entries


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--region", default="cn")
    parser.add_argument("--cdn-base", default="https://cdn.emptysekai.com")
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--music-metas", required=True)
    parser.add_argument("--manifest-out", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--timeout", type=int, default=60)
    args = parser.parse_args()

    if args.region != "cn":
        raise RuntimeError("only region=cn is supported for embedded wasm builds")
    if not args.version.strip():
        raise RuntimeError("masterdata dataVersion is required for the immutable wasm release label")

    out_dir = Path(args.out_dir)
    music_path = Path(args.music_metas)
    manifest_path = Path(args.manifest_out)
    out_dir.mkdir(parents=True, exist_ok=True)
    music_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.parent.mkdir(parents=True, exist_ok=True)

    required = download_tables(
        REQUIRED_TABLES,
        required=True,
        cdn_base=args.cdn_base,
        region=args.region,
        out_dir=out_dir,
        timeout=args.timeout,
    )
    optional = download_tables(
        OPTIONAL_TABLES,
        required=False,
        cdn_base=args.cdn_base,
        region=args.region,
        out_dir=out_dir,
        timeout=args.timeout,
    )

    music_data, music_headers = fetch(music_url(args.cdn_base, args.region), args.timeout)
    music_rows = parse_json(music_data, "music_metas.json")
    if not isinstance(music_rows, list) or not music_rows:
        raise RuntimeError("music_metas.json is empty or not an array")
    music_path.write_bytes(music_data)
    print(f"[ok] music_metas.json {len(music_data)} bytes", file=sys.stderr)

    manifest = {
        "region": args.region,
        "version": args.version,
        "cdn_base": args.cdn_base.rstrip("/"),
        "downloaded_at": utc_now(),
        "required_tables": REQUIRED_TABLES,
        "optional_tables": OPTIONAL_TABLES,
        "tables": required + optional,
        "music_metas": {
            "name": "music_metas.json",
            "url": music_url(args.cdn_base, args.region),
            "bytes": len(music_data),
            "sha256": sha256(music_data),
            "etag": music_headers.get("ETag") or music_headers.get("etag"),
        },
    }
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"[manifest] {manifest_path}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
