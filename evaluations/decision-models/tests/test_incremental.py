import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


DIRECTORY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(DIRECTORY))
SPEC = importlib.util.spec_from_file_location("incremental", DIRECTORY / "incremental.py")
incremental = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(incremental)


class IncrementalTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.corpus = incremental.build_corpus()
        cls.frozen = incremental.freeze(cls.corpus)

    def test_corpus_and_freeze_are_deterministic(self):
        self.assertEqual(self.corpus, incremental.build_corpus())
        self.assertEqual(self.frozen, incremental.freeze(self.corpus))
        incremental.validate_corpus(self.corpus)

    def test_family_translations_asr_and_prefixes_never_cross_splits(self):
        groups = {}
        for prefix in self.corpus["prefixes"]:
            groups.setdefault(prefix["group_id"], set()).add(prefix["split"])
        self.assertTrue(all(len(splits) == 1 for splits in groups.values()))
        family = self.corpus["prefixes"][0]["group_id"]
        related = [p for p in self.corpus["prefixes"] if p["group_id"] == family]
        self.assertEqual({p["language"] for p in related}, {"en", "nl", "mixed"})
        self.assertEqual({p["variant"] for p in related}, {"clean", "asr"})

    def test_only_finalized_available_sources_and_acknowledged_notes_are_evidence(self):
        for prefix in self.corpus["prefixes"]:
            for source in prefix["utterances"]:
                self.assertTrue(source["finalized"])
                self.assertLessEqual(source["end_ms"], prefix["included_through_ms"])
            self.assertNotIn("PROVISIONAL_FUTURE_SENTINEL", incremental.prefix_input(prefix)["state"])
            for note in prefix["notes"]:
                self.assertTrue(note["acknowledged"])
                self.assertLessEqual(note["available_ms"], prefix["included_through_ms"])

    def test_later_source_revision_does_not_mutate_an_earlier_prefix(self):
        corpus = copy.deepcopy(self.corpus)
        meeting = next(prefix["meeting_uid"] for prefix in corpus["prefixes"] if prefix["utterances"])
        related = [prefix for prefix in corpus["prefixes"] if prefix["meeting_uid"] == meeting and prefix["utterances"]]
        earlier, later = related[0], related[-1]
        original = earlier["utterances"][0]["text"]
        later["utterances"][0]["text"] = "An owned later replacement."
        later["utterances"][0]["revision"] += 1
        self.assertEqual(earlier["utterances"][0]["text"], original)
        self.assertEqual(earlier["utterances"][0]["revision"], 1)

    def test_future_note_or_interim_source_is_rejected(self):
        for field in ("note", "interim"):
            corpus = copy.deepcopy(self.corpus)
            prefix = next(p for p in corpus["prefixes"] if p["utterances"])
            if field == "note":
                prefix["notes"].append({"id": "future", "revision": 1, "text": "future", "acknowledged": True,
                                        "available_ms": prefix["included_through_ms"] + 1})
            else:
                prefix["utterances"][0]["finalized"] = False
            with self.assertRaises(ValueError):
                incremental.validate_corpus(corpus)

    def test_shared_meeting_uid_cannot_hide_a_family_split(self):
        corpus = copy.deepcopy(self.corpus)
        corpus["prefixes"][1]["group_id"] = "different-family"
        with self.assertRaises(ValueError):
            incremental.validate_corpus(corpus)

    def test_finalized_and_acknowledged_flags_are_boolean_not_truthy_strings(self):
        corpus = copy.deepcopy(self.corpus)
        next(prefix for prefix in corpus["prefixes"] if prefix["utterances"])["utterances"][0]["finalized"] = "true"
        with self.assertRaises(ValueError):
            incremental.validate_corpus(corpus)

    def test_requests_have_no_gold_or_later_content(self):
        requests = list(incremental.request_rows(self.corpus, self.frozen, "heldout"))
        self.assertTrue(requests)
        for row in requests[:50]:
            encoded = json.dumps(row)
            self.assertNotIn("expected_items", encoded)
            self.assertNotIn("expected_components", encoded)
            self.assertNotIn("PROVISIONAL_FUTURE_SENTINEL", encoded)
            self.assertEqual(row["freeze_sha256"], self.frozen["freeze_sha256"])

    def test_freeze_rejects_any_source_protocol_or_configuration_change(self):
        mutated = copy.deepcopy(self.corpus)
        mutated["prefixes"][0]["included_through_ms"] += 1
        with self.assertRaises(ValueError):
            incremental.verify_freeze(mutated, self.frozen)
        changed = copy.deepcopy(self.frozen)
        changed["configuration"]["configured"]["model"] = "another-model"
        with self.assertRaises(ValueError):
            incremental.verify_freeze(self.corpus, changed)

    def test_duplicate_or_unlinked_response_cannot_inflate_metrics(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        for mutation in ("duplicate", "input", "unknown"):
            rows = copy.deepcopy(predictions[:1])
            if mutation == "duplicate":
                rows *= 2
            elif mutation == "input":
                rows[0]["input_sha256"] = "0" * 64
            else:
                rows[0]["case_id"] = "unknown-case"
            with self.assertRaises(ValueError):
                incremental.evaluate(self.corpus, self.frozen, rows)

    def test_missing_and_failed_predictions_remain_in_denominator(self):
        report = incremental.evaluate(self.corpus, self.frozen, [])
        self.assertEqual(report["scheduled_cases"], len(self.corpus["cases"]))
        self.assertEqual(report["observed_cases"], 0)
        self.assertEqual(report["missing_cases"], report["scheduled_cases"])
        self.assertTrue(all(group["classification"]["coverage"] == 0 for group in report["groups"]))
        self.assertTrue(all(not record["qualified"] for record in report["qualification_records"]))

    def test_perfect_fixture_replay_is_not_model_or_live_qualification(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        report = incremental.evaluate(self.corpus, self.frozen, predictions)
        self.assertEqual(report["observed_cases"], len(self.corpus["cases"]))
        self.assertEqual(report["adopted_records"], [])
        self.assertTrue(all(not record["qualified"] for record in report["qualification_records"]))
        self.assertTrue(all("actual_inference_unmeasured" in record["gates"] for record in report["qualification_records"]))

    def test_future_or_changed_source_quote_is_an_error_not_accepted_evidence(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        target = next(row for row in predictions if row["case_id"].startswith("incremental-heldout-") and row.get("items"))
        target["items"][0]["evidence"][0]["utterance_id"] = "future-utterance"
        report = incremental.evaluate(self.corpus, self.frozen, predictions)
        self.assertGreater(sum(group["items"]["unsupported_sources"] for group in report["groups"]), 0)
        self.assertTrue(any((group["items"]["precision"] or 0) < 1 for group in report["groups"] if group["items"]["proposed_count"]))

    def test_owner_deadline_and_duplicate_metrics_are_independent(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        target = next(row for row in predictions if row["case_id"].startswith("incremental-heldout-") and row.get("items"))
        item = target["items"][0]
        item["owner"] = {"text": "Invented owner", "evidence": []}
        item["due_phrase"] = {"text": "2099-01-01", "evidence": []}
        target["items"].append(copy.deepcopy(item))
        report = incremental.evaluate(self.corpus, self.frozen, predictions)
        self.assertGreater(sum(group["items"]["owner_errors"] for group in report["groups"]), 0)
        self.assertGreater(sum(group["items"]["due_phrase_errors"] for group in report["groups"]), 0)
        self.assertGreater(sum(group["items"]["duplicate_suggestions"] for group in report["groups"]), 0)

    def test_generated_proposal_ids_are_matched_by_evidence_not_hidden_gold_keys(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        for index, row in enumerate(predictions):
            for item_index, item in enumerate(row.get("items", [])):
                item["key"] = f"application-id-{index}-{item_index}"
        report = incremental.evaluate(self.corpus, self.frozen, predictions)
        self.assertTrue(all(group["items"]["precision"] == 1 for group in report["groups"] if group["items"]["proposed_count"]))

    def test_reviewed_edits_and_dismissals_are_explicit_input_and_survive_regeneration(self):
        request = next(row for row in incremental.request_rows(self.corpus, self.frozen, "heldout") if row["review_input"]["accepted_edits"])
        self.assertNotIn("User-edited wording", request["decision_request"]["state"])
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        changed = next(row for row in predictions if row["case_id"] == request["case_id"])
        changed["review_state"] = {"accepted_edits": {}, "dismissed": []}
        report = incremental.evaluate(self.corpus, self.frozen, predictions)
        self.assertTrue(any("reviewed_state_not_preserved" in group["gates"] for group in report["groups"]))

    def test_calibration_never_uses_heldout_predictions(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        before = incremental.evaluate(self.corpus, self.frozen, predictions)["calibration"]
        cases = {case["id"]: case for case in self.corpus["cases"]}
        for row in predictions:
            case = cases[row["case_id"]]
            if case["split"] == "heldout":
                for component in row["components"]:
                    component["prediction"] = None
                    component["probabilities"] = None
        after = incremental.evaluate(self.corpus, self.frozen, predictions)["calibration"]
        self.assertEqual(before, after)

    def test_connection_samples_and_identity_drift_do_not_qualify(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        predictions[0]["evidence_class"] = "synthetic_connection"
        report = incremental.evaluate(self.corpus, self.frozen, predictions)
        self.assertFalse(any(record["qualified"] for record in report["qualification_records"]))
        predictions[0]["identity"]["model"] = "changed-alias"
        report = incremental.evaluate(self.corpus, self.frozen, predictions)
        self.assertGreater(report["identity_mismatch_cases"], 0)

    def test_missing_resource_measurements_are_unavailable_not_zero(self):
        report = incremental.evaluate(self.corpus, self.frozen, [])
        for group in report["groups"]:
            self.assertIsNone(group["resources"]["peak_rss_bytes"])
            self.assertIsNone(group["resources"]["warm_latency"]["p95_ms"])

    def test_type_seven_latency_and_unknown_request_counts_are_reported_honestly(self):
        values = [{"evidence_class": "recorded_inference", "measurements": {"phase": "warm", "latency_ms": value,
                   "peak_rss_bytes": 1000, "cpu_time_ms": 1}} for value in range(1, 32)]
        result = incremental._resources(values)
        self.assertEqual(result["warm_latency"], {"samples": 31, "p50_ms": 16, "p95_ms": 29.5})
        self.assertEqual(result["cpu_time_ms"], 31)
        self.assertIsNone(result["executed_requests"])
        self.assertIsNone(result["usage"])
        self.assertIsNone(result["cost_usd"])

    def test_generated_claim_support_is_separate_from_fixture_checks(self):
        report = incremental.evaluate(self.corpus, self.frozen, incremental.fixture_predictions(self.corpus, self.frozen))
        source_groups = [group for group in report["groups"] if group["use_case"] == "source_assessment"]
        self.assertTrue(source_groups)
        self.assertTrue(all(group["generated_claims"]["count"] == 0 for group in source_groups))
        self.assertTrue(all("generated_claims_unmeasured" in group["gates"] for group in source_groups))

    def test_source_assessment_gold_covers_partial_uncertain_absent_and_explicit_negation(self):
        cases = [case for case in self.corpus["cases"] if case["use_case"] == "source_assessment"]
        self.assertEqual({case["expected_components"]["claim_support"] for case in cases}, {"supported", "contradicted", "insufficient", "uncertain"})
        self.assertEqual({case["expected_components"]["answerability"] for case in cases}, {"answerable", "partial", "absent", "uncertain"})
        tentative = next(case for case in cases if case["id"].startswith("incremental-heldout-00-en-clean-p01:"))
        self.assertEqual(tentative["expected_components"]["claim_support"], "contradicted")

    def test_pending_generated_answer_reviews_do_not_change_suggestion_qualification_scope(self):
        corpus = copy.deepcopy(self.corpus)
        corpus["pending_generated_judgments"] = [{"language": "en", "split": "heldout", "response_sha256": "0" * 64}]
        frozen = incremental.freeze(corpus)
        report = incremental.evaluate(corpus, frozen, incremental.fixture_predictions(corpus, frozen))
        by_context = {(record["context"]["use_case"], record["context"]["language_policy"]): record for record in report["qualification_records"]}
        self.assertFalse(by_context["source_assessment", "en"]["evidence"]["source_judgments_complete"])
        self.assertTrue(by_context["manual_finished_suggestions", "en"]["evidence"]["source_judgments_complete"])

    def test_measured_heldout_export_requires_previously_frozen_calibration(self):
        configuration = incremental._default_configuration()
        configuration["configured"]["protocol"] = "local_worker"
        initial = incremental.freeze(self.corpus, configuration)
        with self.assertRaises(ValueError):
            list(incremental.request_rows(self.corpus, initial, "heldout"))
        calibration_ids = {case["id"] for case in self.corpus["cases"] if case["split"] == "calibration"}
        rows = [row for row in incremental.fixture_predictions(self.corpus, initial) if row["case_id"] in calibration_ids]
        frozen = incremental.calibrate(self.corpus, initial, rows)
        self.assertEqual(frozen["stage"], "heldout_ready")
        self.assertTrue(list(incremental.request_rows(self.corpus, frozen, "heldout")))
        changed = copy.deepcopy(frozen)
        changed["calibration_artifact"]["fits"] = {}
        with self.assertRaises(ValueError):
            incremental.verify_freeze(self.corpus, changed)

    def test_calibration_rejects_a_heldout_row_even_when_labels_are_correct(self):
        heldout = next(row for row in incremental.fixture_predictions(self.corpus, self.frozen)
                       if row["case_id"].startswith("incremental-heldout-"))
        with self.assertRaises(ValueError):
            incremental.calibrate(self.corpus, self.frozen, [heldout])

    def test_nonfinite_or_negative_measurements_are_rejected(self):
        for value in (-1, float("nan"), float("inf"), True):
            predictions = incremental.fixture_predictions(self.corpus, self.frozen)[:1]
            predictions[0]["measurements"] = {"latency_ms": value}
            with self.assertRaises(ValueError):
                incremental.evaluate(self.corpus, self.frozen, predictions)

    def test_probabilistic_fixture_can_not_be_relabelled_as_actual_inference(self):
        rows = incremental.fixture_predictions(self.corpus, self.frozen)
        for row in rows:
            row["evidence_class"] = "recorded_inference"
        report = incremental.evaluate(self.corpus, self.frozen, rows)
        self.assertTrue(all("calibration_not_frozen_before_heldout" in group["gates"] for group in report["groups"]))
        self.assertFalse(any(record["qualified"] for record in report["qualification_records"]))

    def test_credentials_and_secret_bearing_endpoints_are_not_frozen(self):
        for update in ("key", "url", "query"):
            configuration = incremental._default_configuration()
            if update == "key":
                configuration["resource_profile"]["access_token"] = "not-a-real-secret"
            else:
                configuration["configured"]["endpoint"] = "https://user:password@example.invalid/v1" if update == "url" else "https://example.invalid/v1?token=not-real"
            with self.assertRaises(ValueError):
                incremental.freeze(self.corpus, configuration)

    def test_export_uses_the_existing_typed_decision_contract(self):
        row = next(iter(incremental.request_rows(self.corpus, self.frozen, "development")))
        request = row["decision_request"]
        self.assertEqual(request["state"], row["state"])
        for question in request["questions"]:
            self.assertEqual(question["kind"], "choice")
            self.assertEqual([option["id"] for option in question["options"]], row["questions"][question["id"]]["labels"])

    def test_context_refusal_never_truncates_or_accepts_a_response(self):
        configuration = incremental._default_configuration()
        configuration["configured"]["max_input_bytes"] = 1
        frozen = incremental.freeze(self.corpus, configuration)
        request = next(iter(incremental.request_rows(self.corpus, frozen, "development")))
        self.assertEqual(request["status"], "over_context")
        self.assertGreater(len(request["state"].encode()), 1)
        with self.assertRaises(ValueError):
            incremental.evaluate(self.corpus, frozen, incremental.fixture_predictions(self.corpus, frozen)[:1])

    def test_replay_from_stored_inputs_reproduces_counts_gates_and_hashes(self):
        predictions = incremental.fixture_predictions(self.corpus, self.frozen)
        first = incremental.evaluate(self.corpus, self.frozen, predictions)
        second = incremental.evaluate(json.loads(json.dumps(self.corpus, sort_keys=True)), json.loads(json.dumps(self.frozen, sort_keys=True)), json.loads(json.dumps(predictions, sort_keys=True)))
        self.assertEqual(first, second)
        self.assertTrue(all(record["evidence"]["mode"] == "offline_fixture" for record in first["qualification_records"]))

    def test_generated_claim_judgment_is_bound_to_response_cutoff_and_exact_sources(self):
        case = copy.deepcopy(next(case for case in self.corpus["cases"] if case["id"].startswith("incremental-heldout-00-en-clean-p02:source_assessment")))
        prefix = next(prefix for prefix in self.corpus["prefixes"] if prefix["id"] == case["prefix_id"])
        source = prefix["utterances"][-1]
        anchor = {key: source[key] for key in ("utterance_id", "revision", "source_id", "source_fingerprint", "start_ms", "end_ms")}
        anchor.update(start_char=0, end_char=len(source["text"]), text=source["text"])
        response_text = case["claim"] + " [1]"
        response_hash = incremental.hashlib.sha256(response_text.encode()).hexdigest()
        body = {"rubric_version": "incremental-meeting-v1", "response_sha256": response_hash, "claim": case["claim"],
                "label": case["expected_components"]["claim_support"], "source_anchors": [anchor],
                "source_scope_sha256": incremental.digest(incremental.prefix_input(prefix))}
        case["generated_response"] = {"text": response_text, "sha256": response_hash, "claim_start_char": 0,
                                      "claim_end_char": len(case["claim"]),
                                      "judgment": {"label": body["label"], "source_anchors": [anchor], "sha256": incremental.digest(body)}}
        self.assertTrue(incremental._generated_claim_valid(case, prefix))
        case["generated_response"]["judgment"]["source_anchors"][0]["source_fingerprint"] = "0" * 64
        self.assertFalse(incremental._generated_claim_valid(case, prefix))

    def test_unicode_source_fingerprints_follow_shared_core_normalization(self):
        source = {"utterance_id": "u-bom", "source_id": "microphone", "start_ms": 0, "end_ms": 1000,
                  "text": "\ufeffalpha\u00a0\u2003beta\n😀\ufeff", "failed": False}
        self.assertEqual(incremental.source_fingerprint(source), "8a1840c80592f1692d9de9405f944333752b7b3fbb8e3d9674b03ceea425ff96")

    def test_offline_cli_writes_replayable_report_without_any_inference(self):
        with tempfile.TemporaryDirectory() as directory:
            process = subprocess.run([sys.executable, str(DIRECTORY / "incremental.py"), "--offline", "--output", directory],
                                     capture_output=True, text=True, timeout=120)
            self.assertEqual(process.returncode, 0, process.stderr)
            report = json.loads((Path(directory) / "report.json").read_text())
            self.assertEqual(report["adopted_records"], [])
            self.assertTrue((Path(directory) / "freeze.json").is_file())
            self.assertNotIn("Synthetic action", process.stdout)


if __name__ == "__main__":
    unittest.main()
