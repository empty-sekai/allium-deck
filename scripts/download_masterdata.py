#!/usr/bin/env python3
"""Download CN masterdata from the public Empty Sekai CDN for wasm builds."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import urllib.error
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable

from http_retry import DEFAULT_ATTEMPTS, read_url


CDN_TABLES = [
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
    "mysekaiGates",
    "mysekaiGateLevels",
    "eventCards",
    "eventDeckBonuses",
    "eventCardBonusLimits",
    "eventHonorBonuses",
    "worldBloomDifferentAttributeBonuses",
    "worldBlooms",
    "worldBloomSupportDeckBonuses",
    "worldBloomSupportDeckUnitEventLimitedBonuses",
    "eventMysekaiFixtureGameCharacterPerformanceBonusLimits",
    "eventSkillScoreUpLimits",
    "eventRarityBonusRates",
    "honors",
    "bondsHonors",
]


def utc_now() -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    instant = datetime.fromtimestamp(int(epoch), timezone.utc) if epoch else datetime.now(timezone.utc)
    return instant.isoformat(timespec="seconds").replace("+00:00", "Z")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str, timeout: int, *, label: str, retries: int) -> tuple[bytes, dict[str, str]]:
    return read_url(
        url,
        headers={"User-Agent": "allium-deck-wasm-ci/1.0"},
        timeout=timeout,
        attempts=retries,
        label=label,
    )


def parse_json(data: bytes, label: str) -> object:
    try:
        return json.loads(data)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{label} is not valid JSON: {exc}") from exc


def table_url(cdn_base: str, region: str, version: str, table: str) -> str:
    return f"{cdn_base.rstrip('/')}/masterdata/{region}/{version}/{table}.json"


def music_url(cdn_base: str, region: str) -> str:
    return f"{cdn_base.rstrip('/')}/music_metas/{region}/latest/music_metas.json"


def ensure_same_snapshot(before: bytes, after: bytes, label: str) -> None:
    if sha256(before) != sha256(after):
        raise RuntimeError(f"{label} changed during download")


def download_table(
    table: str,
    *,
    cdn_base: str,
    region: str,
    version: str,
    out_dir: Path,
    timeout: int,
    retries: int,
) -> dict[str, object] | None:
    url = table_url(cdn_base, region, version, table)
    target = out_dir / f"{table}.json"
    try:
        data, headers = fetch(url, timeout, label=f"{table}.json", retries=retries)
    except urllib.error.HTTPError as exc:
        raise RuntimeError(f"download failed for {url}: HTTP {exc.code}") from exc
    except Exception as exc:
        raise RuntimeError(f"download failed for {url}: {exc}") from exc

    parse_json(data, f"{table}.json")
    target.write_bytes(data)
    print(f"[ok] {table}.json {len(data)} bytes", file=sys.stderr)
    return {
        "name": f"{table}.json",
        "url": url,
        "bytes": len(data),
        "sha256": sha256(data),
        "etag": headers.get("ETag") or headers.get("etag"),
    }


def download_tables(
    tables: Iterable[str],
    *,
    cdn_base: str,
    region: str,
    version: str,
    out_dir: Path,
    timeout: int,
    retries: int,
    workers: int,
) -> list[dict[str, object]]:
    ordered_tables = list(tables)
    if not ordered_tables:
        return []

    entries = []
    errors = []
    max_workers = max(1, min(workers, len(ordered_tables)))
    with ThreadPoolExecutor(max_workers=max_workers) as executor:
        futures = {
            executor.submit(
                download_table,
                table,
                cdn_base=cdn_base,
                region=region,
                version=version,
                out_dir=out_dir,
                timeout=timeout,
                retries=retries,
            ): table
            for table in ordered_tables
        }
        for future in as_completed(futures):
            table = futures[future]
            try:
                entry = future.result()
            except Exception as exc:
                errors.append(f"{table}: {exc}")
                continue
            if entry is not None:
                entries.append(entry)

    if errors:
        detail = "\n  - ".join(errors)
        raise RuntimeError(f"CDN table download failed:\n  - {detail}")

    order = {f"{table}.json": index for index, table in enumerate(ordered_tables)}
    entries.sort(key=lambda entry: order.get(str(entry["name"]), len(order)))
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
    parser.add_argument("--retries", type=int, default=DEFAULT_ATTEMPTS)
    parser.add_argument("--workers", type=int, default=8)
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

    music_data_before, music_headers = fetch(
        music_url(args.cdn_base, args.region),
        args.timeout,
        label="music_metas.json (before)",
        retries=args.retries,
    )

    tables = download_tables(
        CDN_TABLES,
        cdn_base=args.cdn_base,
        region=args.region,
        version=args.version,
        out_dir=out_dir,
        timeout=args.timeout,
        retries=args.retries,
        workers=args.workers,
    )

    music_data, music_headers_after = fetch(
        music_url(args.cdn_base, args.region),
        args.timeout,
        label="music_metas.json (after)",
        retries=args.retries,
    )
    ensure_same_snapshot(music_data_before, music_data, "music_metas.json")
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
        "tables": tables,
        "music_metas": {
            "name": "music_metas.json",
            "url": music_url(args.cdn_base, args.region),
            "bytes": len(music_data),
            "sha256": sha256(music_data),
            "etag": music_headers.get("ETag") or music_headers.get("etag"),
            "verified_etag": music_headers_after.get("ETag")
            or music_headers_after.get("etag"),
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
