from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
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
    def run_npm_verifier_mock(self, assertions: str) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tarball = Path(tmp) / "package.tgz"
            tarball.write_bytes(b"the exact npm package tested for release")
            program = r'''
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
const { verifyPackage, NOT_FOUND } = await import(process.argv[1]);
const tarball = process.argv[2];
const packageName = "@empty-sekai/allium-deck-wasm";
const version = "0.1.0";
const integrity = `sha512-${createHash("sha512").update(readFileSync(tarball)).digest("base64")}`;
const metadata = { name: packageName, version, dist: { integrity } };
const ok = (value = metadata) => ({ status: 200, ok: true, json: async () => value });
const status = (code) => ({ status: code, ok: false });
const options = { packageName, version, tarball, log: () => {}, pause: async () => {} };
''' + assertions
            result = subprocess.run(
                ["node", "--input-type=module", "-e", program,
                 (SCRIPTS / "verify_npm_package.mjs").as_uri(), str(tarball)],
                cwd=ROOT, capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_npm_checksum_accepts_exact_tarball_after_visibility_and_network_delays(self) -> None:
        self.run_npm_verifier_mock(r'''
const replies = [status(404), new Error("temporary connection failure"), status(503), ok()];
let calls = 0;
let pauses = 0;
const result = await verifyPackage({ ...options, attempts: 4,
  pause: async () => { pauses++; },
  fetchImpl: async (url, init) => {
    assert.equal(url, "https://registry.npmjs.org/%40empty-sekai%2Fallium-deck-wasm/0.1.0");
    assert.ok(init.signal);
    const reply = replies[calls++];
    if (reply instanceof Error) throw reply;
    return reply;
  },
});
assert.equal(result, 0);
assert.equal(calls, 4);
assert.equal(pauses, 3);
''')

    def test_npm_checksum_rejects_changed_tarball_and_wrong_identity_without_retry(self) -> None:
        self.run_npm_verifier_mock(r'''
for (const body of [
  { ...metadata, dist: { integrity: "sha512-different" } },
  { ...metadata, dist: { shasum: "older-hash-without-integrity" } },
  { ...metadata, name: "another-package" },
  { ...metadata, version: "0.0.15" },
]) {
  let calls = 0;
  await assert.rejects(verifyPackage({ ...options, attempts: 3,
    fetchImpl: async () => { calls++; return ok(body); },
  }), /mismatch|identity/);
  assert.equal(calls, 1);
}
''')

    def test_npm_checksum_only_404_is_confirmed_absence(self) -> None:
        self.run_npm_verifier_mock(r'''
let absentCalls = 0;
assert.equal(await verifyPackage({ ...options, attempts: 2,
  fetchImpl: async () => { absentCalls++; return status(404); },
}), NOT_FOUND);
assert.equal(absentCalls, 2);
for (const reply of [status(401), status(429), status(500), ok(null), ok([]),
  { status: 200, ok: true, json: async () => { throw new Error("malformed JSON"); } }]) {
  await assert.rejects(verifyPackage({ ...options, attempts: 2,
    fetchImpl: async () => reply,
  }), /HTTP|JSON/);
}
await assert.rejects(verifyPackage({ ...options, attempts: 2,
  fetchImpl: async () => { throw new Error("offline"); },
}), /request failed/);
''')

    def test_npm_checksum_request_timeout_is_an_error(self) -> None:
        self.run_npm_verifier_mock(r'''
await assert.rejects(verifyPackage({ ...options, timeoutMs: 1,
  fetchImpl: async (url, init) => new Promise((resolve, reject) => {
    init.signal.addEventListener("abort", () => reject(new Error("request timed out")));
  }),
}), /request failed.*timed out/);
''')

    def test_npm_publish_checks_integrity_before_and_after_upload(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        publish = workflow.split("  publish-npm:", 1)[1].split("  publish-crate:", 1)[0]
        self.assertIn("actions/checkout@", publish)
        self.assertEqual(publish.count("node scripts/verify_npm_package.mjs"), 2)
        self.assertIn('case "$LOOKUP_STATUS" in', publish)
        self.assertIn("3) ;;", publish)
        self.assertIn('*) exit "$LOOKUP_STATUS"', publish)
        self.assertIn("PUBLISH_STATUS=$?", publish)
        self.assertIn("--attempts 6 --delay-ms 3000 --timeout-ms 5000", publish)

    def test_manual_preflight_builds_all_assets_without_publication(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        jobs = dict(re.findall(r"^  ([a-z-]+):\n(.*?)(?=^  [a-z-]+:\n|\Z)", workflow, re.M | re.S))
        self.assertIn("if: github.event_name != 'workflow_dispatch' || inputs.recovery_run_id == ''", jobs["test"])
        for name in ["build-binaries", "build-wasm-release", "release-assets"]:
            self.assertNotRegex(jobs[name], re.compile(r"^    if:", re.M), name)
        for name in ["publish-npm", "publish-crate", "release"]:
            self.assertIn("if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')", jobs[name])
            self.assertIn("release-assets", jobs[name])
        self.assertNotIn("npm publish", jobs["build-wasm-release"])
        self.assertNotIn("id-token: write", jobs["build-wasm-release"])
        self.assertIn("needs: [test, build-binaries, build-wasm-release]", jobs["release-assets"])
        self.assertIn("pattern: release-build-*", jobs["release-assets"])
        self.assertIn('NPM_PACKAGE=$(realpath "release/allium-deck-wasm-v${VERSION}.tgz")', jobs["publish-npm"])
        self.assertIn('npm publish --provenance --access public "$NPM_PACKAGE"', jobs["publish-npm"])
        self.assertIn("if: github.event_name == 'workflow_dispatch' && inputs.recovery_run_id != ''", jobs["recover-npm"])
        self.assertIn("scripts/validate_release_recovery.py", jobs["recover-npm"])
        self.assertIn("run-id: ${{ inputs.recovery_run_id }}", jobs["recover-npm"])
        self.assertIn('npm publish --provenance --access public "$NPM_PACKAGE"', jobs["recover-npm"])
        self.assertIn("artifacts/SHA256SUMS", jobs["release"])

    def test_asset_names_use_version_instead_of_dispatch_branch(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        wasm_build = workflow.split("  build-wasm-release:", 1)[1].split("  release-assets:", 1)[0]
        self.assertNotIn("GITHUB_REF_NAME", wasm_build)
        self.assertNotIn("github.ref_name", wasm_build)
        self.assertIn('NPM_TGZ="allium-deck-wasm-v${VERSION}.tgz"', wasm_build)

    def test_npm_metadata_is_finalized_before_archiving(self) -> None:
        packager = load_script("package_wasm")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = root / "package.json"
            package.write_text(json.dumps({
                "name": "@empty-sekai/allium-deck-wasm",
                "version": "0.1.0",
                "files": ["allium_deck.js", "allium_deck_bg.wasm"],
            }), encoding="utf-8")
            packager.prepare_npm_metadata(root, "0.1.0", "empty-sekai/allium-deck")
            result = json.loads(package.read_text(encoding="utf-8"))
            self.assertEqual(result["repository"]["url"], "git+https://github.com/empty-sekai/allium-deck.git")
            self.assertEqual(result["publishConfig"]["access"], "public")
            self.assertEqual(result["files"], ["allium_deck.js", "allium_deck_bg.wasm"])
            original = package.read_bytes()
            with self.assertRaisesRegex(ValueError, "version"):
                packager.prepare_npm_metadata(root, "0.1.1", "empty-sekai/allium-deck")
            self.assertEqual(package.read_bytes(), original)

    def test_browser_bundle_and_npm_input_have_identical_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pkg = root / "pkg"
            pkg.mkdir()
            (pkg / "package.json").write_text(json.dumps({
                "name": "@empty-sekai/allium-deck-wasm",
                "version": "0.1.0",
            }), encoding="utf-8")
            (pkg / "allium_deck.js").write_bytes(b"export default {}")
            (pkg / "allium_deck_bg.wasm").write_bytes(b"wasm fixture")
            output = root / "dist"
            subprocess.run([
                sys.executable, str(SCRIPTS / "package_wasm.py"),
                "--pkg-dir", str(pkg), "--out-dir", str(output),
                "--version", "0.1.0", "--source-repository", "empty-sekai/allium-deck",
                "--source-revision", "a" * 40,
            ], check=True, cwd=ROOT, capture_output=True)
            metadata = (pkg / "package.json").read_bytes()
            self.assertEqual((output / "package.json").read_bytes(), metadata)
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            package_entry = next(row for row in manifest["files"] if row["name"] == "package.json")
            self.assertEqual(package_entry["sha256"], hashlib.sha256(metadata).hexdigest())
            self.assertEqual(manifest["source"]["revision"], "a" * 40)

    def test_checksum_manifest_covers_exactly_five_native_and_two_wasm_assets(self) -> None:
        checksum = load_script("create_release_checksums")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            names = checksum.expected_assets("0.1.0")
            self.assertEqual(len(names), 7)
            for name in names:
                (root / name).write_bytes(name.encode())
            output = checksum.create_checksums(root, "0.1.0")
            expected = "".join(f"{hashlib.sha256(name.encode()).hexdigest()}  {name}\n" for name in sorted(names))
            self.assertEqual(output.read_bytes(), expected.encode())
            self.assertEqual(checksum.create_checksums(root, "0.1.0").read_bytes(), expected.encode())
            missing = sorted(names)[0]
            (root / missing).unlink()
            with self.assertRaisesRegex(ValueError, "missing"):
                checksum.create_checksums(root, "0.1.0")
            (root / missing).write_bytes(b"")
            with self.assertRaisesRegex(ValueError, "nonempty"):
                checksum.create_checksums(root, "0.1.0")
            (root / missing).write_bytes(missing.encode())
            (root / "allium-deck-wasm-v0.0.15.tgz").write_bytes(b"old")
            with self.assertRaisesRegex(ValueError, "unexpected"):
                checksum.create_checksums(root, "0.1.0")

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
