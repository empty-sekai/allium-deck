#!/usr/bin/env python3
"""Upload a directory to Tencent COS."""

from __future__ import annotations

import argparse
import mimetypes
import os
import time
from pathlib import Path
from urllib.parse import urlparse

SINGLE_PUT_LIMIT = 64 * 1024 * 1024
MAX_UPLOAD_ATTEMPTS = 4


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


def normalize_s3_endpoint(endpoint: str, bucket: str) -> str:
    parsed = urlparse(endpoint)
    if not parsed.scheme or not parsed.netloc:
        return endpoint
    prefix = f"{bucket}."
    if parsed.netloc.startswith(prefix):
        return parsed._replace(netloc=parsed.netloc[len(prefix) :]).geturl()
    return endpoint


def upload_one(client, *, bucket: str, file: Path, key: str, cache_control: str) -> None:
    size = file.stat().st_size
    kwargs = {
        "Bucket": bucket,
        "Key": key,
        "EnableMD5": True,
        "CacheControl": cache_control,
        "ContentType": content_type(file),
    }
    for attempt in range(1, MAX_UPLOAD_ATTEMPTS + 1):
        try:
            with file.open("rb") as body:
                if size <= SINGLE_PUT_LIMIT:
                    client.put_object(Body=body, ContentLength=size, **kwargs)
                else:
                    client.upload_file(LocalFilePath=str(file), **kwargs)
            return
        except Exception:
            if attempt >= MAX_UPLOAD_ATTEMPTS:
                raise
            time.sleep(min(2 ** (attempt - 1), 8))


def upload_with_boto3(
    *,
    source_dir: Path,
    prefix: str,
    bucket: str,
    endpoint: str,
    region: str,
    secret_id: str,
    secret_key: str,
    session_token: str,
    cache_control: str,
    addressing_style: str,
) -> int:
    try:
        import boto3
        from botocore.config import Config
    except ImportError as exc:
        raise RuntimeError("boto3 is not installed; run `pip install boto3`") from exc

    client = boto3.client(
        "s3",
        endpoint_url=endpoint,
        region_name=region,
        aws_access_key_id=secret_id,
        aws_secret_access_key=secret_key,
        aws_session_token=session_token or None,
        config=Config(
            connect_timeout=10,
            read_timeout=180,
            retries={"max_attempts": MAX_UPLOAD_ATTEMPTS, "mode": "standard"},
            s3={"addressing_style": addressing_style},
        ),
    )

    uploaded = 0
    for file in sorted(path for path in source_dir.rglob("*") if path.is_file()):
        rel = file.relative_to(source_dir).as_posix()
        key = f"{prefix}/{rel}"
        for attempt in range(1, MAX_UPLOAD_ATTEMPTS + 1):
            try:
                with file.open("rb") as body:
                    client.put_object(
                        Bucket=bucket,
                        Key=key,
                        Body=body,
                        ContentLength=file.stat().st_size,
                        ContentType=content_type(file),
                        CacheControl=cache_control,
                    )
                break
            except Exception:
                if attempt >= MAX_UPLOAD_ATTEMPTS:
                    raise
                time.sleep(min(2 ** (attempt - 1), 8))
        uploaded += 1
        print(f"[upload] s3://{bucket}/{key}")
    return uploaded


def upload_with_qcloud(
    *,
    source_dir: Path,
    prefix: str,
    bucket: str,
    region: str,
    secret_id: str,
    secret_key: str,
    session_token: str,
    cache_control: str,
) -> int:
    try:
        from qcloud_cos import CosConfig, CosS3Client
    except ImportError as exc:
        raise RuntimeError("qcloud_cos is not installed; run `pip install cos-python-sdk-v5`") from exc

    config = CosConfig(
        Region=region,
        SecretId=secret_id,
        SecretKey=secret_key,
        Token=session_token or None,
        Scheme="https",
        Timeout=180,
    )
    client = CosS3Client(config)

    uploaded = 0
    for file in sorted(path for path in source_dir.rglob("*") if path.is_file()):
        rel = file.relative_to(source_dir).as_posix()
        key = f"{prefix}/{rel}"
        upload_one(
            client,
            bucket=bucket,
            file=file,
            key=key,
            cache_control=cache_control,
        )
        uploaded += 1
        print(f"[upload] cos://{bucket}/{key}")
    return uploaded


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
    parser.add_argument(
        "--backend",
        choices=["auto", "qcloud", "s3"],
        default=os.environ.get("UPLOAD_BACKEND", "auto"),
        help="upload backend; auto uses S3 endpoint first when configured, then falls back to qcloud",
    )
    parser.add_argument(
        "--s3-addressing-style",
        choices=["virtual", "path", "auto"],
        default=os.environ.get("S3_ADDRESSING_STYLE", os.environ.get("COS_ADDRESSING_STYLE", "auto")),
    )
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
    endpoint = os.environ.get("S3_ENDPOINT", "").strip()

    prefix = args.prefix.strip("/")
    backend = args.backend
    if backend == "auto":
        backend = "s3" if endpoint else "qcloud"

    if backend == "s3":
        if not endpoint:
            raise RuntimeError("S3 backend requires S3_ENDPOINT")
        endpoint = normalize_s3_endpoint(endpoint, bucket)
        addressing_styles = (
            ["virtual", "path"] if args.s3_addressing_style == "auto" else [args.s3_addressing_style]
        )
        last_error: Exception | None = None
        for addressing_style in addressing_styles:
            try:
                uploaded = upload_with_boto3(
                    source_dir=source_dir,
                    prefix=prefix,
                    bucket=bucket,
                    endpoint=endpoint,
                    region=region,
                    secret_id=secret_id,
                    secret_key=secret_key,
                    session_token=args.session_token,
                    cache_control=args.cache_control,
                    addressing_style=addressing_style,
                )
                print(f"[upload] {uploaded} files")
                return 0
            except Exception as exc:
                last_error = exc
                if addressing_style == addressing_styles[-1]:
                    break
                print(f"[upload] retrying with S3 addressing_style={addressing_styles[-1]} after: {exc}")
        if args.backend == "s3":
            raise last_error or RuntimeError("S3 upload failed")
        print(f"[upload] falling back to qcloud COS SDK after S3 failure: {last_error}")

    if backend == "qcloud" or args.backend == "auto":
        uploaded = upload_with_qcloud(
            source_dir=source_dir,
            prefix=prefix,
            bucket=bucket,
            region=region,
            secret_id=secret_id,
            secret_key=secret_key,
            session_token=args.session_token,
            cache_control=args.cache_control,
        )
        print(f"[upload] {uploaded} files")
        return 0

    raise RuntimeError(f"unsupported backend: {backend}")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}")
        raise SystemExit(1)
