import importlib.util
from pathlib import Path
import sys
import unittest


SPEC = importlib.util.spec_from_file_location(
    "speaker_quality_evaluate", Path(__file__).resolve().parents[1] / "speaker-quality-evaluate.py"
)
EVALUATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EVALUATOR)


def turn(start, end, speaker):
    return {"start_ms": start, "end_ms": end, "speaker": speaker}


def reference(turns, regions=None):
    return {"schema_version": 1,
            "annotated_regions": regions or [{"start_ms": 0, "end_ms": 4_000}],
            "turns": turns}


class SpeakerQualityTests(unittest.TestCase):
    def test_assignment_is_global_instead_of_greedy(self):
        self.assertEqual(EVALUATOR.best_mapping([[9, 8], [8, 0]]), {0: 1, 1: 0})

    def test_speaker_ids_are_permuted_before_scoring(self):
        result = EVALUATOR.score(reference([turn(0, 2_000, "a"), turn(2_000, 4_000, "b")]),
                                 [turn(0, 2_000, 9), turn(2_000, 4_000, 2)])
        self.assertEqual(result["diarization_error_rate"], 0)
        self.assertEqual(result["non_overlap_correct_coverage"], 1)

    def test_fragments_cannot_both_map_to_one_person(self):
        result = EVALUATOR.score(reference([turn(0, 4_000, "a")]),
                                 [turn(0, 2_000, 0), turn(2_000, 4_000, 1)])
        self.assertEqual(result["confusion_speaker_ms"], 2_000)
        self.assertEqual(result["non_overlap_confusion_rate"], 0.5)

    def test_missing_speech_is_not_misrepresented_as_zero_error(self):
        result = EVALUATOR.score(reference([turn(0, 4_000, "a")]), [])
        self.assertEqual(result["missed_speaker_ms"], 4_000)
        self.assertEqual(result["diarization_error_rate"], 1)
        self.assertEqual(result["non_overlap_error_rate"], 1)
        self.assertEqual(result["non_overlap_correct_coverage"], 0)

    def test_overlap_is_scored_in_speaker_time(self):
        result = EVALUATOR.score(reference([turn(0, 4_000, "a"), turn(0, 4_000, "b")]),
                                 [turn(0, 4_000, 0)])
        self.assertEqual(result["reference_speaker_ms"], 8_000)
        self.assertEqual(result["diarization_error_rate"], 0.5)
        self.assertIsNone(result["non_overlap_confusion_rate"])

    def test_extra_speaker_is_false_alarm(self):
        result = EVALUATOR.score(reference([turn(0, 4_000, "a")]),
                                 [turn(0, 4_000, 0), turn(0, 4_000, 1)])
        self.assertEqual(result["false_alarm_speaker_ms"], 4_000)
        self.assertEqual(result["diarization_error_rate"], 1)

    def test_unannotated_time_is_not_treated_as_silence(self):
        result = EVALUATOR.score(reference([turn(1_000, 2_000, "a")],
                                           [{"start_ms": 1_000, "end_ms": 2_000}]),
                                 [turn(1_000, 2_000, 0), turn(3_000, 4_000, 1)])
        self.assertEqual(result["diarization_error_rate"], 0)
        self.assertEqual(result["annotated_wall_ms"], 1_000)
        self.assertEqual(result["hypothesis_speaker_count"], 1)

    def test_duplicate_same_speaker_intervals_are_not_counted_twice(self):
        result = EVALUATOR.score(reference([turn(0, 4_000, "a")]),
                                 [turn(0, 4_000, 0), turn(1_000, 2_000, 0)])
        self.assertEqual(result["diarization_error_rate"], 0)

    def test_reference_needs_explicit_regions(self):
        with self.assertRaises(ValueError):
            EVALUATOR.load_reference({"schema_version": 1, "turns": [turn(0, 1_000, "a")]})

    def test_reference_regions_cannot_overlap_or_exclude_turns(self):
        with self.assertRaises(ValueError):
            EVALUATOR.load_reference(reference([], [{"start_ms": 0, "end_ms": 2_000},
                                                     {"start_ms": 1_000, "end_ms": 3_000}]))
        with self.assertRaises(ValueError):
            EVALUATOR.load_reference(reference([turn(3_000, 5_000, "a")]))

    def test_zero_speech_has_no_invented_accuracy(self):
        result = EVALUATOR.score(reference([]), [turn(0, 1_000, 0)])
        self.assertIsNone(result["diarization_error_rate"])
        self.assertEqual(result["false_alarm_speaker_ms"], 1_000)

    def test_peak_rss_units_are_explicit(self):
        self.assertEqual(EVALUATOR.peak_rss("  1234  maximum resident set size\n"), 1234)
        self.assertEqual(EVALUATOR.peak_rss("Maximum resident set size (kbytes): 1234\n"), 1234 * 1024)
        self.assertIsNone(EVALUATOR.peak_rss("no measurement"))

    def test_expired_deadline_does_not_start_a_process(self):
        result, timed_out = EVALUATOR.run_owned(["/does/not/exist"], Path.cwd(), 0)
        self.assertTrue(timed_out)
        self.assertEqual(result.returncode, 124)

    def test_launch_failure_is_a_retained_attempt_result(self):
        result, timed_out = EVALUATOR.run_owned(["/does/not/exist"], Path.cwd(), 1)
        self.assertFalse(timed_out)
        self.assertEqual(result.returncode, 127)
        self.assertIn("FileNotFoundError", result.stderr)

    def test_timeout_stops_the_owned_process(self):
        result, timed_out = EVALUATOR.run_owned(
            [sys.executable, "-c", "import time; time.sleep(10)"], Path.cwd(), 0.05)
        self.assertTrue(timed_out)
        self.assertEqual(result.returncode, 124)


if __name__ == "__main__":
    unittest.main()
