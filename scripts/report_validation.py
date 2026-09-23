#!/usr/bin/env python3
"""Summarize complete-result and latency coverage from validation JSONL."""

import json
import math
import sys
from collections import defaultdict
from pathlib import Path


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    values.sort()
    index = math.ceil(fraction * len(values)) - 1
    return round(values[max(index, 0)], 3)


def summarize(samples: list[dict]) -> dict:
    measured = [row for row in samples if row["phase"] == "measured"]
    complete = [row for row in measured if row["completion"] == "complete"]
    times = [row["wall_ms"] for row in complete]
    return {
        "samples": len(measured),
        "complete": len(complete),
        "timed_out": sum(row["completion"] == "timed_out" for row in measured),
        "other_incomplete": sum(
            row["completion"] not in ("complete", "timed_out") for row in measured
        ),
        "under_20_ms": sum(t < 20 for t in times),
        "20_to_200_ms": sum(20 <= t <= 200 for t in times),
        "over_200_ms": sum(t > 200 for t in times),
        "p50_complete_ms": percentile(times[:], 0.50),
        "p95_complete_ms": percentile(times[:], 0.95),
        "max_complete_ms": round(max(times), 3) if times else None,
        "max_pool": max((row["pool"] for row in measured), default=None),
    }


def report(path: Path) -> dict:
    records = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    by_type = defaultdict(list)
    for row in records:
        by_type[row["record"]].append(row)
    samples = by_type["sample"]
    if not samples or len(by_type["inventory"]) != 1 or len(by_type["summary"]) != 1:
        raise ValueError("validation artifact lacks inventory, samples, or summary")

    # The harness compares completed rows across rounds. Check this again from
    # the saved artifact so an interrupted output cannot be mistaken for proof.
    result_rows = defaultdict(list)
    for row in samples:
        if row["completion"] == "complete":
            result_rows[(row["case"], row["top_k"])].append(row["results"])
    mismatches = [
        {"case": case, "top_k": top_k}
        for (case, top_k), results in result_rows.items()
        if any(result != results[0] for result in results[1:])
    ]
    groups = defaultdict(list)
    for row in samples:
        key = (
            row["source"],
            row["mode"],
            row["state_mode"],
            row["live_type"],
            row["top_k"],
        )
        groups[key].append(row)
    return {
        "inventory": by_type["inventory"][0],
        "summary": by_type["summary"][0],
        "gate": by_type["gate"][0] if by_type["gate"] else None,
        "measured": summarize(samples),
        "warmup": summarize(
            [{**row, "phase": "measured"} for row in samples if row["phase"] == "warmup"]
        ),
        "complete_result_mismatches": mismatches,
        "groups": [
            {
                "source": key[0],
                "mode": key[1],
                "state_mode": key[2],
                "live_type": key[3],
                "top_k": key[4],
                **summarize(rows),
            }
            for key, rows in sorted(groups.items())
        ],
    }


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: report_validation.py <validation.jsonl>")
    print(json.dumps(report(Path(sys.argv[1])), ensure_ascii=False, indent=2))
