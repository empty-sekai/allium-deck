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
    def test_tag_release_uses_repository_pinned_inputs(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        release_inputs = ROOT / "release-inputs" / "v0.0.4.json"
        self.assertTrue(release_inputs.is_file())
        inputs = __import__("json").loads(release_inputs.read_text(encoding="utf-8"))
        self.assertEqual(inputs["masterdata_version"], "6.0.0.36")
        self.assertRegex(inputs["music_metas_sha256"], r"^[0-9a-f]{64}$")
        next_inputs = __import__("json").loads(
            (ROOT / "release-inputs" / "v0.0.5.json").read_text(encoding="utf-8")
        )
        self.assertEqual(next_inputs, inputs)
        self.assertIn('release-inputs/${GITHUB_REF_NAME}.json', workflow)
        self.assertNotIn("deck-wasm/cn/latest/manifest.json", workflow)

    def test_expected_music_checksum_rejects_mismatch(self) -> None:
        downloader = load_script("download_masterdata")
        with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
            downloader.ensure_expected_checksum(b"actual", "0" * 64, "music_metas.json")

    def test_snapshot_mode_requires_checksum_only_for_pinned_builds(self) -> None:
        downloader = load_script("download_masterdata")
        with self.assertRaisesRegex(RuntimeError, "requires --expected-music-sha256"):
            downloader.validate_snapshot_mode("pinned", "")
        downloader.validate_snapshot_mode("latest", "")

    def test_cnb_release_resolver_loads_tag_controlled_inputs(self) -> None:
        resolver = load_script("resolve_wasm_inputs")
        values = resolver.load_release_inputs(ROOT, "v0.0.4")
        self.assertEqual(values["masterdata_version"], "6.0.0.36")
        self.assertRegex(values["music_metas_sha256"], r"^[0-9a-f]{64}$")

    def test_every_masterdata_download_declares_snapshot_mode(self) -> None:
        for path in (
            ROOT / ".cnb.yml",
            ROOT / ".github" / "workflows" / "build-wasm.yml",
            ROOT / ".github" / "workflows" / "release.yml",
        ):
            content = path.read_text(encoding="utf-8")
            for invocation in content.split("python3 scripts/download_masterdata.py")[1:]:
                self.assertIn("--snapshot-mode", invocation.split("\n\n", 1)[0], path.name)

    def test_cnb_build_uses_standalone_wasm_workspace_member(self) -> None:
        workflow = (ROOT / ".cnb.yml").read_text(encoding="utf-8")
        self.assertIn("cargo run --release --manifest-path wasm/Cargo.toml --bin build_gamedata", workflow)
        self.assertIn("wasm-pack build wasm --target web --scope empty-sekai", workflow)
        self.assertIn("--pkg-dir wasm/pkg", workflow)

    def test_cnb_release_uses_reproducible_zip_and_source_epoch(self) -> None:
        workflow = (ROOT / ".cnb.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/create_reproducible_zip.py", workflow)
        self.assertNotIn("zip -9 -r", workflow)
        resolver = load_script("resolve_wasm_inputs")
        with mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1234567890"}):
            self.assertEqual(resolver.resolve_source_date_epoch("deadbeef"), "1234567890")

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

    def test_npm_smoke_uses_bare_package_import_and_requires_expected_error(self) -> None:
        smoke = (SCRIPTS / "smoke_wasm_package.mjs").read_text(encoding="utf-8")
        self.assertIn('from "@empty-sekai/allium-deck-wasm"', smoke)
        self.assertNotIn('"node_modules"', smoke)
        self.assertIn('throw new Error("recommend_embedded unexpectedly succeeded")', smoke)

    def test_all_wasm_workflows_run_smoke_from_consumer_root(self) -> None:
        for name in ("build-wasm.yml", "release.yml"):
            workflow = (ROOT / ".github" / "workflows" / name).read_text(encoding="utf-8")
            self.assertIn("NPM_PACKAGE=$(realpath", workflow)
            self.assertIn('npm install --prefix "$INSTALL_ROOT" "$NPM_PACKAGE"', workflow)
            self.assertIn('cp scripts/smoke_wasm_package.mjs "$INSTALL_ROOT/smoke.mjs"', workflow)
            self.assertIn('node "$INSTALL_ROOT/smoke.mjs"', workflow)

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
