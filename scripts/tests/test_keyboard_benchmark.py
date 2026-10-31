"""Verify the offline synthetic-task harness without collecting keyboard input."""

import csv
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "keyboard-benchmark.py"
CORPUS = SCRIPT.parents[1] / "docs/evidence/keyboard-benchmark-corpus.json"
SPEC = importlib.util.spec_from_file_location("keyboard_benchmark", SCRIPT)
BENCHMARK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCHMARK)
FIELDS = ["trial_id", "participant", "keyboard", "device", "os", "task_id",
          "elapsed_seconds", "corrected_errors", "harmful_corrections"]


class KeyboardBenchmarkTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "trials.csv"
        self.tasks = BENCHMARK.load_corpus(CORPUS)

    def row(self, trial="trial-1", **changes):
        return {"trial_id": trial, "participant": "person-1", "keyboard": "pennel",
                "device": "iPhone 16 Pro", "os": "iOS 27.0.1", "task_id": "nl-001",
                "elapsed_seconds": "8", "corrected_errors": "1", "harmful_corrections": "0", **changes}

    def write(self, rows, with_text=False, with_ratings=False):
        fields = FIELDS + (["final_text"] if with_text else []) + (["immediacy", "comfort", "annoyance", "fatigue"] if with_ratings else [])
        with self.path.open("w", newline="", encoding="utf-8") as output:
            writer = csv.DictWriter(output, fieldnames=fields)
            writer.writeheader()
            writer.writerows(rows)
        return BENCHMARK.read_trials(self.path, self.tasks)

    def test_corpus_has_ninety_authored_tasks_and_aligned_language_themes(self):
        self.assertEqual(len(self.tasks), 90)
        themes = []
        for language in ("nl", "en", "mixed"):
            tasks = [task for task in self.tasks.values() if task["language"] == language]
            self.assertEqual(len(tasks), 30)
            themes.append({task["theme"] for task in tasks})
            self.assertTrue(any(task["mode"] == "dictation" and task["source_text"] != task["reference_text"] for task in tasks))
        self.assertEqual(themes[0], themes[1])
        self.assertEqual(themes[1], themes[2])

    def test_corpus_rejects_unmarked_and_duplicate_tasks(self):
        data = json.loads(CORPUS.read_text())
        data["synthetic"] = False
        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.validate_corpus(data)
        data["synthetic"] = True
        data["tasks"][1] = data["tasks"][0]
        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.validate_corpus(data)

    def test_invalid_numbers_unknown_tasks_and_duplicate_trial_ids_are_rejected(self):
        invalid = [("elapsed_seconds", value) for value in ("nan", "inf", "-1", "0")]
        invalid += [(field, value) for field in ("corrected_errors", "harmful_corrections") for value in ("-1", "nan", "1.5")]
        invalid += [("task_id", "unknown-task"), ("participant", "person@example.com")]
        for field, value in invalid:
            with self.subTest(field=field, value=value), self.assertRaises(BENCHMARK.BenchmarkError):
                self.write([self.row(**{field: value})])
        with self.assertRaises(BENCHMARK.BenchmarkError):
            self.write([self.row(), self.row()])

    def test_malformed_csv_headers_and_missing_cells_are_rejected(self):
        self.path.write_text("trial_id,trial_id\na,b\n")
        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.read_trials(self.path, self.tasks)
        self.path.write_text(",".join(FIELDS) + "\ntrial-1,person-1\n")
        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.read_trials(self.path, self.tasks)

    def test_aggregate_uses_median_and_reference_exposure_without_text_leakage(self):
        marker = "PRIVATE_MARKER_DO_NOT_REPORT"
        rows = self.write([self.row("a", elapsed_seconds="4", harmful_corrections="1", final_text=marker),
                           self.row("b", elapsed_seconds="10", harmful_corrections="2", final_text=""),
                           self.row("c", elapsed_seconds="100", harmful_corrections="0", final_text="")], with_text=True)
        report = BENCHMARK.summarize(self.tasks, rows)
        group = report["groups"][0]
        self.assertEqual(group["median_elapsed_seconds"], 10)
        self.assertEqual(group["harmful_corrections"], 3)
        words = len(BENCHMARK.word_tokens(self.tasks["nl-001"]["reference_text"]))
        self.assertAlmostEqual(group["harmful_corrections_per_1000_reference_words"], 1000 / words, places=6)
        encoded = json.dumps(report)
        for forbidden in (marker, "person-1", self.tasks["nl-001"]["reference_text"], "final_text"):
            self.assertNotIn(forbidden, encoded)
        self.assertNotIn("wpm", encoded.lower())
        self.assertIsNone(report["paired_comparisons"])

    def test_optional_text_scores_exact_and_explicit_empty_outputs(self):
        reference = self.tasks["nl-001"]["reference_text"]
        rows = self.write([self.row(final_text=reference)], with_text=True)
        group = BENCHMARK.summarize(self.tasks, rows)["groups"][0]
        self.assertEqual((group["wer"], group["cer"]), (0, 0))
        rows = self.write([self.row(final_text="")], with_text=True)
        group = BENCHMARK.summarize(self.tasks, rows)["groups"][0]
        self.assertEqual((group["wer"], group["cer"]), (1, 1))
        rows = self.write([self.row()])
        group = BENCHMARK.summarize(self.tasks, rows)["groups"][0]
        self.assertIsNone(group["wer"])
        self.assertEqual(group["text_scored_trials"], 0)

    def test_pairing_requires_same_participant_device_os_language_and_task_set(self):
        rows = self.write([self.row("apple", keyboard="apple", elapsed_seconds="10"), self.row("pennel")])
        paired = BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple")["paired_comparisons"][0]
        self.assertEqual(paired["matched_task_pairs"], 1)
        self.assertAlmostEqual(paired["median_elapsed_change_percent"], -20)
        for field, value in (("participant", "person-2"), ("device", "iPhone 15"), ("os", "iOS 26"), ("task_id", "en-001")):
            with self.subTest(field=field):
                rows = self.write([self.row("apple", keyboard="apple"), self.row("pennel", **{field: value})])
                comparisons = BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple")["paired_comparisons"]
                self.assertTrue(all(item["matched_task_pairs"] == 0 for item in comparisons))
                self.assertTrue(all(item["median_elapsed_change_percent"] is None for item in comparisons))

    def test_partial_task_overlap_reports_unpaired_instead_of_extrapolating(self):
        rows = self.write([self.row("a", keyboard="apple"), self.row("b"), self.row("c", task_id="nl-002")])
        paired = BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple")["paired_comparisons"][0]
        self.assertEqual(paired["matched_task_pairs"], 0)
        self.assertEqual(paired["unpaired_reasons"]["different_task_sets"], 1)

    def test_repeated_trials_do_not_receive_extra_weight_in_paired_comparison(self):
        rows = self.write([self.row("a", keyboard="apple", elapsed_seconds="10"),
                           self.row("b", elapsed_seconds="4"), self.row("c", elapsed_seconds="12")])
        paired = BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple")["paired_comparisons"][0]
        self.assertEqual(paired["matched_task_pairs"], 1)
        self.assertAlmostEqual(paired["median_elapsed_change_percent"], -20)

    def test_extreme_finite_observations_cannot_produce_nonfinite_report_values(self):
        rows = self.write([self.row("a", keyboard="apple", elapsed_seconds="1e-308"),
                           self.row("b", elapsed_seconds="1e308")])
        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple")

    def test_mode_slices_expose_dictation_regressions_without_mixing_comparisons(self):
        rows = self.write([self.row("at", keyboard="apple", elapsed_seconds="10"), self.row("pt"),
                           self.row("ad", keyboard="apple", task_id="nl-023", elapsed_seconds="10"),
                           self.row("pd", task_id="nl-023", elapsed_seconds="20")])
        report = BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple")
        self.assertEqual(report["mode_filter"], "all")
        self.assertTrue(report["contains_mixed_modes"])
        self.assertTrue(all(group["mode"] == "mixed" for group in report["groups"]))
        changes = {item["mode"]: item["median_elapsed_change_percent"] for item in report["paired_comparisons"]}
        self.assertEqual(changes, {"typing": -20, "dictation": 100})
        self.assertEqual(report["by_mode"]["dictation"]["included_trials"], 2)
        filtered = BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple", mode="typing")
        self.assertEqual((filtered["supplied_trials"], filtered["included_trials"], filtered["excluded_trials"]), (4, 2, 2))
        self.assertEqual(set(filtered["by_mode"]), {"typing"})
        self.assertTrue(all(item["mode"] == "typing" for item in filtered["paired_comparisons"]))

    def test_different_modes_cannot_be_paired_and_empty_filter_is_explicit(self):
        rows = self.write([self.row("a", keyboard="apple"), self.row("b", task_id="nl-023")])
        comparisons = BENCHMARK.summarize(self.tasks, rows, apple_keyboard="apple")["paired_comparisons"]
        self.assertEqual(len(comparisons), 2)
        self.assertTrue(all(item["matched_task_pairs"] == 0 for item in comparisons))
        self.assertTrue(all(item["median_elapsed_change_percent"] is None for item in comparisons))
        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.summarize(self.tasks, rows, mode="unknown")
        rows = self.write([self.row()])
        with self.assertRaises(BENCHMARK.BenchmarkError):
            BENCHMARK.summarize(self.tasks, rows, mode="dictation")

    def test_missing_subjective_ratings_are_not_imputed(self):
        rows = self.write([self.row("a", immediacy="6", comfort="5", annoyance="2", fatigue=""),
                           self.row("b", immediacy="", comfort="7", annoyance="", fatigue="3")], with_ratings=True)
        report = BENCHMARK.summarize(self.tasks, rows)
        ratings = report["groups"][0]["subjective_ratings"]
        self.assertEqual(ratings["immediacy"], {"count": 1, "median": 6})
        self.assertEqual(ratings["comfort"], {"count": 2, "median": 6})
        self.assertEqual(ratings["annoyance"], {"count": 1, "median": 2})
        self.assertEqual(ratings["fatigue"], {"count": 1, "median": 3})
        self.assertNotIn("person-1", json.dumps(report))
        rows = self.write([self.row()])
        missing = BENCHMARK.summarize(self.tasks, rows)["groups"][0]["subjective_ratings"]
        self.assertTrue(all(value == {"count": 0, "median": None} for value in missing.values()))

    def test_subjective_ratings_must_be_finite_and_in_range(self):
        for field in ("immediacy", "comfort", "annoyance", "fatigue"):
            for value in ("nan", "inf", "-1", "0", "7.1", "8", "not-a-rating"):
                with self.subTest(field=field, value=value), self.assertRaises(BENCHMARK.BenchmarkError):
                    self.write([self.row(**{field: value})], with_ratings=True)

    def test_cli_supports_mode_filter_and_optional_rating_template(self):
        result = subprocess.run(["python3", str(SCRIPT), "template", "--with-ratings"], capture_output=True, text=True, check=True)
        self.assertEqual(next(csv.reader(io.StringIO(result.stdout))), FIELDS + ["immediacy", "comfort", "annoyance", "fatigue"])
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.write([self.row("t"), self.row("d", task_id="nl-023")])
        result = subprocess.run(["python3", str(SCRIPT), "summarize", str(self.path), "--mode", "dictation"], capture_output=True, text=True, check=True)
        report = json.loads(result.stdout)
        self.assertEqual(report["included_trials"], 1)
        self.assertTrue(all(group["mode"] == "dictation" for group in report["groups"]))

    def test_cli_template_contains_only_header_and_errors_do_not_echo_input(self):
        result = subprocess.run(["python3", str(SCRIPT), "template"], capture_output=True, text=True, check=True)
        self.assertEqual(next(csv.reader(io.StringIO(result.stdout))), FIELDS)
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.path.write_text(",".join(FIELDS) + "\nPRIVATE_INPUT_MUST_NOT_BE_ECHOED\n")
        result = subprocess.run(["python3", str(SCRIPT), "summarize", str(self.path)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("PRIVATE_INPUT_MUST_NOT_BE_ECHOED", result.stderr + result.stdout)


if __name__ == "__main__":
    unittest.main()
