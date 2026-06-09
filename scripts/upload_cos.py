#!/usr/bin/env python3
"""Upload a directory to Tencent COS."""

from __future__ import annotations

import argparse
import mimetypes
import os
from pathlib import Path
from urllib.parse import urlparse


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


def env_or_arg(value: str | None, *env_names: str) -> str:
    resolved = value or ""
    for env_name in env_names:
        if resolved:
            break
        resolved = os.environ.get(env_name, "")
    if not resolved:
        raise RuntimeError(f"missing {'/'.join(env_names)}")
    return resolved


def region_from_endpoint(endpoint: str) -> str:
    host = urlparse(endpoint).netloc or endpoint
    parts = host.split(".")
    if len(parts) >= 3 and parts[0] == "cos":
        return parts[1]
    return ""


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dir", required=True)
    parser.add_argument("--prefix", required=True)
    parser.add_argument("--bucket")
    parser.add_argument("--region")
    parser.add_argument("--secret-id")
    parser.add_argument("--secret-key")
    parser.add_argument("--session-token", default=os.environ.get("COS_SESSION_TOKEN", "") or os.environ.get("S3_SESSION_TOKEN", ""))
    parser.add_argument("--cache-control", default="public,max-age=300")
    args = parser.parse_args()

    source_dir = Path(args.dir)
    if not source_dir.is_dir():
        raise RuntimeError(f"not a directory: {source_dir}")

    bucket = env_or_arg(args.bucket, "COS_BUCKET", "S3_BUCKET")
    region = args.region or os.environ.get("COS_REGION", "") or os.environ.get("S3_REGION", "")
    if not region:
        region = region_from_endpoint(os.environ.get("S3_ENDPOINT", ""))
    if not region:
        raise RuntimeError("missing COS_REGION/S3_REGION")
    secret_id = env_or_arg(args.secret_id, "COS_SECRET_ID", "S3_ACCESS_KEY_ID")
    secret_key = env_or_arg(args.secret_key, "COS_SECRET_KEY", "S3_ACCESS_KEY_SECRET")

    try:
        from qcloud_cos import CosConfig, CosS3Client
    except ImportError as exc:
        raise RuntimeError("qcloud_cos is not installed; run `pip install cos-python-sdk-v5`") from exc

    config = CosConfig(
        Region=region,
        SecretId=secret_id,
        SecretKey=secret_key,
        Token=args.session_token or None,
        Scheme="https",
    )
    client = CosS3Client(config)
    prefix = args.prefix.strip("/")

    uploaded = 0
    for file in sorted(path for path in source_dir.rglob("*") if path.is_file()):
        rel = file.relative_to(source_dir).as_posix()
        key = f"{prefix}/{rel}"
        client.upload_file(
            Bucket=bucket,
            LocalFilePath=str(file),
            Key=key,
            EnableMD5=True,
            CacheControl=args.cache_control,
            ContentType=content_type(file),
        )
        uploaded += 1
        print(f"[upload] cos://{bucket}/{key}")

    print(f"[upload] {uploaded} files")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}")
        raise SystemExit(1)
