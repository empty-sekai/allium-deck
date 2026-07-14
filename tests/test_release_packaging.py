from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))


def load_script(name: str):
    path = SCRIPTS / f"{name}.py"
    if not path.exists():
        raise AssertionError(f"missing release helper: {path}")
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleasePackagingTests(unittest.TestCase):
    def test_masterdata_url_uses_immutable_version(self) -> None:
        downloader = load_script("download_masterdata")
        self.assertEqual(
            downloader.table_url("https://cdn.example", "cn", "6.0.0.36", "cards"),
            "https://cdn.example/masterdata/cn/6.0.0.36/cards.json",
        )

    def test_snapshot_guard_rejects_changed_content(self) -> None:
        downloader = load_script("download_masterdata")
        self.assertTrue(hasattr(downloader, "ensure_same_snapshot"))
        with self.assertRaisesRegex(RuntimeError, "changed during download"):
            downloader.ensure_same_snapshot(b"before", b"after", "music_metas.json")

    def test_release_timestamps_use_source_date_epoch(self) -> None:
        downloader = load_script("download_masterdata")
        packager = load_script("package_wasm")
        previous = os.environ.get("SOURCE_DATE_EPOCH")
        os.environ["SOURCE_DATE_EPOCH"] = "0"
        try:
            self.assertEqual(downloader.utc_now(), "1970-01-01T00:00:00Z")
            self.assertEqual(packager.utc_now(), "1970-01-01T00:00:00Z")
        finally:
            if previous is None:
                os.environ.pop("SOURCE_DATE_EPOCH", None)
            else:
                os.environ["SOURCE_DATE_EPOCH"] = previous

    def test_zip_bytes_are_reproducible(self) -> None:
        zipper = load_script("create_reproducible_zip")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            (source / "b.txt").write_text("b", encoding="utf-8")
            (source / "a.txt").write_text("a", encoding="utf-8")
            first = root / "first.zip"
            second = root / "second.zip"
            zipper.create_zip(source, first, 315532800)
            time.sleep(0.01)
            os.utime(source / "a.txt", (1_700_000_000, 1_700_000_000))
            zipper.create_zip(source, second, 315532800)
            self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_zip_cli_accepts_explicit_epoch_without_environment(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            (source / "file.txt").write_text("content", encoding="utf-8")
            env = os.environ.copy()
            env.pop("SOURCE_DATE_EPOCH", None)
            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "create_reproducible_zip.py"),
                    "--source-dir",
                    str(source),
                    "--output",
                    str(root / "out.zip"),
                    "--source-date-epoch",
                    "315532800",
                ],
                check=True,
                env=env,
            )


if __name__ == "__main__":
    unittest.main()
