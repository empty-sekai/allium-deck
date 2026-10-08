#!/usr/bin/env python3
"""Read-only validation of original tag artifacts before recovering publication."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import stat
import sys
import tarfile
import zipfile
from pathlib import Path, PurePosixPath


REPOSITORY = "empty-sekai/allium-deck"
PACKAGE = "@empty-sekai/allium-deck-wasm"
MAX_MEMBER_BYTES = 100 * 1024 * 1024


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def file_hash(path: Path, algorithm: str = "sha256") -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, algorithm).hexdigest()


def safe_name(name: str) -> bool:
    path = PurePosixPath(name)
    return bool(name) and not path.is_absolute() and ".." not in path.parts and "\\" not in name


def release_names(version: str) -> set[str]:
    return {
        *(f"recommend_cli-v{version}-{platform}" for platform in (
            "linux-x86_64", "linux-x86_64-musl", "linux-aarch64",
            "windows-x86_64.exe", "macos-aarch64",
        )),
        f"allium-deck-wasm-v{version}-cn.zip",
        f"allium-deck-wasm-v{version}.tgz",
    }


def validate_run(run: dict, artifacts: dict, repository: str, tag: str, revision: str) -> dict:
    require(run.get("repository", {}).get("full_name") == repository, "source run repository mismatch")
    require(run.get("event") == "push", "source run must be a push event")
    require(run.get("path") == ".github/workflows/release.yml", "source workflow path mismatch")
    require(run.get("head_branch") == tag, "source run tag mismatch")
    require(run.get("head_sha") == revision, "source run revision mismatch")
    require(run.get("status") == "completed", "source run has not completed")
    require(run.get("conclusion") in ("success", "failure"), "source run conclusion cannot be recovered")
    require(type(run.get("id")) is int and run["id"] > 0, "source run ID is missing")
    repository_id = run.get("repository", {}).get("id")
    require(type(repository_id) is int and repository_id > 0, "source repository ID is missing")
    rows = artifacts.get("artifacts")
    require(isinstance(rows, list), "artifact list is missing")
    selected = [row for row in rows if isinstance(row, dict) and row.get("name") == "release-assets"]
    require(len(selected) == 1, "expected exactly one release-assets artifact")
    artifact = selected[0]
    require(artifact.get("expired") is False, "release-assets is expired or expiration state is missing")
    require(type(artifact.get("id")) is int and artifact["id"] > 0, "artifact ID is missing")
    require(bool(re.fullmatch(r"sha256:[a-f0-9]{64}", str(artifact.get("digest", "")))), "artifact digest is missing or invalid")
    source = artifact.get("workflow_run")
    require(isinstance(source, dict), "artifact workflow-run identity is missing")
    require(source.get("id") == run["id"], "artifact belongs to a different run")
    require(source.get("head_sha") == revision, "artifact revision mismatch")
    require(source.get("head_branch") == tag, "artifact tag mismatch")
    require(source.get("repository_id") == repository_id, "artifact repository mismatch")
    require(source.get("head_repository_id") == repository_id, "artifact head repository mismatch")
    return artifact


def validate_checksums(directory: Path, version: str) -> None:
    expected = release_names(version)
    actual = {entry.name for entry in directory.iterdir()}
    require(actual == expected | {"SHA256SUMS"}, "release asset filenames do not match the complete release set")
    sums = directory / "SHA256SUMS"
    require(sums.is_file() and not sums.is_symlink(), "SHA256SUMS must be a regular file")
    recorded: dict[str, str] = {}
    for line in sums.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([a-fA-F0-9]{64}) [ *](.+)", line)
        require(match is not None, "invalid SHA256SUMS row")
        digest, name = match.groups()
        require(name in expected and name not in recorded, "unexpected or duplicate SHA256SUMS filename")
        recorded[name] = digest.lower()
    require(set(recorded) == expected, "SHA256SUMS does not cover every release asset")
    for name in expected:
        path = directory / name
        require(path.is_file() and not path.is_symlink() and path.stat().st_size > 0,
                f"release asset is not a nonempty regular file: {name}")
        require(file_hash(path) == recorded[name], f"release checksum mismatch: {name}")


def read_browser_bundle(path: Path, version: str, repository: str, revision: str) -> dict[str, bytes]:
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        names = [entry.filename for entry in entries]
        require(len(names) == len(set(names)), "duplicate ZIP entry")
        require(all(safe_name(name) and PurePosixPath(name).name == name for name in names), "invalid browser bundle entry path")
        require(all(not entry.is_dir() and not stat.S_ISLNK(entry.external_attr >> 16)
                    and entry.file_size <= MAX_MEMBER_BYTES for entry in entries), "invalid browser bundle member")
        content = {name: archive.read(name) for name in names}
    require("manifest.json" in content, "browser manifest is missing")
    manifest = json.loads(content["manifest.json"])
    require(manifest.get("name") == "allium-deck-wasm", "browser manifest package mismatch")
    require(manifest.get("version") == version, "browser manifest version mismatch")
    require(manifest.get("source") == {"repository": repository, "revision": revision}, "browser manifest source mismatch")
    require(manifest.get("entrypoint") == "allium_deck.js" and manifest.get("wasm") == "allium_deck_bg.wasm", "browser manifest entrypoint mismatch")
    files = manifest.get("files")
    require(isinstance(files, list), "browser manifest file list is missing")
    declared: set[str] = set()
    for entry in files:
        require(isinstance(entry, dict), "invalid browser manifest file row")
        name = entry.get("name")
        require(isinstance(name, str) and name not in declared and name in content and name != "manifest.json", "duplicate or missing browser manifest file")
        declared.add(name)
        require(type(entry.get("bytes")) is int and entry["bytes"] == len(content[name]), f"browser file size mismatch: {name}")
        require(entry.get("sha256") == hashlib.sha256(content[name]).hexdigest(), f"browser file checksum mismatch: {name}")
    require(declared == set(content) - {"manifest.json"}, "browser manifest does not cover every archive file")
    return content


def read_npm_tarball(path: Path) -> dict[str, bytes]:
    with tarfile.open(path, "r:gz") as archive:
        entries = archive.getmembers()
        names = [entry.name for entry in entries]
        require(len(names) == len(set(names)), "duplicate npm tarball entry")
        require(all(safe_name(name) and name.startswith("package/") for name in names), "invalid npm tarball entry path")
        require(all(entry.isfile() and entry.size <= MAX_MEMBER_BYTES for entry in entries), "npm tarball contains a non-regular or oversized entry")
        return {entry.name.removeprefix("package/"): archive.extractfile(entry).read() for entry in entries}


def validate_recovery(*, tag: str, source_sha: str, run: dict, artifacts: dict,
                      asset_dir: Path, repository: str = REPOSITORY) -> dict:
    require(repository == REPOSITORY, "unsupported recovery repository")
    require(bool(re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", tag)), "invalid release tag")
    require(bool(re.fullmatch(r"[a-f0-9]{40}", source_sha)), "source-sha must be a full commit hash")
    version = tag[1:]
    artifact = validate_run(run, artifacts, repository, tag, source_sha)
    validate_checksums(asset_dir, version)
    browser = read_browser_bundle(asset_dir / f"allium-deck-wasm-{tag}-cn.zip", version, repository, source_sha)
    tarball = asset_dir / f"allium-deck-wasm-{tag}.tgz"
    npm = read_npm_tarball(tarball)
    for name in ("package.json", "allium_deck.js", "allium_deck_bg.wasm"):
        require(name in browser and name in npm and browser[name] == npm[name], f"ZIP/npm content mismatch: {name}")
    metadata = json.loads(npm["package.json"])
    require(metadata.get("name") == PACKAGE and metadata.get("version") == version, "npm package identity mismatch")
    require(metadata.get("repository") == {"type": "git", "url": f"git+https://github.com/{repository}.git"}, "npm package repository mismatch")
    with tarball.open("rb") as handle:
        sri = "sha512-" + base64.b64encode(hashlib.file_digest(handle, "sha512").digest()).decode("ascii")
    return {
        "repository": repository, "tag": tag, "revision": source_sha, "version": version,
        "runId": run["id"], "artifactId": artifact["id"], "artifactDigest": artifact["digest"],
        "wasmSha256": hashlib.sha256(npm["allium_deck_bg.wasm"]).hexdigest(),
        "tgzSha256": file_hash(tarball), "tgzSRI": sri,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--run-json", required=True, type=Path)
    parser.add_argument("--artifacts-json", required=True, type=Path)
    parser.add_argument("--asset-dir", required=True, type=Path)
    parser.add_argument("--repository", default=REPOSITORY, choices=[REPOSITORY])
    args = parser.parse_args()
    result = validate_recovery(tag=args.tag, source_sha=args.source_sha,
        run=json.loads(args.run_json.read_text(encoding="utf-8-sig")),
        artifacts=json.loads(args.artifacts_json.read_text(encoding="utf-8-sig")),
        asset_dir=args.asset_dir, repository=args.repository)
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
