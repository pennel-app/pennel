#!/usr/bin/env python3
"""Freeze source-bound prefix experiments and replay recorded results without inference."""

import argparse
from collections import Counter, defaultdict
import copy
import hashlib
import json
import math
from pathlib import Path
import sys
from urllib.parse import urlsplit

DIRECTORY = Path(__file__).resolve().parent
sys.path.insert(0, str(DIRECTORY))

import calibration
import metrics
from incremental_corpus import LANGUAGES, USE_CASES, build_corpus, digest, source_fingerprint, validate_corpus

PROTOCOL_PATH = DIRECTORY / "protocol-incremental-v1.json"


def protocol():
    return json.loads(PROTOCOL_PATH.read_text(encoding="utf-8"))


def _file_hash(path):
    return hashlib.sha256(Path(path).read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def _default_configuration():
    return {"configured": {"protocol": "offline_fixture", "endpoint": "offline://owned-fixtures", "model": "owned-fixture",
                           "expected_model": "owned-fixture", "upstream": None, "max_input_bytes": 16000, "timeout_secs": 30},
            "actual": {"model": "owned-fixture", "revision": "incremental-fixture-v1", "provider": "owned-fixture",
                       "artifact_sha256": digest("owned-offline-fixture-not-a-model")},
            "precision": "fixture", "grouping": "one-prefix-independent-questions",
            "preprocessing": {"version": 1, "finalized_only": True, "notes": "acknowledged-at-cutoff", "context": "complete-prefix-or-refuse"},
            "resource_profile": {"mode": "offline_fixture", "hardware": "unmeasured"}}


def _configuration(configuration):
    value = copy.deepcopy(configuration if configuration is not None else _default_configuration())
    def no_credentials(item):
        if isinstance(item, dict):
            if any(key.lower() in {"api_key", "access_token", "refresh_token", "authorization", "password", "secret", "bearer"} for key in item):
                raise ValueError("credentials_forbidden_in_frozen_configuration")
            for child in item.values(): no_credentials(child)
        elif isinstance(item, list):
            for child in item: no_credentials(child)
    no_credentials(value)
    if set(value) != {"configured", "actual", "preprocessing", "resource_profile", "grouping", "precision"}:
        raise ValueError("unknown_configuration_field")
    for key in ("configured", "actual", "preprocessing", "resource_profile"):
        if not isinstance(value.get(key), dict):
            raise ValueError("missing_configuration_context:" + key)
    for key in ("protocol", "endpoint", "model", "expected_model", "upstream", "max_input_bytes", "timeout_secs"):
        if key not in value["configured"]:
            raise ValueError("missing_configured_identity:" + key)
    if set(value["configured"]) != {"protocol", "endpoint", "model", "expected_model", "upstream", "max_input_bytes", "timeout_secs"}:
        raise ValueError("unknown_configured_identity_field")
    for key in ("protocol", "endpoint", "model"):
        if not isinstance(value["configured"][key], str) or not value["configured"][key]:
            raise ValueError("invalid_configured_identity:" + key)
    if set(value["actual"]) != {"model", "revision", "provider", "artifact_sha256"}:
        raise ValueError("unknown_actual_identity_field")
    for key in ("model", "revision", "provider", "artifact_sha256"):
        if not isinstance(value["actual"].get(key), str) or not value["actual"][key]:
            raise ValueError("missing_actual_identity:" + key)
    expected_model = value["configured"]["expected_model"]
    if expected_model is not None and expected_model != value["actual"]["model"]:
        raise ValueError("configured_expected_model_mismatch")
    endpoint = urlsplit(value["configured"]["endpoint"])
    if endpoint.username or endpoint.password or endpoint.query or endpoint.fragment:
        raise ValueError("credential_or_query_endpoint_forbidden")
    for key in ("max_input_bytes", "timeout_secs"):
        if isinstance(value["configured"][key], bool) or not isinstance(value["configured"][key], int) or value["configured"][key] <= 0:
            raise ValueError("invalid_configured_limit:" + key)
    if not value.get("grouping") or not value.get("precision"):
        raise ValueError("missing_prompt_execution_context")
    return value


def _questions(use_case):
    if use_case == "source_assessment":
        return {"claim_support": {"task": "claim_support", "labels": protocol()["support_labels"],
                                  "instructions": "Assess the supplied claim only against these finalized sources, including available retractions. Related text is not support."},
                "answerability": {"task": "answerability", "labels": protocol()["answerability_labels"],
                                   "instructions": "Can these exact finalized sources answer the material question in the supplied claim? Distinguish answerable, absent, partial and uncertain evidence; do not infer missing information."}}
    return {key: {"task": "events", "labels": protocol()["event_labels"],
                  "instructions": "Classify the current " + description + ". Consider tentative language, negation, later changes and retractions only through this cutoff. Return only the allowed choice; complete items and exact owner/due spans are scored in a separate pipeline stage."}
            for key, description in (("action", "checklist commitment"), ("decision", "cabinet decision"), ("question", "backup-key question"))}


def freeze(corpus, configuration=None, calibration_artifact=None, calibration_predictions=None):
    validate_corpus(corpus)
    configuration = _configuration(configuration)
    value = {"schema_version": 1, "protocol_id": "incremental-meeting-v1", "configuration": configuration,
             "protocol_sha256": _file_hash(PROTOCOL_PATH), "base_protocol_sha256": _file_hash(DIRECTORY / "protocol.json"),
             "corpus_sha256": digest(corpus), "split_sha256": digest(sorted({(p["group_id"], p["split"]) for p in corpus["prefixes"]})),
             "prompt_sha256": digest({use_case: _questions(use_case) for use_case in USE_CASES}),
             "schema_sha256": digest({"version": 1, "item": ["key", "kind", "status", "text", "evidence", "owner", "due_phrase", "supersedes"],
                                      "anchor": ["utterance_id", "revision", "source_id", "source_fingerprint", "start_char", "end_char", "text", "start_ms", "end_ms"]}),
             "label_order_sha256": digest({use_case: {key: question["labels"] for key, question in _questions(use_case).items()} for use_case in USE_CASES}),
             "preprocessing_sha256": digest(configuration["preprocessing"]), "resource_profile_sha256": digest(configuration["resource_profile"]),
             "source_sha256": {name: _file_hash(DIRECTORY / name) for name in ("incremental.py", "incremental_corpus.py", "metrics.py", "calibration.py")},
             "stage": "offline_replay" if configuration["configured"]["protocol"] == "offline_fixture" else "calibration",
             "calibration_artifact": calibration_artifact, "calibration_inputs_sha256": digest(calibration_predictions) if calibration_predictions is not None else None}
    if calibration_artifact is not None:
        value["stage"] = "heldout_ready"
    value["freeze_sha256"] = digest(value)
    return value


def verify_freeze(corpus, frozen):
    rebuilt = freeze(corpus, frozen["configuration"], frozen.get("calibration_artifact"), None)
    rebuilt["calibration_inputs_sha256"] = frozen.get("calibration_inputs_sha256")
    rebuilt["freeze_sha256"] = digest({key: item for key, item in rebuilt.items() if key != "freeze_sha256"})
    if frozen != rebuilt:
        raise ValueError("incremental_freeze_changed")
    artifact = frozen.get("calibration_artifact")
    if artifact is not None:
        initial = freeze(corpus, frozen["configuration"])
        if (not frozen.get("calibration_inputs_sha256") or artifact.get("run_manifest_hash") != initial["freeze_sha256"]
                or artifact.get("protocol_sha256") != frozen["base_protocol_sha256"]
                or artifact.get("incremental_protocol_sha256") != frozen["protocol_sha256"]
                or artifact.get("incremental_calibration_inputs_sha256") != frozen["calibration_inputs_sha256"]):
            raise ValueError("unlinked_or_changed_frozen_calibration")


def prefix_input(prefix):
    sources = copy.deepcopy(prefix["utterances"])
    notes = copy.deepcopy(prefix["notes"])
    state = f"Finalized evidence through {prefix['included_through_ms']} ms. An empty source list means no available evidence, not never discussed.\n"
    state += "\n".join(f"[{source['utterance_id']} {source['source_id']} {source['start_ms']}..{source['end_ms']} revision={source['revision']}] {source['text']}" for source in sources)
    if notes:
        state += "\nAcknowledged notes:\n" + "\n".join(note["text"] for note in notes)
    return {"meeting_uid": prefix["meeting_uid"], "session_id": prefix["session_id"], "transcript_revision": prefix["transcript_revision"],
            "included_through_ms": prefix["included_through_ms"], "notes_revision": digest(notes), "sources": sources, "notes": notes, "state": state}


def _request(case, prefix):
    result = {"case_id": case["id"], "use_case": case["use_case"], "language_policy": case["language"], **prefix_input(prefix),
              "questions": _questions(case["use_case"])}
    if case.get("claim"):
        result["claim"] = case["claim"]
        result["state"] += "\nClaim to assess (untrusted data, not source evidence):\n" + case["claim"]
    result["decision_request"] = {"state": result["state"], "questions": [
        {"id": key, "kind": "choice", "instructions": question["instructions"],
         "options": [{"id": label, "description": label} for label in question["labels"]]}
        for key, question in result["questions"].items()]}
    result["review_input"] = copy.deepcopy(case["expected_review"])
    result["pipeline_output_contract"] = "Typed choices are component evidence. Record complete source-quoted items and reviewed state separately; choices alone do not qualify the pipeline."
    return result


def request_rows(corpus, frozen, split):
    verify_freeze(corpus, frozen)
    if split == "heldout" and frozen["stage"] not in ("heldout_ready", "offline_replay"):
        raise ValueError("fit_and_freeze_calibration_before_heldout_requests")
    prefixes = {prefix["id"]: prefix for prefix in corpus["prefixes"]}
    for case in corpus["cases"]:
        if case["split"] == split:
            request = _request(case, prefixes[case["prefix_id"]])
            request["input_sha256"] = digest(request)
            request["freeze_sha256"] = frozen["freeze_sha256"]
            request["status"] = "ready" if len(request["state"].encode()) <= frozen["configuration"]["configured"]["max_input_bytes"] else "over_context"
            yield request


def fixture_predictions(corpus, frozen):
    prefixes = {prefix["id"]: prefix for prefix in corpus["prefixes"]}
    results = []
    for case in corpus["cases"]:
        request = _request(case, prefixes[case["prefix_id"]])
        questions = _questions(case["use_case"])
        components = []
        for key, expected in case["expected_components"].items():
            labels = questions[key]["labels"]
            components.append({"id": key, "prediction": expected,
                               "probabilities": {label: .96 if label == expected else .04 / (len(labels) - 1) for label in labels},
                               "score_type": "categorical_probabilities"})
        results.append({"case_id": case["id"], "freeze_sha256": frozen["freeze_sha256"], "input_sha256": digest(request),
                        "identity": copy.deepcopy(frozen["configuration"]["actual"]), "status": "ok", "evidence_class": "offline_fixture",
                        "components": components, "items": copy.deepcopy(case["expected_items"]),
                        "review_state": copy.deepcopy(case["expected_review"]), "measurements": {}})
    return results


def _expanded(cases, predictions, frozen):
    rows = []
    configuration = frozen["configuration"]
    for case in cases:
        prediction = predictions.get(case["id"], {})
        valid = prediction.get("status") == "ok" and prediction.get("identity") == configuration["actual"]
        components = {}
        for component in prediction.get("components", []):
            if component.get("id") in components:
                raise ValueError("duplicate_component_id")
            components[component.get("id")] = component
        questions = _questions(case["use_case"])
        if set(components) - set(questions):
            raise ValueError("unknown_component_id")
        for key, expected in case["expected_components"].items():
            component, question = components.get(key, {}), questions[key]
            probabilities = component.get("probabilities") if valid else None
            if probabilities is not None:
                metrics.validate_probabilities(probabilities, question["labels"])
            label = component.get("prediction") if valid else None
            if label is not None and label not in question["labels"]:
                raise ValueError("unknown_prediction_label")
            identity = {**configuration["actual"], "transport": configuration["configured"]["protocol"],
                        "endpoint": configuration["configured"]["endpoint"], "precision": configuration["precision"],
                        "frozen_configuration": configuration, "incremental_protocol": frozen["protocol_sha256"]}
            rows.append({"id": case["id"] + ":" + key, "case_id": case["id"], "runner_id": "incremental-config", "language": case["language"],
                         "split": case["split"], "group_id": case["group_id"], "task": question["task"], "subtask": case["use_case"] + ":" + key,
                         "expected": expected, "prediction": label, "probabilities": probabilities,
                         "raw_score_type": component.get("score_type", "unavailable"), "label_order": question["labels"],
                         "question": {"type": "choice", "instructions": question["instructions"], "criteria": {label: label for label in question["labels"]}},
                         "prompt_hash": digest(question), "grouping": configuration["grouping"], "identity": identity,
                         "run_manifest_hash": frozen["freeze_sha256"], "use_case": case["use_case"], "component": key})
    return rows


def calibrate(corpus, frozen, predictions):
    verify_freeze(corpus, frozen)
    by_id = _validate_predictions(corpus, frozen, predictions)
    cases = [case for case in corpus["cases"] if case["id"] in by_id]
    if any(case["split"] == "heldout" for case in cases):
        raise ValueError("heldout_predictions_forbidden_during_calibration")
    rows = _expanded(cases, by_id, frozen)
    artifact = calibration.fit_predictions(rows, run_manifest_hash=frozen["freeze_sha256"])
    artifact["incremental_protocol_sha256"] = frozen["protocol_sha256"]
    artifact["incremental_calibration_inputs_sha256"] = digest(predictions)
    artifact["incremental_evidence_mode"] = "recorded_inference" if predictions and all(
        row.get("evidence_class") == "recorded_inference" and row.get("identity") == frozen["configuration"]["actual"]
        for row in predictions) else "offline_fixture"
    return freeze(corpus, frozen["configuration"], artifact, predictions)


def _validate_predictions(corpus, frozen, predictions):
    cases = {case["id"]: case for case in corpus["cases"]}
    prefixes = {prefix["id"]: prefix for prefix in corpus["prefixes"]}
    result = {}
    for row in predictions:
        case_id = row.get("case_id")
        if case_id in result or case_id not in cases:
            raise ValueError("duplicate_or_unknown_prediction")
        case = cases[case_id]
        request = _request(case, prefixes[case["prefix_id"]])
        if row.get("freeze_sha256") != frozen["freeze_sha256"] or row.get("input_sha256") != digest(request):
            raise ValueError("unlinked_prediction_or_changed_source")
        if row.get("status") == "ok" and len(request["state"].encode()) > frozen["configuration"]["configured"]["max_input_bytes"]:
            raise ValueError("successful_response_for_refused_context")
        if row.get("evidence_class") not in ("offline_fixture", "synthetic_connection", "recorded_inference"):
            raise ValueError("unknown_execution_evidence")
        for field in ("latency_ms", "peak_rss_bytes", "cpu_time_ms", "executed_requests", "input_bytes"):
            value = row.get("measurements", {}).get(field)
            if value is not None and (isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0):
                raise ValueError("invalid_resource_measurement:" + field)
        result[case_id] = row
    return result


def _calibrated(row, artifact):
    value = {**row, "accepted": False, "prediction": None, "probabilities": None, "calibration_status": "unavailable"}
    try:
        if row.get("probabilities") is None or row.get("raw_score_type") not in metrics.PROBABILITY_TYPES:
            return value
        context = calibration.prediction_context(row)
        fitted = artifact["fits"].get(metrics._context_hash(context))
        if not fitted or fitted["status"] != "fitted":
            return value
        probability = metrics.apply_calibration(row["probabilities"], fitted["fit"], context)
        chosen = max(sorted(probability), key=probability.get)
        threshold = fitted["fit"]["threshold"]["threshold"]
        return {**row, "probabilities": probability, "prediction": chosen, "score_type": "calibrated_probabilities",
                "accepted": threshold is not None and probability[chosen] >= threshold, "calibration_status": "calibrated"}
    except (ValueError, KeyError, TypeError):
        return value


def _anchor_valid(anchor, prefix):
    source = next((source for source in prefix["utterances"] if source["utterance_id"] == anchor.get("utterance_id")), None)
    if source is None:
        return False
    start, end = anchor.get("start_char"), anchor.get("end_char")
    return (isinstance(start, int) and not isinstance(start, bool) and isinstance(end, int) and not isinstance(end, bool)
            and 0 <= start < end <= len(source["text"]) and source["text"][start:end] == anchor.get("text")
            and all(anchor.get(field) == source[field] for field in ("revision", "source_id", "source_fingerprint", "start_ms", "end_ms")))


def _field_valid(value, expected, prefix):
    if expected is None:
        return value is None
    return (isinstance(value, dict) and value.get("text") == expected["text"] and bool(value.get("evidence"))
            and all(_anchor_valid(anchor, prefix) and value["text"] in anchor["text"] for anchor in value["evidence"]))


def _score_items(case, prefix, response):
    gold = {item["key"]: item for item in case["expected_items"]}
    items = response.get("items", [])
    if not isinstance(items, list):
        raise ValueError("invalid_item_result")
    result = Counter({key: 0 for key in ("expected_count", "proposed_count", "duplicate_suggestions", "anchor_count", "valid_anchors",
                                       "unsupported_sources", "owner_errors", "due_phrase_errors", "final_status_errors", "correct_count",
                                       "missed_items", "false_positive_suggestions", "review_expected", "review_retained")})
    result.update(expected_count=len(gold), proposed_count=len(items))
    seen, correct = set(), set()
    for item in items:
        if not isinstance(item, dict):
            result["unsupported_sources"] += 1
            continue
        anchors = item.get("evidence", [])
        supplied_ids = {anchor.get("utterance_id") for anchor in anchors}
        matching = [expected for expected in gold.values()
                    if item.get("text") == expected["text"]
                    or {anchor["utterance_id"] for anchor in expected["evidence"]}.issubset(supplied_ids)]
        expected = matching[0] if len(matching) == 1 else None
        key = expected["key"] if expected else digest([item.get("kind"), item.get("status"), item.get("text"), sorted(str(value) for value in supplied_ids)])
        duplicate = key in seen
        seen.add(key)
        result["duplicate_suggestions"] += duplicate
        source_valid = bool(anchors) and all(_anchor_valid(anchor, prefix) for anchor in anchors)
        required = {anchor["utterance_id"] for anchor in expected["evidence"]} if expected else set()
        source_valid = source_valid and required.issubset(supplied_ids)
        result["anchor_count"] += len(anchors)
        result["valid_anchors"] += sum(_anchor_valid(anchor, prefix) for anchor in anchors)
        result["unsupported_sources"] += not source_valid
        owner_ok = expected is not None and _field_valid(item.get("owner"), expected["owner"], prefix)
        due_ok = expected is not None and _field_valid(item.get("due_phrase"), expected["due_phrase"], prefix)
        final_ok = expected is not None and item.get("status") == expected["status"]
        result["owner_errors"] += not owner_ok
        result["due_phrase_errors"] += not due_ok
        result["final_status_errors"] += not final_ok
        exact = (expected is not None and not duplicate and source_valid and owner_ok and due_ok and final_ok
                 and all(item.get(field) == expected.get(field) for field in ("kind", "text", "supersedes")))
        if exact:
            correct.add(key)
    result["correct_count"] = len(correct)
    result["missed_items"] = len(gold) - len(correct)
    result["false_positive_suggestions"] = len(items) - len(correct)
    expected_review = case["expected_review"]
    supplied_review = response.get("review_state")
    result["review_expected"] = len(expected_review["accepted_edits"]) + len(expected_review["dismissed"])
    result["review_retained"] = result["review_expected"] if supplied_review == expected_review else 0
    return result, correct


def _ratio(a, b):
    return a / b if b else None


def _generated_claim_valid(case, prefix):
    generated = case.get("generated_response", {})
    text = generated.get("text")
    start, end = generated.get("claim_start_char"), generated.get("claim_end_char")
    judgment = generated.get("judgment", {})
    if (not isinstance(text, str) or generated.get("sha256") != hashlib.sha256(text.encode()).hexdigest()
            or not isinstance(start, int) or isinstance(start, bool) or not isinstance(end, int) or isinstance(end, bool)
            or not 0 <= start < end <= len(text) or text[start:end] != case.get("claim")
            or judgment.get("label") != case["expected_components"].get("claim_support")):
        return False
    anchors = judgment.get("source_anchors", [])
    body = {"rubric_version": "incremental-meeting-v1", "response_sha256": generated["sha256"],
            "claim": case["claim"], "label": judgment.get("label"), "source_anchors": anchors,
            "source_scope_sha256": digest(prefix_input(prefix))}
    return (judgment.get("sha256") == digest(body) and all(_anchor_valid(anchor, prefix) for anchor in anchors)
            and (bool(anchors) or judgment.get("label") in ("insufficient", "uncertain")))


def _resources(responses):
    measurements = [row.get("measurements", {}) for row in responses if row.get("evidence_class") == "recorded_inference"]
    warm = [value["latency_ms"] for value in measurements if value.get("phase") == "warm" and isinstance(value.get("latency_ms"), (int, float))]
    cold = [value["latency_ms"] for value in measurements if value.get("phase") == "cold" and isinstance(value.get("latency_ms"), (int, float))]
    timing = lambda values: {"samples": len(values), "p50_ms": metrics.percentile(values, 50) if values else None,
                             "p95_ms": metrics.percentile(values, 95) if values else None}
    measured = lambda key: [value[key] for value in measurements if isinstance(value.get(key), (int, float)) and not isinstance(value.get(key), bool)]
    executed = measured("executed_requests")
    usage = [value["usage"] for value in measurements if value.get("usage") is not None]
    costs = [value["cost_usd"] for value in measurements if value.get("cost_usd") is not None]
    return {"warm_latency": timing(warm), "cold_latency": timing(cold),
            "peak_rss_bytes": max(measured("peak_rss_bytes"), default=None), "cpu_time_ms": sum(measured("cpu_time_ms")) if measured("cpu_time_ms") else None,
            "executed_requests": sum(executed) if executed and len(executed) == len(measurements) else None,
            "executed_input_bytes": sum(measured("input_bytes")) if measurements and len(measured("input_bytes")) == len(measurements) else None,
            "usage": usage or None, "cost_usd": costs or None,
            "foreground_impact_accepted": bool(measurements) and any(value.get("foreground_impact_accepted") is True for value in measurements),
            "background_resource_accepted": bool(measurements) and any(value.get("background_resource_accepted") is True for value in measurements)}


def evaluate(corpus, frozen, predictions):
    verify_freeze(corpus, frozen)
    by_id = _validate_predictions(corpus, frozen, list(predictions))
    expanded = _expanded(corpus["cases"], by_id, frozen)
    artifact = frozen.get("calibration_artifact")
    if artifact is None:
        artifact = calibration.fit_predictions(expanded, run_manifest_hash=frozen["freeze_sha256"])
    calibrated = [_calibrated(row, artifact) for row in expanded if row["split"] == "heldout"]
    prefixes = {prefix["id"]: prefix for prefix in corpus["prefixes"]}
    groups = []
    criteria = protocol()["acceptance"]
    for use_case in USE_CASES:
        for language in LANGUAGES:
            cases = [case for case in corpus["cases"] if case["split"] == "heldout" and case["use_case"] == use_case and case["language"] == language]
            rows = [row for row in calibrated if row["use_case"] == use_case and row["language"] == language]
            counts, end_to_end = Counter(), []
            for case in cases:
                response = by_id.get(case["id"], {})
                item_counts, correct = _score_items(case, prefixes[case["prefix_id"]], response)
                counts.update(item_counts)
                component_rows = {row["component"]: row for row in rows if row["case_id"] == case["id"]}
                expected_items = {item["key"]: item for item in case["expected_items"]}
                for component, expected in case["expected_components"].items():
                    if use_case == "source_assessment":
                        continue
                    key = {"action": "action-checklist", "decision": "decision-cabinet", "question": "question-key"}[component]
                    row = component_rows[component]
                    prediction = row["prediction"] if row["accepted"] else None
                    if expected != "none" and (key not in correct or key not in expected_items):
                        prediction = None
                    end_to_end.append({"expected": expected, "prediction": prediction})
            item_metrics = {**dict(counts), "precision": _ratio(counts["correct_count"], counts["proposed_count"]),
                            "recall": _ratio(counts["correct_count"], counts["expected_count"]),
                            "anchor_fidelity": _ratio(counts["valid_anchors"], counts["anchor_count"]),
                            "review_retention": _ratio(counts["review_retained"], counts["review_expected"])}
            responses = [by_id[case["id"]] for case in cases if case["id"] in by_id]
            resources = _resources(responses)
            resources["scheduled_input_bytes"] = sum(len(_request(case, prefixes[case["prefix_id"]])["state"].encode()) for case in cases)
            actual = bool(responses) and len(responses) == len(cases) and all(row.get("evidence_class") == "recorded_inference" for row in responses)
            identity_ok = all(row.get("identity") == frozen["configuration"]["actual"] for row in responses)
            selected = metrics.classification_metrics(rows)
            stages = {component: metrics.classification_metrics([row for row in rows if row["component"] == component])
                      for component in _questions(use_case)}
            end_metrics = metrics.classification_metrics(end_to_end)
            generated_cases = [case for case in cases if case.get("claim_provenance") == "recorded_generated"]
            generated_valid = [case for case in generated_cases if _generated_claim_valid(case, prefixes[case["prefix_id"]])]
            gates = []
            supported_protocols = {"open_router_decisions", "open_router_system_one", "type_safe", "perplexity", "mercury", "local_decider", "pennel_cloud"}
            if frozen["configuration"]["configured"]["protocol"] not in supported_protocols:
                gates.append("product_route_unavailable")
            if not actual: gates.append("actual_inference_unmeasured")
            if not identity_ok: gates.append("actual_identity_mismatch")
            if any(row["calibration_status"] != "calibrated" for row in rows): gates.append("uncalibrated_cases")
            if actual and frozen["stage"] != "heldout_ready": gates.append("calibration_not_frozen_before_heldout")
            if actual and artifact.get("incremental_evidence_mode") != "recorded_inference": gates.append("calibration_inference_unmeasured")
            if use_case == "source_assessment":
                if any(stage["selective_precision"] is None or stage["selective_precision"] < criteria["source_selective_precision_min"] or stage["coverage"] < criteria["source_coverage_min"] for stage in stages.values()):
                    gates.append("source_quality_gate_failed")
                if len(generated_valid) < criteria["generated_claim_judgments_min"]:
                    gates.append("generated_claims_unmeasured")
            else:
                if selected["macro_f1"] is None or selected["macro_f1"] < criteria["events_macro_f1_min"]: gates.append("event_quality_gate_failed")
                if end_metrics["macro_f1"] is None or end_metrics["macro_f1"] < criteria["end_to_end_macro_f1_min"]: gates.append("complete_item_quality_gate_failed")
                if item_metrics["precision"] is None or item_metrics["precision"] < criteria["accepted_item_precision_min"]: gates.append("item_precision_gate_failed")
                if counts["owner_errors"] or counts["final_status_errors"]: gates.append("unsupported_owner_or_final_status")
                if counts["duplicate_suggestions"] or counts["unsupported_sources"]: gates.append("duplicate_or_unsupported_suggestion")
                if item_metrics["review_retention"] is not None and item_metrics["review_retention"] < 1: gates.append("reviewed_state_not_preserved")
            if (resources["warm_latency"]["samples"] < criteria["warm_samples_min"] or resources["cold_latency"]["samples"] < criteria["cold_samples_min"]
                    or not resources["peak_rss_bytes"] or not resources["cpu_time_ms"] or not resources["warm_latency"]["p95_ms"] or resources["executed_requests"] is None):
                gates.append("resource_measurements_unavailable")
            latency_budget = criteria["local_source_warm_p95_ms_max"] if use_case == "source_assessment" else criteria["local_event_warm_p95_ms_max"]
            if frozen["configuration"]["configured"]["protocol"] in ("local", "local_worker", "local_decider") and resources["warm_latency"]["p95_ms"] is not None and resources["warm_latency"]["p95_ms"] > latency_budget:
                gates.append("local_event_latency_budget_failed")
            if use_case != "manual_finished_suggestions" and not resources["foreground_impact_accepted"]:
                gates.append("foreground_impact_unmeasured")
            if use_case == "background_live_suggestions" and not resources["background_resource_accepted"]:
                gates.append("background_resources_unqualified")
            pending_judgments = sum(item.get("language") == language and item.get("split") == "heldout" for item in corpus.get("pending_generated_judgments", [])) if use_case == "source_assessment" else 0
            if use_case == "source_assessment" and pending_judgments:
                gates.append("generated_claim_judgments_pending")
            groups.append({"use_case": use_case, "language": language, "cases": len(cases), "classification": selected, "stages": stages,
                           "end_to_end_classification": end_metrics, "items": item_metrics, "resources": resources,
                           "actual_inference": actual, "identity_ok": identity_ok, "generated_claims": {"count": len(generated_valid), "unresolved": len(generated_cases) - len(generated_valid) + pending_judgments},
                           "gates": gates, "recommendation": "adopt" if not gates else "defer"})
    report = {"schema_version": 1, "protocol_id": "incremental-meeting-v1", "freeze_sha256": frozen["freeze_sha256"],
              "scheduled_cases": len(corpus["cases"]), "observed_cases": len(by_id), "missing_cases": len(corpus["cases"]) - len(by_id),
              "identity_mismatch_cases": sum(row.get("identity") != frozen["configuration"]["actual"] for row in by_id.values()),
              "calibration": artifact, "calibration_sha256": digest(artifact), "groups": groups,
              "unavailable_stages": ["candidate_recall", "reranking", "deadline_normalization", "natural_recording_asr_acceptance"],
              "adopted_records": [], "interpretation": "Component classification, complete source-quoted tuples, generated-claim support and native resource acceptance are separate gates. This replay never activates a model."}
    report["report_sha256"] = digest(report)
    report["qualification_records"] = [_qualification(group, frozen, artifact, report["report_sha256"]) for group in groups]
    return report


def _qualification(group, frozen, artifact, report_hash):
    context = {"use_case": group["use_case"], "language_policy": group["language"],
               "task": "meeting_source_assessment_v1" if group["use_case"] == "source_assessment" else "meeting_suggestions_v1",
               "configured": frozen["configuration"]["configured"], "actual": frozen["configuration"]["actual"],
               **{key: frozen[key] for key in ("prompt_sha256", "schema_sha256", "label_order_sha256", "preprocessing_sha256", "resource_profile_sha256")},
               "grouping": frozen["configuration"]["grouping"], "calibration_sha256": digest(artifact)}
    fitted = [fit for fit in artifact["fits"].values() if fit["context"].get("language") == group["language"] and str(fit["context"].get("subtask", "")).startswith(group["use_case"] + ":")]
    resources, classified, items = group["resources"], group["classification"], group["items"]
    stage_precision = [stage["selective_precision"] for stage in group["stages"].values()]
    stage_coverage = [stage["coverage"] for stage in group["stages"].values()]
    precision = min(stage_precision) if all(value is not None for value in stage_precision) else None
    coverage = min(stage_coverage) if all(value is not None for value in stage_coverage) else None
    record = {"schema_version": 1, "qualification_id": digest(context), "context": context,
              "evidence": {"mode": "recorded_inference" if group["actual_inference"] and artifact.get("incremental_evidence_mode") == "recorded_inference" and frozen["stage"] == "heldout_ready" else "offline_fixture",
                           **{key: frozen[key] for key in ("protocol_sha256", "corpus_sha256", "split_sha256")}, "report_sha256": report_hash,
                           "prefix_isolation": True, "heldout_disjoint": True, "source_judgments_complete": not group["generated_claims"]["unresolved"] and (group["use_case"] != "source_assessment" or group["generated_claims"]["count"] >= 10),
                           "failures_counted": True, "resource_measurements_complete": "resource_measurements_unavailable" not in group["gates"],
                           "foreground_impact_accepted": resources["foreground_impact_accepted"], "background_resource_accepted": resources["background_resource_accepted"]},
              "metrics": {"macro_f1": min(value for value in [classified["macro_f1"], group["end_to_end_classification"]["macro_f1"]] if value is not None) if classified["macro_f1"] is not None else None,
                          "accepted_item_precision": items["precision"], "correct_owner_and_final_status": not items.get("owner_errors", 0) and not items.get("final_status_errors", 0) if items.get("proposed_count") else None,
                          "selective_precision": precision, "coverage": coverage,
                          "warm_p95_ms": resources["warm_latency"]["p95_ms"], "peak_rss_bytes": resources["peak_rss_bytes"], "cpu_time_ms": resources["cpu_time_ms"],
                          "calibration_observations": min((fit["sample_count"] for fit in fitted), default=0),
                          "calibration_classes": min((fit["observed_class_count"] for fit in fitted), default=0),
                          "calibration_meeting_groups": min((fit["meeting_group_count"] for fit in fitted), default=0),
                          "heldout_observations": classified["count"]},
              "qualified": not group["gates"], "gates": group["gates"], "recommendation": group["recommendation"]}
    return record


def _read(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def _save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def verify_cached_models(model_ids, cache_root):
    import local_models
    results = []
    for model_id in model_ids:
        if model_id not in local_models.registry():
            raise ValueError("unknown_cached_model")
        status = local_models.model_status(model_id, cache_root)
        results.append({"model": model_id, "status": status["status"], "reason": status.get("reason"),
                        "artifact_bytes": status.get("artifact_bytes"), "identity": status.get("identity"),
                        "artifact_sha256": digest(status["identity"]) if status.get("identity") else None})
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="Replay deterministic owned fixtures; never run a model.")
    parser.add_argument("--output", type=Path, default=DIRECTORY.parents[1] / "build/incremental-evaluation")
    parser.add_argument("--configuration", type=Path)
    parser.add_argument("--corpus", type=Path, help="Explicit frozen imported corpus, including generated-claim review provenance when available.")
    parser.add_argument("--freeze", type=Path)
    parser.add_argument("--predictions", type=Path, help="Recorded JSONL, bound to this exact freeze and input hashes.")
    parser.add_argument("--requests", choices=("development", "calibration", "heldout"))
    parser.add_argument("--calibrate", action="store_true", help="Fit calibration-only predictions and freeze them before held-out export.")
    parser.add_argument("--verify-cached-model", action="append", default=[], help="Verify an existing registry artifact offline; never download or load it.")
    parser.add_argument("--cache", type=Path, default=DIRECTORY.parents[1] / "build/evaluation-models")
    args = parser.parse_args(argv)
    if args.verify_cached_model:
        status = verify_cached_models(args.verify_cached_model, args.cache)
        _save(args.output / "cached-artifacts.json", status)
        print(metrics.canonical_json({"models": [{"model": row["model"], "status": row["status"], "artifact_sha256": row["artifact_sha256"]} for row in status], "inference_executed": False, "downloads": 0}))
        return 0
    corpus = _read(args.corpus) if args.corpus else build_corpus()
    frozen = _read(args.freeze) if args.freeze else freeze(corpus, _read(args.configuration) if args.configuration else None)
    verify_freeze(corpus, frozen)
    args.output.mkdir(parents=True, exist_ok=True)
    if args.requests:
        output = args.output / (args.requests + "-requests.jsonl")
        with output.open("w", encoding="utf-8") as handle:
            for row in request_rows(corpus, frozen, args.requests):
                handle.write(metrics.canonical_json(row) + "\n")
        _save(args.output / "freeze.json", frozen)
        print(metrics.canonical_json({"requests": str(output), "freeze_sha256": frozen["freeze_sha256"], "inference_executed": False}))
        return 0
    predictions = [json.loads(line) for line in args.predictions.read_text(encoding="utf-8").splitlines() if line.strip()] if args.predictions else fixture_predictions(corpus, frozen) if args.offline else []
    if args.calibrate:
        frozen = calibrate(corpus, frozen, predictions)
        _save(args.output / "freeze.json", frozen)
        print(metrics.canonical_json({"stage": frozen["stage"], "freeze_sha256": frozen["freeze_sha256"], "inference_executed": False}))
        return 0
    report = evaluate(corpus, frozen, predictions)
    _save(args.output / "freeze.json", frozen)
    _save(args.output / "report.json", report)
    _save(args.output / "candidate-records.json", {"schema_version": 1, "candidate_records": report["qualification_records"], "adopted_records": []})
    if args.offline:
        with (args.output / "predictions.jsonl").open("w", encoding="utf-8") as handle:
            handle.writelines(metrics.canonical_json(row) + "\n" for row in predictions)
    print(metrics.canonical_json({"scheduled_cases": report["scheduled_cases"], "observed_cases": report["observed_cases"],
                                "candidate_records": len(report["qualification_records"]), "adopted_records": 0,
                                "report_sha256": report["report_sha256"], "inference_executed": False}))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, KeyError, TypeError) as error:
        print("Incremental evaluation refused: " + str(error), file=sys.stderr)
        raise SystemExit(2)
