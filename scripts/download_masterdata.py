#!/usr/bin/env python3
"""Download masterdata from the Team-Haruki upstream repositories for wasm builds.

标准上游输入（--region 映射）：
- cn → https://github.com/Team-Haruki/haruki-sekai-sc-master （branch: master, 目录: master/）
- jp → https://github.com/Team-Haruki/haruki-sekai-master     （branch: master, 目录: master/）

版本标签 = 上游仓库 master 分支最新 commit SHA（不可变、可追溯）。
国内 runner 可通过 GH_RAW_MIRROR 环境变量配置镜像前缀（例如
``https://ghproxy.net``），拼成 ``{mirror}/https://raw.githubusercontent.com/...``。

music_metas 不在 masterdata 仓库内，仍从 `--music-metas-url`（默认 Empty Sekai
CDN 独立命名空间）获取。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import urllib.error
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable

from http_retry import DEFAULT_ATTEMPTS, read_url


UPSTREAM_REPOS = {
    "cn": ("Team-Haruki/haruki-sekai-sc-master", "master", "master"),
    "jp": ("Team-Haruki/haruki-sekai-master", "master", "master"),
}


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


def raw_base(mirror: str) -> str:
    base = f"https://raw.githubusercontent.com"
    mirror = (mirror or os.environ.get("GH_RAW_MIRROR", "")).strip().rstrip("/")
    return f"{mirror}/{base}" if mirror else base


def table_url(mirror: str, repo: str, branch: str, prefix: str, table: str) -> str:
    return f"{raw_base(mirror)}/{repo}/{branch}/{prefix}/{table}.json"


def fetch_version(repo: str, branch: str, timeout: int, retries: int) -> str:
    """解析上游 master 分支最新 commit SHA。

    优先 commits atom（无认证、无 REST API 速率限制），失败时回落
    api.github.com。返回完整 40 位 SHA。
    """
    atom_url = f"https://github.com/{repo}/commits/{branch}.atom"
    data, _headers = fetch(atom_url, timeout, label="upstream version (atom)", retries=retries)
    entries = re.findall(r"<id>[^<]*?/commit/([0-9a-f]{40})</id>", data.decode("utf-8", "replace"))
    if entries:
        return entries[0]
    api_url = f"https://api.github.com/repos/{repo}/git/refs/heads/{branch}"
    data, _headers = fetch(api_url, timeout, label="upstream version (api)", retries=retries)
    payload = parse_json(data, "upstream version")
    sha = str(((payload or {}).get("object") or {}).get("sha") or "").strip()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise RuntimeError(f"could not resolve upstream commit sha for {repo}@{branch}")
    return sha


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


# music metas 不在 masterdata 仓库内，按区服取自社区公开源：
# - cn：Empty Sekai CDN 独立命名空间
# - jp：33 Kit 公开数据基座（xfl03/33KitFrontend NEXT_PUBLIC_SEKAI_DATA_BASE）
MUSIC_METAS_SOURCES = {
    "cn": "https://cdn.emptysekai.com/music_metas/cn/latest/music_metas.json",
    "jp": "https://sekai-data.3-3.dev/music_metas.json",
}


def music_url(cdn_base: str, region: str) -> str:
    return MUSIC_METAS_SOURCES.get(region) or         f"{cdn_base.rstrip('/')}/music_metas/{region}/latest/music_metas.json"


def ensure_same_snapshot(before: bytes, after: bytes, label: str) -> None:
    if sha256(before) != sha256(after):
        raise RuntimeError(f"{label} changed during download")


def ensure_expected_checksum(data: bytes, expected: str, label: str) -> None:
    expected = expected.strip().lower()
    if len(expected) != 64 or any(character not in "0123456789abcdef" for character in expected):
        raise RuntimeError(f"{label} expected SHA-256 is invalid")
    actual = sha256(data)
    if actual != expected:
        raise RuntimeError(
            f"{label} checksum mismatch: expected {expected}, downloaded {actual}"
        )


def validate_snapshot_mode(mode: str, expected_music_sha256: str) -> None:
    if mode == "pinned" and not expected_music_sha256.strip():
        raise RuntimeError("snapshot-mode=pinned requires --expected-music-sha256")


def download_table(
    table: str,
    *,
    mirror: str,
    repo: str,
    branch: str,
    prefix: str,
    out_dir: Path,
    timeout: int,
    retries: int,
) -> dict[str, object]:
    url = table_url(mirror, repo, branch, prefix, table)
    target = out_dir / f"{table}.json"
    try:
        data, _headers = fetch(url, timeout, label=f"{table}.json", retries=retries)
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
    }


def download_tables(
    tables: Iterable[str],
    *,
    mirror: str,
    repo: str,
    branch: str,
    prefix: str,
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
                mirror=mirror,
                repo=repo,
                branch=branch,
                prefix=prefix,
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
            entries.append(entry)

    if errors:
        detail = "\n  - ".join(errors)
        raise RuntimeError(f"upstream table download failed:\n  - {detail}")

    order = {f"{table}.json": index for index, table in enumerate(ordered_tables)}
    entries.sort(key=lambda entry: order.get(str(entry["name"]), len(order)))
    return entries


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--region", default="cn", choices=sorted(UPSTREAM_REPOS))
    parser.add_argument("--upstream-repo", default="",
                        help="override the Team-Haruki upstream repo (owner/name)")
    parser.add_argument("--upstream-ref", default="",
                        help="override the upstream branch/ref")
    parser.add_argument("--version", default="",
                        help="pin an upstream commit SHA instead of resolving latest")
    parser.add_argument("--music-metas", required=True)
    parser.add_argument("--music-metas-url", default="",
                        help="music metas source override; per-region default")
    parser.add_argument("--cdn-base", default="https://cdn.emptysekai.com",
                        help="fallback base for music metas of unlisted regions")
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--manifest-out", required=True)
    parser.add_argument("--snapshot-mode", choices=["latest", "pinned"], required=True)
    parser.add_argument("--expected-music-sha256", default="")
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--retries", type=int, default=DEFAULT_ATTEMPTS)
    parser.add_argument("--workers", type=int, default=8)
    args = parser.parse_args()

    repo, branch, prefix = UPSTREAM_REPOS[args.region]
    if args.upstream_repo:
        repo = args.upstream_repo
    if args.upstream_ref:
        branch = args.upstream_ref
    mirror = os.environ.get("GH_RAW_MIRROR", "")

    validate_snapshot_mode(args.snapshot_mode, args.expected_music_sha256)

    out_dir = Path(args.out_dir)
    music_path = Path(args.music_metas)
    manifest_path = Path(args.manifest_out)
    out_dir.mkdir(parents=True, exist_ok=True)
    music_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.parent.mkdir(parents=True, exist_ok=True)

    version = args.version.strip()
    if version:
        validate_snapshot_mode("pinned", args.expected_music_sha256)
    elif args.snapshot_mode == "latest":
        version = fetch_version(repo, branch, args.timeout, args.retries)
    else:
        raise RuntimeError("snapshot-mode=pinned requires --version (upstream commit sha)")

    music_cdn_base = args.music_metas_url.strip() or \
        f"https://cdn.emptysekai.com/music_metas/{args.region}/latest/music_metas.json"
    if music_cdn_base.endswith("music_metas.json"):
        music_download_url = music_cdn_base
    else:
        music_download_url = f"{music_cdn_base.rstrip('/')}/music_metas.json"

    music_data_before, _headers = fetch(
        music_download_url,
        args.timeout,
        label="music_metas.json (before)",
        retries=args.retries,
    )

    tables = download_tables(
        CDN_TABLES,
        mirror=mirror,
        repo=repo,
        branch=branch,
        prefix=prefix,
        out_dir=out_dir,
        timeout=args.timeout,
        retries=args.retries,
        workers=args.workers,
    )

    music_data, _headers_after = fetch(
        music_download_url,
        args.timeout,
        label="music_metas.json (after)",
        retries=args.retries,
    )
    ensure_same_snapshot(music_data_before, music_data, "music_metas.json")
    if args.expected_music_sha256:
        ensure_expected_checksum(
            music_data,
            args.expected_music_sha256,
            "music_metas.json",
        )
    music_rows = parse_json(music_data, "music_metas.json")
    if not isinstance(music_rows, list) or not music_rows:
        raise RuntimeError("music_metas.json is empty or not an array")
    music_path.write_bytes(music_data)
    print(f"[ok] music_metas.json {len(music_data)} bytes", file=sys.stderr)

    manifest = {
        "region": args.region,
        "version": version,
        "upstream": {
            "repo": repo,
            "ref": branch,
            "commit": version,
        },
        "downloaded_at": utc_now(),
        "tables": tables,
        "music_metas": {
            "name": "music_metas.json",
            "url": music_download_url,
            "bytes": len(music_data),
            "sha256": sha256(music_data),
        },
    }
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"[manifest] {manifest_path}", file=sys.stderr)
    print(f"[version] {version}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
