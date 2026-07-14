#!/usr/bin/env python3
"""Create a deterministic ZIP archive from a directory."""

from __future__ import annotations

import argparse
import os
import zipfile
from datetime import datetime, timezone
from pathlib import Path


ZIP_EPOCH = 315532800  # 1980-01-01, the earliest timestamp supported by ZIP.


def create_zip(source_dir: Path, output: Path, source_date_epoch: int) -> None:
    timestamp = max(source_date_epoch, ZIP_EPOCH)
    date_time = datetime.fromtimestamp(timestamp, timezone.utc).timetuple()[:6]
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(
        output,
        "w",
        compression=zipfile.ZIP_DEFLATED,
        compresslevel=9,
    ) as archive:
        for path in sorted(item for item in source_dir.rglob("*") if item.is_file()):
            relative = path.relative_to(source_dir).as_posix()
            info = zipfile.ZipInfo(relative, date_time=date_time)
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, path.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--source-date-epoch",
        type=int,
        default=int(os.environ.get("SOURCE_DATE_EPOCH", ZIP_EPOCH)),
    )
    args = parser.parse_args()
    create_zip(args.source_dir, args.output, args.source_date_epoch)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
