#!/usr/bin/env python3
"""Create/update a GitHub Release and upload assets."""

from __future__ import annotations

import argparse
import json
import mimetypes
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


API_BASE = "https://api.github.com"
API_VERSION = "2022-11-28"


class GitHubError(RuntimeError):
    def __init__(self, status: int, body: str):
        super().__init__(f"GitHub API HTTP {status}: {body[:500]}")
        self.status = status
        self.body = body


def env_first(*names: str) -> str:
    for name in names:
        value = os.environ.get(name, "").strip()
        if value:
            return value
    return ""


def request_json(token: str, method: str, url: str, payload: dict | None = None) -> dict:
    data = None
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": API_VERSION,
        "User-Agent": "allium-deck-cnb-release",
    }
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            body = response.read()
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        raise GitHubError(exc.code, body) from exc
    if not body:
        return {}
    return json.loads(body)


def request_empty(token: str, method: str, url: str) -> None:
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": API_VERSION,
        "User-Agent": "allium-deck-cnb-release",
    }
    request = urllib.request.Request(url, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            response.read()
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        raise GitHubError(exc.code, body) from exc


def upload_asset(token: str, upload_url: str, asset: Path) -> None:
    content_type = mimetypes.guess_type(asset.name)[0] or "application/octet-stream"
    endpoint = upload_url.split("{", 1)[0]
    url = f"{endpoint}?name={urllib.parse.quote(asset.name)}"
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": API_VERSION,
        "User-Agent": "allium-deck-cnb-release",
        "Content-Type": content_type,
        "Content-Length": str(asset.stat().st_size),
    }
    request = urllib.request.Request(url, data=asset.read_bytes(), headers=headers, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            response.read()
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        raise GitHubError(exc.code, body) from exc


def get_release(token: str, repo: str, tag: str) -> dict | None:
    url = f"{API_BASE}/repos/{repo}/releases/tags/{urllib.parse.quote(tag, safe='')}"
    try:
        return request_json(token, "GET", url)
    except GitHubError as exc:
        if exc.status == 404:
            return None
        raise


def ensure_release(token: str, repo: str, tag: str, title: str, body: str, prerelease: bool) -> dict:
    release = get_release(token, repo, tag)
    if release is not None:
        return release

    try:
        return request_json(
            token,
            "POST",
            f"{API_BASE}/repos/{repo}/releases",
            {
                "tag_name": tag,
                "name": title or tag,
                "body": body,
                "draft": False,
                "prerelease": prerelease,
            },
        )
    except GitHubError as exc:
        if exc.status == 422:
            release = get_release(token, repo, tag)
            if release is not None:
                return release
        raise


def delete_existing_asset(token: str, release: dict, asset_name: str) -> None:
    for asset in release.get("assets", []):
        if asset.get("name") == asset_name:
            request_empty(token, "DELETE", asset["url"])
            print(f"[github-release] deleted existing asset {asset_name}")
            return


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=env_first("GITHUB_RELEASE_REPO", "GITHUB_REPOSITORY"))
    parser.add_argument("--tag", default=env_first("GITHUB_REF_NAME", "CNB_BRANCH"))
    parser.add_argument("--title", default="")
    parser.add_argument("--body", default="")
    parser.add_argument("--body-file", default="")
    parser.add_argument("--asset", action="append", required=True)
    parser.add_argument("--prerelease", action="store_true")
    parser.add_argument("--clobber", action="store_true")
    args = parser.parse_args()

    token = env_first("GITHUB_RELEASE_TOKEN", "GH_TOKEN", "GITHUB_TOKEN")
    if not token:
        raise RuntimeError("missing GITHUB_RELEASE_TOKEN/GH_TOKEN/GITHUB_TOKEN")
    if not args.repo:
        raise RuntimeError("missing GitHub repo, pass --repo or set GITHUB_RELEASE_REPO")
    if not args.tag:
        raise RuntimeError("missing release tag")

    body = args.body
    if args.body_file:
        body = Path(args.body_file).read_text(encoding="utf-8")
    if not body:
        body = f"Automated release assets for {args.tag}."

    release = ensure_release(
        token,
        args.repo,
        args.tag,
        args.title or args.tag,
        body,
        args.prerelease or "-" in args.tag,
    )

    for asset_arg in args.asset:
        asset = Path(asset_arg)
        if not asset.is_file():
            raise RuntimeError(f"asset not found: {asset}")
        if args.clobber:
            delete_existing_asset(token, release, asset.name)
            release = get_release(token, args.repo, args.tag) or release
        upload_asset(token, release["upload_url"], asset)
        print(f"[github-release] uploaded {asset.name} to {args.repo}@{args.tag}")

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
