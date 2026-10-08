#!/usr/bin/env python3
"""Build every proof, audit transitive axioms, and check source/coverage drift.

No dependencies other than Python 3.11+ and the pinned Lean/Lake toolchain.
A successful run checks the declared scope; it does not change open obligations
into proofs. Use --require-complete when evaluating stage-one completion.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
NAME = re.compile(r"Allium(?:\.[A-Za-z_][A-Za-z_0-9']*)+")
STATUSES = {"proved", "partial", "open", "out_of_scope"}


def digest(path: Path) -> str:
    """Ignore checkout CRLF conversion, not any other source change."""
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def run(args: list[str], *, expect_failure: bool = False) -> str:
    result = subprocess.run(args, cwd=HERE, text=True, encoding="utf-8",
                            errors="replace", stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=1800, check=False)
    if expect_failure:
        if result.returncode == 0 or "AXIOM AUDIT FAILED:" not in result.stdout:
            raise RuntimeError(f"Negative audit fixture did not fail as intended:\n{result.stdout}")
    elif result.returncode != 0:
        raise RuntimeError(f"Command failed ({result.returncode}): {args}\n{result.stdout}")
    return result.stdout


def check_inventory(obligations: list[dict]) -> None:
    """The complete manual inventory must remain visible in the proof scope."""
    document = (ROOT / "docs/pruning-proof.md").read_text(encoding="utf-8")
    section = document.split("## 26. Proof-to-code inventory", 1)[1].split("## 27.", 1)[0]
    rows = [line.strip("|").split("|")[0].strip()
            for line in section.splitlines() if line.startswith("| ")][2:]
    if not rows:
        raise ValueError("No pruning mechanisms found in the source inventory")
    expected = {f"P{i:02}": description for i, description in enumerate(rows, 1)}
    actual = {o["id"]: o["description"] for o in obligations if o["id"].startswith("P")}
    if actual != expected:
        raise ValueError("Pruning inventory mismatch: do not remove, rename or omit a mechanism")
    required = set(expected) | {"S01", "S02", "S03", "S04", "S05", "R01"}
    if {o["id"] for o in obligations} != required:
        raise ValueError("Core/search/scene/refinement coverage inventory mismatch")
    for obligation in obligations:
        if obligation["status"] == "out_of_scope" and obligation["id"] != "R01":
            raise ValueError(f"First-tier mathematical obligation cannot be excluded: {obligation['id']}")


def inventory_self_test(obligations: list[dict]) -> None:
    """Reject omitting hard work or relabelling it as Rust refinement."""
    missing = [dict(o) for o in obligations if o["id"] != "P01"]
    excluded = [dict(o, status="out_of_scope") if o["id"] == "S05" else dict(o)
                for o in obligations]
    for label, fixture in [("omitted-pruning-mechanism", missing),
                           ("excluded-mathematical-obligation", excluded)]:
        try:
            check_inventory(fixture)
        except ValueError:
            print(f"NEGATIVE COVERAGE TEST PASSED: {label}")
        else:
            raise RuntimeError(f"Coverage fixture was incorrectly accepted: {label}")


def metadata() -> tuple[dict, list[str]]:
    manifest = json.loads((HERE / "coverage.json").read_text(encoding="utf-8"))
    if manifest.get("schema") != 1 or not manifest.get("sources") or not manifest.get("obligations"):
        raise ValueError("Missing or unsupported source/coverage manifest")
    for relative, expected in manifest["sources"].items():
        path = (ROOT / relative).resolve()
        if not path.is_relative_to(ROOT) or not path.is_file():
            raise ValueError(f"Invalid source path: {relative}")
        actual = digest(path)
        if actual != expected:
            raise ValueError(f"Source drift: {relative}\nexpected {expected}\nactual   {actual}\n"
                             "Reconcile the model and proofs before updating the snapshot.")
    modules = {".".join(p.relative_to(HERE).with_suffix("").parts)
               for p in (HERE / "Allium").rglob("*.lean")}
    imported = set(re.findall(r"^import\s+(Allium(?:\.[A-Za-z_0-9]+)+)\s*$",
                              (HERE / "Allium.lean").read_text(encoding="utf-8"), re.M))
    if modules != imported:
        raise ValueError(f"Umbrella import mismatch: missing={modules-imported}; stale={imported-modules}")
    if not modules:
        raise ValueError("No proof modules found")
    ids: set[str] = set()
    theorem_names: set[str] = set()
    for obligation in manifest["obligations"]:
        oid = obligation["id"]
        if oid in ids or obligation["status"] not in STATUSES:
            raise ValueError(f"Invalid/duplicate obligation: {oid}")
        ids.add(oid)
        if not obligation.get("description") or not obligation.get("sources"):
            raise ValueError(f"Missing scope/source information: {oid}")
        if not set(obligation["sources"]).issubset(manifest["sources"]):
            raise ValueError(f"Unpinned source in obligation {oid}")
        if obligation["status"] == "proved" and (obligation.get("remaining") or not obligation.get("theorems")):
            raise ValueError(f"A proved obligation needs theorem references and no remaining work: {oid}")
        if obligation["status"] in {"partial", "open"} and not obligation.get("remaining"):
            raise ValueError(f"An incomplete obligation must disclose remaining work: {oid}")
        for theorem in obligation.get("theorems", []):
            if not NAME.fullmatch(theorem):
                raise ValueError(f"Invalid theorem name: {theorem}")
            theorem_names.add(theorem)
    check_inventory(manifest['obligations'])
    complete = all(o["status"] in {"proved", "out_of_scope"} for o in manifest["obligations"])
    if manifest.get("stage_one_complete") != complete:
        raise ValueError("stage_one_complete contradicts the obligation table")
    print(f"SOURCE CHECK PASSED: {len(manifest['sources'])} files; {len(modules)} imported proof modules")
    for status in sorted(STATUSES):
        count = sum(o["status"] == status for o in manifest["obligations"])
        print(f"COVERAGE {status}: {count}")
    print(f"STAGE ONE COMPLETE: {complete}")
    return manifest, sorted(theorem_names)


def audit(theorems: list[str], self_test: bool) -> None:
    script = (HERE / "Audit.lean").read_text(encoding="utf-8")
    imports, body = script.split("/-!", 1)
    # Preserve the checker verbatim; inject fixtures before its run_cmd block.
    body = "/-!" + body
    with tempfile.TemporaryDirectory(prefix="allium-lean-audit-") as tmp:
        root = Path(tmp)
        checked = root / "Check.lean"
        checked.write_text(script + "\n" + "\n".join(f"#check {name}" for name in theorems) + "\n",
                           encoding="utf-8")
        output = run(["lake", "env", "lean", "-DwarningAsError=true", str(checked)])
        for line in output.splitlines():
            if "AXIOM AUDIT PASSED:" in line or "depends on axioms:" in line:
                print(line)
        if "AXIOM AUDIT PASSED:" not in output:
            raise RuntimeError("Audit completed without its success marker")
        if self_test:
            fixtures = {
                "sorry": "theorem Allium.negativeSorry : False := by sorry\n",
                "foreign-axiom": "axiom ForeignUntrusted : False\n"
                                 "theorem Allium.negativeForeign : False := ForeignUntrusted\n",
                "unused-local-axiom": "axiom Allium.negativeUnused : False\n",
                "native-decide": "theorem Allium.negativeNative : (2 + 2 : Nat) = 4 := by native_decide\n",
            }
            for label, fixture in fixtures.items():
                path = root / f"Reject-{label}.lean"
                path.write_text(imports + "\n" + fixture + "\n" + body, encoding="utf-8")
                run(["lake", "env", "lean", "-DwarningAsError=true", str(path)], expect_failure=True)
                print(f"NEGATIVE AUDIT TEST PASSED: {label}")


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, 'reconfigure'):
            stream.reconfigure(encoding='utf-8', errors='backslashreplace')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="verify rejection of four axiom escape routes")
    parser.add_argument("--require-complete", action="store_true", help="fail while any stage-one obligation is incomplete")
    args = parser.parse_args()
    manifest, theorems = metadata()
    print(run(["lake", "build"]).strip())
    audit(theorems, args.self_test)
    if args.self_test:
        inventory_self_test(manifest['obligations'])
    if args.require_complete and not manifest["stage_one_complete"]:
        pending = [o["id"] for o in manifest["obligations"] if o["status"] in {"partial", "open"}]
        raise RuntimeError("Stage-one verification is incomplete: " + ", ".join(pending))
    print("VERIFICATION PASSED FOR THE DECLARED SCOPE (see coverage.json)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"VERIFICATION FAILED: {exc}", file=sys.stderr)
        sys.exit(1)
