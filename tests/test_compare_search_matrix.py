"""The report must not turn missing or partial evidence into an exact A/B pass."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("matrix", Path(__file__).parents[1] / "scripts" / "compare_search_matrix.py")
matrix = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(matrix)


class ComparisonTest(unittest.TestCase):
    def rows(self, **changes):
        row = dict(case=1, source="synthetic", family="random", seed=2,
                   scene="world_bloom", pool_size=26, top_k=8, timeout_ms=100,
                   warmup=False, completion="complete", elapsed_ms=1.0, input_checksum_fnv1a64="abc", sample=1,
                   stats={"visited_nodes": 3}, results=[{"score": 10}, {"score": 9}])
        row.update(changes)
        return row

    def report(self, left, right, **options):
        with tempfile.TemporaryDirectory() as directory:
            paths = []
            for variant, rows in (("baseline", left), ("candidate", right)):
                path = Path(directory) / f"{variant}.jsonl"
                path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
                paths.append((variant, path))
            return matrix.summarize(paths, **options)

    def test_lower_rank_mismatch_is_not_hidden_by_equal_top_one(self):
        report = self.report([self.rows()], [self.rows(results=[{"score": 10}, {"score": 8}])])
        self.assertIn("ordered Top-K mismatch", [item["reason"] for item in report["failures"]])

    def test_timeout_does_not_certify_equal_incumbents(self):
        report = self.report([self.rows()], [self.rows(completion="timed_out")])
        self.assertEqual(report["paired_complete_cases"], 0)
        self.assertFalse(report["cases"][0]["complete_result_equal"])
        self.assertIsNone(report["cases"][0]["candidate"]["complete_runs"])
        self.assertTrue(report["failures"])

    def test_warmups_are_excluded_and_missing_pairs_fail(self):
        report = self.report([self.rows(), self.rows(warmup=True, elapsed_ms=999)], [self.rows()])
        self.assertEqual(report["cases"][0]["baseline"]["all_runs"]["max_ms"], 1.0)
        missing = self.report([self.rows()], [])
        self.assertEqual(missing["failures"][0]["reason"], "unpaired case")

    def test_equal_but_incomplete_or_duplicated_samples_are_rejected(self):
        for rows in ([self.rows()], [self.rows(), self.rows()]):
            with self.assertRaises(ValueError):
                self.report(rows, rows, expected_repeats=2)

    def test_deadline_diagnostics_are_explicitly_not_an_exact_gate(self):
        report = self.report([self.rows(completion="timed_out")],
                             [self.rows(completion="timed_out")], require_complete=False)
        self.assertEqual(report["gate"], "deadline_diagnostics")
        self.assertEqual(report["paired_complete_cases"], 0)


if __name__ == "__main__":
    unittest.main()
