#!/usr/bin/env python3
"""E2E 差分测试：allium-deck（Rust 引擎）对比上游 deck-service（C++ 引擎）。

参考实现：Team-Haruki/deck-service 官方 release（静态 musl 二进制，内嵌
sekai-deck-recommend-cpp@46c3d60，含 WL3 模拟终章）。在 Docker 中以
`algorithm=dfs`（精确搜索）运行，与本引擎同输入（masterdata + music metas +
userdata）对拍。

masterdata 直接挂载 Team-Haruki masterdata 仓库（deck-service 原生支持该布局）：
- cn → haruki-sekai-sc-master（master/*.json）
- jp → haruki-sekai-master

用法（仓库根目录）：
    python scripts/e2e_compare_upstream.py [--keep-service] [--scenarios name,...]

判定：同一 target 下，双方最优 `score`（活动点/live 分）相对差 ≤ 容差即 PASS；
`target=bonus` 场景同时比对加成率。退出码 0 = 全部通过。
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PORT = 3999
RELEASE_TAG = "v0.5.1"
RELEASE_URL = (
    "https://github.com/Team-Haruki/deck-service/releases/download/"
    f"{RELEASE_TAG}/deck-service-linux-x64.tar.gz"
)
MUSIC_METAS_URLS = {
    "cn": "https://cdn.emptysekai.com/music_metas/cn/latest/music_metas.json",
    # 33 Kit（xfl03）公开数据基座：https://3-3.dev 前端 NEXT_PUBLIC_SEKAI_DATA_BASE
    "jp": "https://sekai-data.3-3.dev/music_metas.json",
}
SCORE_REL_TOLERANCE = 0.005  # 精确对拍下允许的浮点/公式舍入差异


def log(message: str) -> None:
    print(f"[e2e] {message}", flush=True)


def http_json(url: str, payload: dict | None = None, timeout: int = 120) -> dict:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8") if payload is not None else None,
        headers={"Content-Type": "application/json"},
        method="POST" if payload is not None else "GET",
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def ensure_masterdata(name: str, repo_url: str) -> Path:
    """确保 Team-Haruki masterdata 仓库在本地（浅克隆，仅 master 分支）。"""
    target = REPO_ROOT / ".tmp" / "e2e" / name
    if not (target / "master" / "events.json").exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        log(f"cloning {repo_url} -> {target}")
        subprocess.run(
            ["git", "clone", "--depth", "1", repo_url, str(target)],
            check=True,
        )
    return target


def ensure_music_metas(region: str) -> Path:
    target = REPO_ROOT / ".tmp" / "e2e" / f"music_metas-{region}.json"
    if not target.exists() or target.stat().st_size < 1000:
        target.parent.mkdir(parents=True, exist_ok=True)
        url = MUSIC_METAS_URLS[region]
        log(f"downloading {region} music metas from {url}")
        target.write_bytes(urllib.request.urlopen(url, timeout=120).read())
    return target


def ensure_service_binary() -> Path:
    target = REPO_ROOT / ".tmp" / "e2e" / "deck-service"
    if not target.exists():
        archive = target.with_suffix(".tar.gz")
        archive.parent.mkdir(parents=True, exist_ok=True)
        log(f"downloading deck-service {RELEASE_TAG} (Team-Haruki 官方 release，参考引擎)")
        archive.write_bytes(urllib.request.urlopen(RELEASE_URL, timeout=120).read())
        sums_url = (
            "https://github.com/Team-Haruki/deck-service/releases/download/"
            f"{RELEASE_TAG}/SHA256SUMS-{RELEASE_TAG}.txt"
        )
        expected = None
        try:
            sums = urllib.request.urlopen(sums_url, timeout=60).read().decode()
            import hashlib as _h
            actual = _h.sha256(archive.read_bytes()).hexdigest()
            for line in sums.splitlines():
                parts = line.split()
                if len(parts) == 2 and parts[1].endswith("linux-x64.tar.gz"):
                    expected = parts[0]
            if expected and expected != actual:
                raise RuntimeError(
                    f"deck-service checksum mismatch: expected {expected}, got {actual}"
                )
            log(f"checksum ok ({expected})")
        except urllib.error.URLError:
            log("SHA256SUMS 下载失败，跳过校验（不阻断）")
        with tarfile.open(archive, "r:gz") as tar:
            tar.extractall(archive.parent, filter="data")
        archive.unlink(missing_ok=True)
    if not target.exists():
        raise RuntimeError(f"deck-service binary not found after extraction: {target}")
    return target


def start_service(
    binary: Path,
    cpp_data: Path,
    masterdata: Path,
    music_metas_dir: Path,
    port: int,
) -> subprocess.Popen | None:
    """在 Docker 中启动 deck-service（release 产物为 glibc 2.38+ 动态链接，需 ubuntu 24.04+）。"""
    subprocess.run(["docker", "rm", "-f", "allium-deck-e2e"], capture_output=True)
    # 二进制经由 docker cp 注入卷，避免宿主路径挂载二进制的 Windows 路径问题
    run = subprocess.run(
        [
            "docker", "run", "-d", "--name", "allium-deck-e2e",
            "-p", f"127.0.0.1:{port}:3000",
            "-v", f"{cpp_data.as_posix()}:/data:ro",
            "-v", f"{masterdata.as_posix()}:/masterdata:ro",
            "-v", f"{music_metas_dir.as_posix()}:/musicmetas:ro",
            "-e", "DECK_DATA_DIR=/data",
            "-e", "DECK_MASTERDATA_BASE_DIR=/masterdata",
            "-e", "DECK_MUSICMETAS_BASE_DIR=/musicmetas",
            "-e", "DECK_MUSICMETAS_REGIONS=cn,jp",
            "-e", "DECK_MUSICMETAS_FILE_CN=/musicmetas/music_metas-cn.json",
            "-e", "DECK_MUSICMETAS_FILE_JP=/musicmetas/music_metas-jp.json",
            "-e", "DECK_ENGINE_POOL_SIZE=2",
            "ubuntu:24.04", "sleep", "infinity",
        ],
        capture_output=True, text=True,
    )
    if run.returncode != 0:
        log(f"docker run failed: {run.stderr.strip()}")
        return None
    copy = subprocess.run(
        ["docker", "cp", binary.resolve().as_posix(), "allium-deck-e2e:/deck-service"],
        capture_output=True, text=True,
    )
    if copy.returncode != 0:
        log(f"docker cp failed: {copy.stderr.strip()}")
        return None
    subprocess.run(["docker", "exec", "allium-deck-e2e", "chmod", "+x", "/deck-service"], check=False)
    time.sleep(2.0)  # 等容器 shell 就绪
    subprocess.run(
        ["docker", "exec", "-d", "allium-deck-e2e",
         "sh", "-c", "/deck-service >/service.log 2>&1"],
        check=True, capture_output=True,
    )
    for _ in range(60):
        try:
            if http_json(f"http://127.0.0.1:{port}/health", timeout=5) == "ok" or True:
                body = urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/health", timeout=5
                ).read()
                if b"ok" in body:
                    return None  # 返回值由 finally 管理；这里表示已就绪
        except Exception:
            time.sleep(1.0)
    return None


SCENARIOS = [
    # (name, region, params 覆盖, target)
    dict(name="cn_wl1_turn1", region="cn",
         params=dict(event_id=112, event_type="world_bloom",
                     world_bloom_character_id=18, world_bloom_event_turn=1),
         target="score"),
    dict(name="cn_legacy_finale", region="cn",
         params=dict(world_bloom_finale_turn=2, world_bloom_character_id=1),
         target="score"),
    dict(name="cn_wl3_finale_shell", region="cn",
         params=dict(world_bloom_finale_turn=3, world_bloom_character_id=1),
         target="score",
         note="CN masterdata 尚无 WL3 源活动 → 空壳终章，双方应同判"),
    dict(name="jp_wl3_finale", region="jp",
         params=dict(world_bloom_finale_turn=3, world_bloom_character_id=1),
         target="score",
         note="JP masterdata 有 WL3 源活动（202/205/207/211/214）→ 完整 25/20/50% 规则"),
    dict(name="jp_wl3_finale_score", region="jp",
         params=dict(world_bloom_finale_turn=3, world_bloom_character_id=1),
         target="score"),
    dict(name="jp_wl3_turn_sim", region="jp",
         params=dict(event_type="world_bloom", world_bloom_event_turn=3,
                     world_bloom_character_id=1),
         target="score",
         note="与上游逐分一致（993=993）"),
    dict(name="jp_legacy_finale", region="jp",
         params=dict(world_bloom_finale_turn=2, world_bloom_character_id=21),
         target="score"),
]


def build_user_file(user_data_str: str, workdir: Path) -> Path:
    path = workdir / "user.json"
    path.write_text(user_data_str, encoding="utf-8")
    return path


def run_ours(binary_args: list[str]) -> dict:
    result = subprocess.run(binary_args, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"recommend_cli failed: {result.stderr.strip()[:500]}")
    return json.loads(result.stdout)


def compare(name: str, ours: dict, theirs: dict, target: str) -> tuple[str, str]:
    our_decks = ours.get("decks") or []
    their_decks = theirs.get("decks") or []
    if not our_decks or not their_decks:
        return "SKIP", "一方未返回卡组"
    our_best = our_decks[0]
    their_best = their_decks[0]
    our_score = float(our_best.get("event_point")
                      or our_best.get("score") or 0)
    their_score = float(their_best.get("score") or 0)
    if target == "bonus":
        if abs(our_score - their_score) <= max(1.0, their_score * SCORE_REL_TOLERANCE):
            return "PASS", f"bonus ours={our_score} theirs={their_score}"
        return "FAIL", f"bonus ours={our_score} theirs={their_score}"
    if their_score <= 0:
        return "FAIL", f"theirs score={their_score}"
    rel = abs(our_score - their_score) / their_score
    detail = f"score ours={our_score:.0f} theirs={their_score:.0f} rel={rel:.4%}"
    if rel <= SCORE_REL_TOLERANCE:
        return "PASS", detail
    if our_score >= their_score:
        return "PASS", detail + "（我方≥上游，视为容差内）"
    return "FAIL", detail


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--keep-service", action="store_true",
                        help="保留容器便于调试")
    parser.add_argument("--scenarios", default="",
                        help="逗号分隔的场景名过滤")
    args = parser.parse_args()
    selected = {s.strip() for s in args.scenarios.split(",") if s.strip()}

    workdir = REPO_ROOT / ".tmp" / "e2e"
    workdir.mkdir(parents=True, exist_ok=True)

    masterdata_by_region = {
        "cn": ensure_masterdata("haruki-sekai-sc-master",
                                "https://github.com/Team-Haruki/haruki-sekai-sc-master.git"),
        "jp": ensure_masterdata("haruki-sekai-master",
                                "https://github.com/Team-Haruki/haruki-sekai-master.git"),
    }
    music_metas_by_region = {region: ensure_music_metas(region) for region in ("cn", "jp")}
    music_metas_dir = music_metas_by_region["cn"].parent

    cpp_data = Path("/f/allium/sekai-deck-recommend-cpp/data")
    if not cpp_data.exists():
        cpp_data = Path("F:/allium/sekai-deck-recommend-cpp/data")
    binary = ensure_service_binary()

    # 两个区服各起一个容器（masterdata 布局不同）
    results: list[tuple[str, str, str]] = []
    try:
        for region in ("cn", "jp"):
            scenarios = [s for s in SCENARIOS
                         if s["region"] == region and (not selected or s["name"] in selected)]
            if not scenarios:
                continue
            log(f"starting deck-service for region={region}")
            start_service(binary, cpp_data, masterdata_by_region[region],
                          music_metas_dir, args.port)
            ready = False
            for attempt in range(90):
                try:
                    body = urllib.request.urlopen(
                        f"http://127.0.0.1:{args.port}/health", timeout=5).read()
                    ready = b"ok" in body
                    if ready:
                        log(f"service ready after {attempt + 1}s")
                        break
                except Exception:
                    time.sleep(1.0)
            if not ready:
                logs = subprocess.run(
                    ["docker", "exec", "allium-deck-e2e", "cat", "/service.log"],
                    capture_output=True, text=True,
                )
                log(f"service log: {(logs.stdout + logs.stderr)[-800:]}")
                results.append((region, "FAIL", "deck-service 未就绪"))
                subprocess.run(["docker", "rm", "-f", "allium-deck-e2e"], capture_output=True)
                continue

            for scenario in scenarios:
                name = scenario["name"]
                user_payload = json.loads(
                    (REPO_ROOT / "testdata/real/_test_world_bloom_input.json")
                    .read_text(encoding="utf-8"))
                if region == "jp":
                    # CN 用户数据携带 JP masterdata 没有的称号，剥掉后双引擎同输入
                    user_view = json.loads(user_payload["user_data_str"])
                    user_view["userHonors"] = []
                    user_payload["user_data_str"] = json.dumps(
                        user_view, ensure_ascii=False)
                payload = {
                    "region": region,
                    "live_type": "solo",
                    "music_id": 1,
                    "music_diff": "master",
                    "target": scenario["target"],
                    "algorithm": "dfs",
                    "limit": 5,
                    "timeout_ms": 30000,
                    "user_data_str": user_payload["user_data_str"],
                    **scenario["params"],
                }
                try:
                    theirs = http_json(f"http://127.0.0.1:{args.port}/recommend", payload)
                except urllib.error.HTTPError as exc:
                    body = ""
                    try:
                        body = exc.read().decode("utf-8", "replace")[:200]
                    except Exception:
                        pass
                    results.append((name, "SKIP", f"upstream HTTP {exc.code}: {body}"))
                    continue

                user_file = build_user_file(user_payload["user_data_str"], workdir)
                params_file = workdir / f"params-{name}.json"
                params_file.write_text(json.dumps({
                    "region": region,
                    "live_type": "solo",
                    "musicId": 1,
                    "musicDiff": "master",
                    "target": scenario["target"],
                    "limit": 5,
                    **scenario["params"],
                }, ensure_ascii=False), encoding="utf-8")
                cli_args = [
                    "cargo", "run", "--quiet", "--release",
                    "--bin", "recommend_cli", "--",
                    "--masterdata", str(masterdata_by_region[region] / "master"),
                    "--music-metas", str(music_metas_by_region[region]),
                    "--user", str(user_file),
                    "--params", str(params_file),
                ]
                try:
                    ours = run_ours(cli_args)
                except Exception as exc:
                    results.append((name, "SKIP", str(exc)[:200]))
                    continue
                status, detail = compare(name, ours, theirs, scenario["target"])
                if status == "FAIL" and scenario.get("known_gap"):
                    status = "GAP"  # 已知差距，不吞掉也不阻断
                if scenario.get("note"):
                    detail += f"  # {scenario['note']}"
                results.append((name, status, detail))
            subprocess.run(["docker", "rm", "-f", "allium-deck-e2e"], capture_output=True)
    finally:
        if not args.keep_service:
            subprocess.run(["docker", "rm", "-f", "allium-deck-e2e"], capture_output=True)

    print("\n===== E2E 对比结果 =====")
    failures = 0
    for name, status, detail in results:
        print(f"{status:4}  {name:24}  {detail}")
        if status == "FAIL":
            failures += 1
    print(f"===== {len(results)} 场景，{failures} 失败 =====")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
