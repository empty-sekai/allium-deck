#!/usr/bin/env python3
"""Run interleaved, pinned search matrices and compare complete ordered Top-K.

Test executables must be built with the same compiler and flags. JSONL files are
immutable raw observations; repeated timings are not independent input cases.
"""
import argparse
from collections import defaultdict
import hashlib
import json
import math
from pathlib import Path
import platform
import os
import subprocess
import sys


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def quantiles(values):
    ordered = sorted(values)
    return {
        **{f"p{q}_ms": ordered[max(0, math.ceil(len(ordered) * q / 100) - 1)]
           for q in (50, 95, 99)},
        "max_ms": ordered[-1], "samples": len(ordered),
    }


def signature(row):
    return json.dumps(row["results"], sort_keys=True, separators=(",", ":"))


def summarize(paths, require_complete=True, expected_repeats=None):
    grouped = defaultdict(lambda: defaultdict(list))
    raw = []
    for variant, path in paths:
        count = 0
        samples = defaultdict(list)
        for line in path.read_text().splitlines():
            row = json.loads(line)
            if row["warmup"]:
                continue
            grouped[row["case"]][variant].append(row)
            samples[row["case"]].append(row.get("sample"))
            count += 1
        if expected_repeats is not None:
            expected = list(range(1, expected_repeats + 1))
            for case, numbers in samples.items():
                if sorted(numbers) != expected:
                    raise ValueError(f"invalid sample IDs: {path.name} case={case}")
        raw.append({"variant": variant, "path": str(path), "sha256": digest(path), "samples": count})
    cases = []
    failures = []
    for case, variants in sorted(grouped.items()):
        if set(variants) != {"baseline", "candidate"}:
            failures.append({"case": case, "reason": "unpaired case"})
            continue
        first = variants["candidate"][0]
        report = {key: first[key] for key in ("case", "source", "family", "seed", "scene", "pool_size", "top_k", "timeout_ms", "input_checksum_fnv1a64")}
        identity = dict(report)
        stable = {}
        for variant, rows in variants.items():
            if any(any(row[key] != value for key, value in identity.items()) for row in rows):
                failures.append({"case": case, "variant": variant, "reason": "input identity differs"})
            complete = [row for row in rows if row["completion"] == "complete"]
            sigs = {signature(row) for row in complete}
            stable[variant] = sigs
            report[variant] = {
                "all_runs": quantiles([row["elapsed_ms"] for row in rows]),
                "complete_runs": quantiles([row["elapsed_ms"] for row in complete]) if complete else None,
                "complete": len(complete), "timed_out": len(rows) - len(complete),
                "complete_signatures": len(sigs),
            }
            if len(sigs) > 1:
                failures.append({"case": case, "variant": variant, "reason": "non-deterministic complete result"})
        if len(variants["baseline"]) != len(variants["candidate"]):
            failures.append({"case": case, "reason": "unequal repeat counts"})
        comparable = bool(stable["baseline"] and stable["candidate"])
        report["complete_result_equal"] = comparable and stable["baseline"] == stable["candidate"]
        if comparable and not report["complete_result_equal"]:
            failures.append({"case": case, "reason": "ordered Top-K mismatch"})
        report["all_runs_complete"] = all(row["completion"] == "complete" for rows in variants.values() for row in rows)
        if require_complete and not report["all_runs_complete"]:
            failures.append({"case": case, "reason": "exact comparison requires every run to complete"})
        if report["all_runs_complete"]:
            # These optimizations preserve traversal as well as the feasible set.
            work = [{json.dumps(row["stats"], sort_keys=True) for row in variants[v]} for v in ("baseline", "candidate")]
            report["work_equal"] = work[0] == work[1] and len(work[0]) == 1
            if not report["work_equal"]:
                failures.append({"case": case, "reason": "proof work differs"})
            report["median_ratio"] = report["candidate"]["all_runs"]["p50_ms"] / report["baseline"]["all_runs"]["p50_ms"]
        cases.append(report)
    return {"schema": 1, "gate": "complete_equivalence" if require_complete else "deadline_diagnostics",
            "distinct_cases": len(cases),
            "distinct_generated_pools": len({(x["family"], x["seed"], x["pool_size"]) for x in cases}),
            "paired_complete_cases": sum(x["all_runs_complete"] for x in cases),
            "failures": failures, "cases": cases, "raw": raw,
            "interpretation": "Empirical quantiles of controlled repeats; synthetic inputs are not production traffic. Timed-out outputs do not prove exact Top-K."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=6)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--cpu", default="2")
    parser.add_argument("--sizes", default="26,78,132")
    parser.add_argument("--seeds", default="0,1,2,3")
    parser.add_argument("--topks", default="1,8,30,100")
    parser.add_argument("--timeout-ms", default="0")
    parser.add_argument("--allow-incomplete", action="store_true", help="deadline diagnostic run; never an exact-equivalence pass")
    parser.add_argument("--source-manifest", type=Path, required=True, help="frozen source, compiler and input identities")
    args = parser.parse_args()
    if args.rounds < 1 or args.repeats < 1:
        parser.error("rounds and repeats must be positive")
    for name in ("sizes", "seeds", "topks"):
        values = [int(value) for value in getattr(args, name).split(',')]
        if not values or len(values) != len(set(values)):
            parser.error(f"{name} must contain distinct integers")
    executables = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    binary_hashes = {name: digest(path) for name, path in executables.items()}
    if len(set(binary_hashes.values())) != 2:
        parser.error("baseline and candidate binaries are identical; verify their source and build directories")
    args.output.mkdir(parents=True, exist_ok=False)
    metadata = {"platform": platform.platform(), "python": sys.version,
                "affinity_cpu": args.cpu, "rounds": args.rounds, "repeats_per_round": args.repeats,
                "sizes": args.sizes, "seeds": args.seeds, "topks": args.topks, "timeout_ms": args.timeout_ms,
                "allow_incomplete": args.allow_incomplete,
                "source_manifest": {"path": str(args.source_manifest.resolve()), "sha256": digest(args.source_manifest)},
                "binaries": {name: {"path": str(path), "sha256": binary_hashes[name]} for name, path in executables.items()}}
    (args.output / "environment.json").write_text(json.dumps(metadata, indent=2) + "\n")
    paths = []
    for round_index in range(args.rounds):
        order = ["baseline", "candidate"] if round_index % 2 == 0 else ["candidate", "baseline"]
        for variant in order:
            path = (args.output / f"{variant}-{round_index}.jsonl").resolve()
            env = dict(os.environ, ALLIUM_VALIDATION_OUT=str(path),
                       ALLIUM_VALIDATION_REPEATS=str(args.repeats), ALLIUM_VALIDATION_SIZES=args.sizes,
                       ALLIUM_VALIDATION_SEEDS=args.seeds, ALLIUM_VALIDATION_TOPKS=args.topks,
                       ALLIUM_VALIDATION_TIMEOUT_MS=args.timeout_ms)
            command = ["taskset", "-c", args.cpu, str(executables[variant]), "validation_performance_matrix", "--ignored", "--nocapture", "--test-threads=1"]
            print(f"round={round_index} variant={variant}", flush=True)
            with (args.output / f"{variant}-{round_index}.log").open("w") as log:
                subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            paths.append((variant, path))
    summary = summarize(paths, require_complete=not args.allow_incomplete, expected_repeats=args.repeats)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    expected_cases = 2 * 3 * len(args.sizes.split(',')) * len(args.seeds.split(',')) * len(args.topks.split(','))
    if summary["distinct_cases"] != expected_cases:
        summary["failures"].append({"reason": "matrix coverage count differs", "expected": expected_cases})
        (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    for case in summary["cases"]:
        for variant in executables:
            if case[variant]["all_runs"]["samples"] != args.rounds * args.repeats:
                summary["failures"].append({"case": case["case"], "variant": variant, "reason": "missing measured samples"})
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({key: summary[key] for key in ("distinct_cases", "distinct_generated_pools", "paired_complete_cases", "failures")}))
    return bool(summary["failures"])


if __name__ == "__main__":
    sys.exit(main())
