from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock
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
    def test_release_serializes_attempts_for_each_tag(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn("concurrency:", workflow)
        self.assertIn("group: release-${{ github.ref }}", workflow)
        self.assertIn("scripts/verify_crates_checksum.py", workflow)

    def test_release_changelog_uses_supported_git_cliff_arguments(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertNotIn("--first-parent", workflow)

    def test_release_ci_installs_required_rust_components(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn("components: rustfmt, clippy", workflow)
        self.assertIn("cargo clippy --manifest-path wasm/Cargo.toml --all-targets", workflow)
        self.assertIn("cargo test --manifest-path wasm/Cargo.toml --all-targets --release", workflow)

    def test_registry_preflight_rejects_malformed_secret_without_printing_it(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn('[[ "$CARGO_REGISTRY_TOKEN" =~ ^cio[[:alnum:]]{32}$ ]]', workflow)
        self.assertIn("must contain only the raw crates.io token", workflow)

    def test_registry_preflight_does_not_use_cookie_only_me_endpoint(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertNotIn("https://crates.io/api/v1/me", workflow)

    def test_npm_publish_uses_trusted_publishing_without_stored_token(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn("id-token: write", workflow)
        self.assertIn("npm publish --provenance", workflow)
        self.assertNotIn("NODE_AUTH_TOKEN", workflow)
        self.assertNotIn("NPM_TOKEN", workflow)

    def test_npm_wasm_version_comes_from_the_crate_not_masterdata(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn("cargo metadata --manifest-path wasm/Cargo.toml", workflow)
        self.assertNotIn("MASTERDATA_VERSION", workflow)
        self.assertNotIn("release-inputs", workflow)
        self.assertNotIn("download_masterdata", workflow)

    def test_smoke_uses_external_masterdata_and_no_embedded_export(self) -> None:
        smoke = (SCRIPTS / "smoke_wasm_package.mjs").read_text(encoding="utf-8")
        self.assertIn('from "@empty-sekai/allium-deck-wasm"', smoke)
        self.assertIn("load_masterdata", smoke)
        self.assertIn("recommend", smoke)
        self.assertNotIn("recommend_embedded", smoke)
        self.assertNotIn('"node_modules"', smoke)

    def test_all_wasm_workflows_run_smoke_from_consumer_root(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn("NPM_PACKAGE=$(realpath", workflow)
        self.assertIn('npm install --prefix "$INSTALL_ROOT" "$NPM_PACKAGE"', workflow)
        self.assertIn('cp scripts/smoke_wasm_package.mjs "$INSTALL_ROOT/smoke.mjs"', workflow)
        self.assertIn('node "$INSTALL_ROOT/smoke.mjs"', workflow)

    def test_release_timestamps_use_source_date_epoch(self) -> None:
        packager = load_script("package_wasm")
        previous = os.environ.get("SOURCE_DATE_EPOCH")
        os.environ["SOURCE_DATE_EPOCH"] = "0"
        try:
            self.assertEqual(packager.utc_now(), "1970-01-01T00:00:00Z")
        finally:
            if previous is None:
                os.environ.pop("SOURCE_DATE_EPOCH", None)
            else:
                os.environ["SOURCE_DATE_EPOCH"] = previous

    def test_package_manifest_has_no_cdn_or_masterdata(self) -> None:
        source = Path(SCRIPTS / "package_wasm.py").read_text(encoding="utf-8")
        self.assertNotIn("masterdata_version", source)
        self.assertNotIn("cdn_base", source)
        self.assertNotIn("--cdn-base", source)
        self.assertNotIn("--masterdata-manifest", source)

    def test_crates_checksum_poll_recovers_after_registry_delay(self) -> None:
        verifier = load_script("verify_crates_checksum")
        with mock.patch.object(
            verifier,
            "query_checksum",
            side_effect=[None, None, "a" * 64],
        ), mock.patch.object(verifier.time, "sleep"):
            verifier.wait_for_matching_checksum(
                "allium-deck",
                "0.0.4",
                "a" * 64,
                attempts=3,
                delay=0,
            )

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
