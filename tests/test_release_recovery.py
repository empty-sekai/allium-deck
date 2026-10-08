from __future__ import annotations

import copy
import hashlib
import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("recovery", ROOT / "scripts/validate_release_recovery.py")
recovery = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(recovery)


class ReleaseRecoveryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.sha = "a" * 40
        self.run = {
            "id": 100, "event": "push", "path": ".github/workflows/release.yml",
            "head_branch": "v0.1.0", "head_sha": self.sha,
            "status": "completed", "conclusion": "failure",
            "repository": {"id": 200, "full_name": recovery.REPOSITORY},
        }
        self.artifact = {
            "id": 300, "name": "release-assets", "expired": False,
            "digest": "sha256:" + "b" * 64,
            "workflow_run": {"id": 100, "head_sha": self.sha, "head_branch": "v0.1.0",
                             "repository_id": 200, "head_repository_id": 200},
        }
        self.artifacts = {"artifacts": [self.artifact]}
        package = {"name": recovery.PACKAGE, "version": "0.1.0",
                   "repository": {"type": "git", "url": f"git+https://github.com/{recovery.REPOSITORY}.git"}}
        self.browser = {"package.json": json.dumps(package).encode(),
                        "allium_deck.js": b"export default function initialize() {}",
                        "allium_deck_bg.wasm": b"\0asm-test-bytes"}
        self.manifest = {"name": "allium-deck-wasm", "version": "0.1.0",
                         "source": {"repository": recovery.REPOSITORY, "revision": self.sha},
                         "entrypoint": "allium_deck.js", "wasm": "allium_deck_bg.wasm",
                         "files": [{"name": name, "bytes": len(content),
                                    "sha256": hashlib.sha256(content).hexdigest()}
                                   for name, content in self.browser.items()]}
        for name in recovery.release_names("0.1.0"):
            if name.startswith("recommend_cli-"):
                (self.directory / name).write_bytes(b"native executable fixture")
        self.write_zip()
        self.write_tar(self.browser)
        self.write_sums()

    def write_zip(self) -> None:
        with zipfile.ZipFile(self.directory / "allium-deck-wasm-v0.1.0-cn.zip", "w") as archive:
            for name, content in self.browser.items():
                archive.writestr(name, content)
            archive.writestr("manifest.json", json.dumps(self.manifest))

    def write_tar(self, files: dict[str, bytes]) -> None:
        with tarfile.open(self.directory / "allium-deck-wasm-v0.1.0.tgz", "w:gz") as archive:
            for name, content in files.items():
                member = tarfile.TarInfo("package/" + name)
                member.size = len(content)
                archive.addfile(member, io.BytesIO(content))

    def write_sums(self) -> None:
        rows = [f"{recovery.file_hash(self.directory / name)}  {name}\n"
                for name in sorted(recovery.release_names("0.1.0"))]
        (self.directory / "SHA256SUMS").write_text("".join(rows), encoding="utf-8")

    def validate(self) -> dict:
        return recovery.validate_recovery(tag="v0.1.0", source_sha=self.sha,
            run=self.run, artifacts=self.artifacts, asset_dir=self.directory)

    def test_valid_failed_or_successful_tag_build_is_read_only(self) -> None:
        before = {path.name: path.read_bytes() for path in self.directory.iterdir()}
        result = self.validate()
        self.assertEqual(result["artifactId"], 300)
        self.assertEqual(result["revision"], self.sha)
        self.assertEqual(result["wasmSha256"], hashlib.sha256(self.browser["allium_deck_bg.wasm"]).hexdigest())
        self.assertTrue(result["tgzSRI"].startswith("sha512-"))
        self.run["conclusion"] = "success"
        self.assertEqual(self.validate(), result)
        self.assertEqual(before, {path.name: path.read_bytes() for path in self.directory.iterdir()})

    def test_unrelated_or_incomplete_workflow_runs_are_rejected(self) -> None:
        original = copy.deepcopy(self.run)
        changes = [
            ("head_sha", "c" * 40), ("head_branch", "v0.0.15"), ("event", "workflow_dispatch"),
            ("path", ".github/workflows/ci.yml"), ("status", "in_progress"),
            ("conclusion", "cancelled"), ("repository", {"id": 200, "full_name": "other/repo"}),
        ]
        for key, value in changes:
            with self.subTest(key=key):
                self.run = {**original, key: value}
                with self.assertRaises(ValueError):
                    self.validate()

    def test_artifact_must_be_unique_unexpired_and_bound_to_the_original_run(self) -> None:
        original = copy.deepcopy(self.artifact)
        variants = [[], [original, copy.deepcopy(original)], [{**original, "expired": True}],
                    [{**original, "digest": None}], [{**original, "workflow_run": None}]]
        for field, value in [("head_sha", "d" * 40), ("id", 999), ("head_branch", "main"),
                             ("repository_id", 999), ("head_repository_id", 999)]:
            modified = copy.deepcopy(original)
            modified["workflow_run"][field] = value
            variants.append([modified])
        for entries in variants:
            with self.subTest(entries=entries):
                self.artifacts = {"artifacts": entries}
                with self.assertRaises(ValueError):
                    self.validate()

    def test_all_seven_checksums_are_required_and_never_rewritten(self) -> None:
        sums = self.directory / "SHA256SUMS"
        original = sums.read_bytes()
        extra = self.directory / "unpacked"
        extra.mkdir()
        with self.assertRaisesRegex(ValueError, "filenames"):
            self.validate()
        extra.rmdir()
        for content in [b"".join(original.splitlines(keepends=True)[1:]), original + original.splitlines(keepends=True)[0]]:
            sums.write_bytes(content)
            with self.assertRaises(ValueError):
                self.validate()
            self.assertEqual(sums.read_bytes(), content)
        sums.write_bytes(original)
        native = self.directory / "recommend_cli-v0.1.0-linux-x86_64"
        native.write_bytes(b"changed native executable")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.validate()
        self.assertEqual(sums.read_bytes(), original)

    def test_archive_source_and_internal_hashes_are_checked_after_outer_checksums(self) -> None:
        original = copy.deepcopy(self.manifest)
        variants = []
        for key, value in [("source", {"repository": recovery.REPOSITORY, "revision": "e" * 40}),
                           ("version", "0.0.15")]:
            variants.append({**original, key: value})
        bad_hash = copy.deepcopy(original)
        bad_hash["files"][0]["sha256"] = "0" * 64
        variants.append(bad_hash)
        for manifest in variants:
            with self.subTest(manifest=manifest):
                self.manifest = manifest
                self.write_zip()
                self.write_sums()
                with self.assertRaises(ValueError):
                    self.validate()

    def test_tgz_bytes_must_equal_zip_and_metadata_identity_must_match(self) -> None:
        for name in ["allium_deck_bg.wasm", "allium_deck.js", "package.json"]:
            with self.subTest(name=name):
                modified = {**self.browser, name: b"different"}
                self.write_tar(modified)
                self.write_sums()
                with self.assertRaisesRegex(ValueError, "ZIP/npm content mismatch"):
                    self.validate()
        wrong_package = json.loads(self.browser["package.json"])
        wrong_package["name"] = "wrong-package"
        self.browser["package.json"] = json.dumps(wrong_package).encode()
        row = next(row for row in self.manifest["files"] if row["name"] == "package.json")
        row.update(bytes=len(self.browser["package.json"]), sha256=hashlib.sha256(self.browser["package.json"]).hexdigest())
        self.write_zip()
        self.write_tar(self.browser)
        self.write_sums()
        with self.assertRaisesRegex(ValueError, "npm package identity mismatch"):
            self.validate()


if __name__ == "__main__":
    unittest.main()
