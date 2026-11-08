"""Frozen production Suggestions exports, offline replay and an opt-in local pilot."""

import argparse
import base64
import hashlib
import ipaddress
import json
from pathlib import Path
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import metrics

DIRECTORY = Path(__file__).resolve().parent
PROTOCOL = DIRECTORY / "protocol-finished-production-v1.json"
MAX_RESPONSE_BYTES = 2 * 1024 * 1024


class FinishedError(ValueError):
    pass


class TransportFailure(FinishedError):
    def __init__(self, code, http_status=None, raw_response=b""):
        super().__init__(code)
        self.http_status = http_status
        self.raw_response = raw_response


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
                                     allow_nan=False).encode()).hexdigest()


def file_hash(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class CoreBridge:
    def __init__(self, binary):
        self.binary = str(Path(binary).resolve(strict=True))
        self.artifact_sha256 = file_hash(self.binary)

    def call_before(self, request, deadline):
        return self(request, deadline=deadline)

    def __call__(self, request, *, deadline=None):
        if deadline is not None and time.monotonic() >= deadline:
            raise FinishedError("core_bridge_timeout")
        if file_hash(self.binary) != self.artifact_sha256:
            raise FinishedError("bridge_binary_changed")
        encoded = json.dumps(request, ensure_ascii=False) + "\n"
        timeout = 30 if deadline is None else min(30, deadline - time.monotonic())
        if timeout <= 0:
            raise FinishedError("core_bridge_timeout")
        try:
            result = subprocess.run([self.binary], input=encoded, text=True, capture_output=True,
                                    timeout=timeout, check=False)
        except subprocess.TimeoutExpired:
            raise FinishedError("core_bridge_timeout") from None
        except OSError:
            raise FinishedError("core_bridge_unavailable") from None
        if result.returncode:
            raise FinishedError("core_bridge_failed")
        try:
            lines = result.stdout.splitlines()
            if len(lines) != 1:
                raise ValueError()
            value = json.loads(lines[0])
        except (ValueError, TypeError):
            raise FinishedError("core_bridge_invalid_output") from None
        if not isinstance(value, dict):
            raise FinishedError("core_bridge_invalid_output")
        if "error" in value:
            raise FinishedError(str(value.get("error", "core_bridge_invalid_output")))
        return value


def _bridge_call(bridge, request, *, deadline=None, clock=time.monotonic):
    if deadline is None:
        return bridge(request)
    if clock() >= deadline:
        raise FinishedError("core_bridge_timeout")
    call = getattr(bridge, "call_before", None)
    if not callable(call):
        raise FinishedError("deadline_capable_bridge_required")
    value = call(request, deadline)
    if clock() >= deadline:
        raise FinishedError("core_bridge_timeout")
    return value


def freeze(fixtures, configuration, bridge, expected_actual=None, expected_answers=None,
           *, deadline=None, clock=time.monotonic):
    if not isinstance(fixtures, list) or not fixtures:
        raise FinishedError("fixtures_required")
    families, ids, exports = {}, set(), []
    for fixture in fixtures:
        if not isinstance(fixture, dict) or fixture.get("language") not in ("en", "nl", "mixed"):
            raise FinishedError("invalid_fixture_language")
        split, family, identifier = fixture.get("split"), fixture.get("family_id"), fixture.get("id")
        if split not in ("development", "calibration", "heldout") or not isinstance(family, str) or not family:
            raise FinishedError("invalid_fixture_split")
        if not isinstance(identifier, str) or not identifier or identifier in ids:
            raise FinishedError("duplicate_or_missing_fixture_id")
        ids.add(identifier)
        if family in families and families[family] != split:
            raise FinishedError("family_split_leakage")
        families[family] = split
        exports.append(_bridge_call(bridge, {"op": "finished_meeting_export", "fixture": fixture,
                                            "configuration": configuration, "expected_actual": expected_actual},
                                    deadline=deadline, clock=clock))
    _validate_expected_answers(exports, expected_answers)
    value = {"schema_version": 1, "protocol_id": "production-finished-meeting-v1",
             "protocol_sha256": file_hash(PROTOCOL), "runner_sha256": file_hash(__file__),
             "bridge_sha256": bridge.artifact_sha256, "exports": exports,
             "expected_answers": expected_answers,
             "split_sha256": digest(sorted(families.items())), "production_admission": False}
    value["freeze_sha256"] = digest(value)
    return value


def verify_freeze(frozen, bridge, *, deadline=None, clock=time.monotonic):
    if (not isinstance(frozen, dict) or frozen.get("schema_version") != 1
            or frozen.get("protocol_id") != "production-finished-meeting-v1"
            or frozen.get("protocol_sha256") != file_hash(PROTOCOL)
            or frozen.get("runner_sha256") != file_hash(__file__)
            or frozen.get("bridge_sha256") != bridge.artifact_sha256
            or frozen.get("freeze_sha256") != digest({k: v for k, v in frozen.items() if k != "freeze_sha256"})
            or frozen.get("production_admission") is not False):
        raise FinishedError("production_freeze_changed")
    exports = frozen.get("exports")
    if not isinstance(exports, list) or not exports:
        raise FinishedError("production_freeze_changed")
    for exported in exports:
        rebuilt = _bridge_call(bridge, {"op": "finished_meeting_export", "fixture": exported["fixture"],
                                        "configuration": exported["configuration"],
                                        "expected_actual": exported.get("expected_actual")},
                               deadline=deadline, clock=clock)
        if rebuilt != exported:
            raise FinishedError("production_export_changed")
    rebuilt = freeze([value["fixture"] for value in exports], exports[0]["configuration"], bridge,
                     exports[0].get("expected_actual"), frozen.get("expected_answers"),
                     deadline=deadline, clock=clock)
    if rebuilt != frozen:
        raise FinishedError("production_freeze_changed")


def _validate_expected_answers(exports, expected):
    if expected is None:
        return
    if not isinstance(expected, dict) or set(expected) != {x["fixture"]["id"] for x in exports}:
        raise FinishedError("expected_answer_fixtures_changed")
    for exported in exports:
        rows = expected[exported["fixture"]["id"]]
        if not isinstance(rows, list) or len(rows) != len(exported["windows"]):
            raise FinishedError("expected_answer_windows_changed")
        for window, answers in zip(exported["windows"], rows):
            choices = {q["id"]: {o["id"] for o in q["options"]}
                       for q in window["request"]["questions"] if q["kind"] == "choice"}
            if not isinstance(answers, dict) or set(answers) != set(choices) or any(
                    answer not in choices[key] for key, answer in answers.items()):
                raise FinishedError("expected_answer_contract_changed")


def score_responses(frozen, responses, bridge):
    """Descriptive raw component scores; calibration/item judgments remain unqualified."""
    verify_freeze(frozen, bridge)
    expected = frozen.get("expected_answers")
    if expected is None:
        raise FinishedError("expected_answers_not_frozen")
    if not isinstance(responses, list) or len(responses) != len(frozen["exports"]):
        raise FinishedError("response_fixture_count_changed")
    languages = {key: [] for key in ("en", "nl", "mixed")}
    failures = {key: 0 for key in languages}
    for exported, raw_rows in zip(frozen["exports"], responses):
        replay = bridge({"op": "finished_meeting_replay", "export": exported, "responses": raw_rows})
        observed = {row["window_index"]: row for row in replay.get("windows", [])}
        language = exported["fixture"]["language"]
        for index, labels in enumerate(expected[exported["fixture"]["id"]]):
            row = observed.get(index, {})
            if row.get("status") != "ok":
                failures[language] += 1
            answers = {a["id"]: a for a in row.get("result", {}).get("answers", [])}
            for key, label in labels.items():
                answer = answers.get(key, {})
                valid = row.get("status") == "ok" and not answer.get("abstained", True)
                distribution = {p["option_id"]: p["probability"] for p in answer.get("probabilities", [])}
                languages[language].append({"expected": label, "prediction": answer.get("choice") if valid else None,
                                           "probabilities": distribution or None,
                                           "calibration_status": "uncalibrated"})
    return {"schema_version": 1, "protocol_id": frozen["protocol_id"],
            "freeze_sha256": frozen["freeze_sha256"], "qualification": False,
            "production_admission": False, "evidence_class": "raw_response_replay",
            "by_language": {language: {"raw_component_metrics": metrics.classification_metrics(rows),
                                       "failed_or_missing_windows": failures[language],
                                       "complete_item_metrics": None, "accepted_item_precision": None,
                                       "calibration_status": "uncalibrated"}
                            for language, rows in languages.items()},
            "unmet_qualification": ["calibration_unfrozen", "complete_item_judgments_unavailable",
                                    "resource_measurements_unavailable", "native_acceptance_unavailable"]}


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def _local_http_worker():
    """One killable request; stdout is one owned metadata line followed by raw bytes."""
    emitted_header = False
    response = None
    try:
        value = json.load(sys.stdin)
        endpoint = value["endpoint"]
        _loopback_endpoint(endpoint)
        body = base64.b64decode(value["body"], validate=True)
        timeout = value["timeout"]
        if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not 0 < timeout <= 30:
            raise FinishedError("invalid_transport_timeout")
        response = _open_local_response(endpoint, body, timeout)
        sys.stdout.buffer.write(json.dumps({"http_status": response.code}).encode() + b"\n")
        sys.stdout.buffer.flush()
        emitted_header = True
        remaining = MAX_RESPONSE_BYTES + 1
        while remaining > 0:
            chunk = response.read1(min(64 * 1024, remaining))
            if not chunk:
                break
            sys.stdout.buffer.write(chunk)
            sys.stdout.buffer.flush()
            remaining -= len(chunk)
        return 0
    except Exception:
        if not emitted_header:
            sys.stdout.buffer.write(b'{"error":"transport_failed"}\n')
            sys.stdout.buffer.flush()
        return 1
    finally:
        if response is not None:
            response.close()


def _open_local_response(endpoint, body, timeout):
    # Ignore environment proxies and never follow a local server's redirect off-device.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())
    request = urllib.request.Request(endpoint, data=body, headers={"Content-Type": "application/json"}, method="POST")
    try:
        return opener.open(request, timeout=timeout)
    except urllib.error.HTTPError as error:
        return error


def _loopback_endpoint(endpoint):
    try:
        parsed = urllib.parse.urlsplit(endpoint)
        address = ipaddress.ip_address(parsed.hostname or "")
        valid = (address.is_loopback and parsed.scheme == "http" and parsed.path == "/v1/decisions"
                 and parsed.port is not None and not parsed.username and not parsed.password
                 and not parsed.query and not parsed.fragment)
    except ValueError:
        valid = False
    if not valid:
        raise FinishedError("explicit_local_loopback_endpoint_required")
    return endpoint


def local_transport(endpoint, body, timeout, *, deadline=None):
    _loopback_endpoint(endpoint)
    deadline = min(deadline, time.monotonic() + timeout) if deadline is not None else time.monotonic() + timeout
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TransportFailure("transport_deadline_exceeded")
    payload = json.dumps({"endpoint": endpoint, "body": base64.b64encode(body).decode(),
                          "timeout": min(30, remaining)}).encode()
    process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "_local_http_worker"],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    expired = False
    try:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            expired = True
            process.kill()
            output, _ = process.communicate()
        else:
            output, _ = process.communicate(payload, timeout=remaining)
    except subprocess.TimeoutExpired:
        expired = True
        process.kill()
        output, _ = process.communicate()
    except BaseException:
        process.kill()
        process.communicate()
        raise
    header, separator, raw = output.partition(b"\n")
    status = None
    if separator and len(header) <= 1024:
        try:
            metadata = json.loads(header)
            status = metadata.get("http_status")
        except (ValueError, AttributeError):
            pass
    if len(raw) > MAX_RESPONSE_BYTES + 1:
        raise TransportFailure("transport_worker_invalid_output", status, raw[:MAX_RESPONSE_BYTES + 1])
    if expired or time.monotonic() >= deadline:
        raise TransportFailure("transport_deadline_exceeded", status, raw)
    if process.returncode or not isinstance(status, int):
        raise TransportFailure("transport_failed", status, raw)
    return status, raw


def _local_configuration(configuration):
    endpoint = _loopback_endpoint(configuration.get("endpoint", ""))
    if configuration.get("protocol") != "local_decider":
        raise FinishedError("explicit_local_loopback_endpoint_required")
    if configuration.get("model") != "pplx-decider-flash-v1.2":
        raise FinishedError("pilot_model_changed")
    if configuration.get("timeout_secs") != 30 or configuration.get("max_input_bytes") != 16000:
        raise FinishedError("pilot_limits_changed")
    return endpoint


def run_local_pilot(frozen, bridge, *, execute=False, transport=None, max_seconds=1800,
                    clock=time.monotonic, started_at=None):
    if not execute:
        raise FinishedError("local_execution_not_enabled")
    if isinstance(max_seconds, bool) or not isinstance(max_seconds, (int, float)) or not 0 < max_seconds <= 1800:
        raise FinishedError("invalid_evaluation_budget")
    started = clock() if started_at is None else started_at
    global_deadline = started + max_seconds
    report = {"schema_version": 1, "protocol_id": frozen.get("protocol_id"),
              "freeze_sha256": frozen.get("freeze_sha256"), "status": "blocked", "reason": None,
              "production_admission": False, "qualification": False, "hosted_spend_usd": 0,
              "physical_requests": 0, "requests": [], "fixtures": [],
              "runtime_cpu_time_ms": None, "runtime_peak_rss_bytes": None}
    try:
        verify_freeze(frozen, bridge, deadline=global_deadline, clock=clock)
    except Exception:
        if clock() >= global_deadline:
            report["reason"] = "evaluation_budget_exceeded"
            return _finish(report, started, clock)
        raise
    for exported in frozen["exports"]:
        _local_configuration(exported["configuration"])
        if exported["fixture"]["split"] != "development":
            raise FinishedError("pilot_is_development_only")
    if clock() >= global_deadline:
        report["reason"] = "evaluation_budget_exceeded"
        return _finish(report, started, clock)
    for exported in frozen["exports"]:
        job_started = clock()
        job_deadline = min(global_deadline, job_started + exported["configuration"]["timeout_secs"])
        replies = []
        fixture_report = {"id": exported["fixture"]["id"], "language": exported["fixture"]["language"],
                          "physical_requests": 0, "replay": None}
        report["fixtures"].append(fixture_report)
        # Re-preparation is inside the whole-job clock and cannot discard earlier attempts.
        try:
            rebuilt = _bridge_call(bridge, {"op": "finished_meeting_export", "fixture": exported["fixture"],
                                           "configuration": exported["configuration"],
                                           "expected_actual": exported.get("expected_actual")},
                                   deadline=job_deadline, clock=clock)
            if rebuilt != exported:
                raise FinishedError("production_export_changed")
        except Exception as error:
            report["reason"] = (_deadline_reason(clock(), job_deadline, global_deadline)
                                or (str(error) if isinstance(error, FinishedError) else "core_bridge_failed"))
            return _finish(report, started, clock)
        for index, window in enumerate(exported["windows"]):
            remaining = min(job_deadline, global_deadline) - clock()
            if remaining <= 0:
                report["reason"] = "evaluation_budget_exceeded" if clock() >= global_deadline else "job_deadline_exceeded"
                return _finish(report, started, clock)
            body = window["wire_body"].encode()
            entry = {"fixture_id": exported["fixture"]["id"], "window_index": index,
                     "request_sha256": hashlib.sha256(body).hexdigest(), "http_status": None,
                     "raw_response": None, "elapsed_ms": None}
            report["requests"].append(entry)
            report["physical_requests"] += 1
            fixture_report["physical_requests"] += 1
            request_started = clock()
            try:
                if transport is None:
                    status, raw = local_transport(exported["configuration"]["endpoint"], body, remaining,
                                                  deadline=min(job_deadline, global_deadline))
                else:
                    status, raw = transport(exported["configuration"]["endpoint"], body, remaining)
                raw_bytes = _response_entry(entry, status, raw, complete=True)
                if len(raw_bytes) > MAX_RESPONSE_BYTES:
                    raise FinishedError("response_too_large")
                if entry["raw_response"] is None:
                    raise FinishedError("response_not_utf8")
            except Exception as error:
                if isinstance(error, TransportFailure):
                    _response_entry(entry, error.http_status, error.raw_response, complete=False)
                entry["elapsed_ms"] = (clock() - request_started) * 1000
                report["reason"] = (_deadline_reason(clock(), job_deadline, global_deadline)
                                    or (str(error) if isinstance(error, FinishedError) else "transport_failed"))
                return _finish(report, started, clock)
            entry["elapsed_ms"] = (clock() - request_started) * 1000
            if clock() >= job_deadline or clock() >= global_deadline:
                report["reason"] = "evaluation_budget_exceeded" if clock() >= global_deadline else "job_deadline_exceeded"
                return _finish(report, started, clock)
            if not isinstance(status, int) or not 200 <= status < 300:
                report["reason"] = "provider_failed"
                return _finish(report, started, clock)
            replies.append({"window_index": index, "http_status": status, "body": entry["raw_response"]})
            try:
                replay = _bridge_call(bridge, {"op": "finished_meeting_replay", "export": exported, "responses": replies},
                                      deadline=min(job_deadline, global_deadline), clock=clock)
            except Exception as error:
                report["reason"] = (_deadline_reason(clock(), job_deadline, global_deadline)
                                    or (str(error) if isinstance(error, FinishedError) else "core_bridge_failed"))
                return _finish(report, started, clock)
            fixture_report["replay"] = replay
            deadline_reason = _deadline_reason(clock(), job_deadline, global_deadline)
            if deadline_reason:
                report["reason"] = deadline_reason
                return _finish(report, started, clock)
            if replay.get("status") != "ok":
                report["reason"] = replay.get("status", "invalid_replay")
                return _finish(report, started, clock)
        fixture_report["elapsed_ms"] = (clock() - job_started) * 1000
        deadline_reason = _deadline_reason(clock(), job_deadline, global_deadline)
        if deadline_reason:
            report["reason"] = deadline_reason
            return _finish(report, started, clock)
    if clock() >= global_deadline:
        report["reason"] = "evaluation_budget_exceeded"
        return _finish(report, started, clock)
    report["status"] = "compatible"
    report["reason"] = None
    return _finish(report, started, clock)


def _finish(report, started, clock):
    report["elapsed_ms"] = (clock() - started) * 1000
    return report


def _response_entry(entry, status, raw, *, complete):
    value = raw.encode() if isinstance(raw, str) else bytes(raw)
    entry.update(http_status=status, response_sha256=hashlib.sha256(value).hexdigest(),
                 response_bytes=len(value), raw_response_complete=complete)
    try:
        entry["raw_response"] = value.decode("utf-8")
    except UnicodeDecodeError:
        entry["raw_response"] = None
        entry["raw_response_base64"] = base64.b64encode(value).decode()
    return value


def _deadline_reason(now, job_deadline, global_deadline):
    if now >= global_deadline:
        return "evaluation_budget_exceeded"
    if now >= job_deadline:
        return "job_deadline_exceeded"
    return None


def main():
    started_at = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("export", "replay", "score", "pilot"))
    parser.add_argument("--bridge", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--fixtures")
    parser.add_argument("--configuration")
    parser.add_argument("--expected-actual")
    parser.add_argument("--expected-answers")
    parser.add_argument("--freeze")
    parser.add_argument("--responses")
    parser.add_argument("--execute-local", action="store_true")
    parser.add_argument("--max-seconds", type=float, default=1800)
    args = parser.parse_args()
    required = ("fixtures", "configuration") if args.operation == "export" else ("freeze",)
    if args.operation in ("replay", "score"):
        required += ("responses",)
    if any(getattr(args, key) is None for key in required):
        parser.error("missing required input for " + args.operation + ": " + ", ".join(required))
    bridge = CoreBridge(args.bridge)
    read = lambda path: json.loads(Path(path).read_text())
    if args.operation == "export":
        result = freeze(read(args.fixtures), read(args.configuration), bridge,
                        read(args.expected_actual) if args.expected_actual else None,
                        read(args.expected_answers) if args.expected_answers else None)
    else:
        frozen = read(args.freeze)
        if args.operation == "pilot":
            result = run_local_pilot(frozen, bridge, execute=args.execute_local, max_seconds=args.max_seconds,
                                     started_at=started_at)
        elif args.operation == "score":
            result = score_responses(frozen, read(args.responses), bridge)
        else:
            verify_freeze(frozen, bridge)
            responses = read(args.responses)
            if not isinstance(responses, list) or len(responses) != len(frozen["exports"]):
                raise FinishedError("response_fixture_count_changed")
            result = [bridge({"op":"finished_meeting_replay", "export":exported, "responses":rows})
                      for exported, rows in zip(frozen["exports"], responses)]
    Path(args.output).write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"output": str(Path(args.output)), "status": result.get("status", "exported") if isinstance(result, dict) else "replayed",
                      "production_admission": False}))
    return 2 if isinstance(result, dict) and result.get("status") == "blocked" else 0


if __name__ == "__main__":
    if sys.argv[1:] == ["_local_http_worker"]:
        raise SystemExit(_local_http_worker())
    try:
        raise SystemExit(main())
    except FinishedError as error:
        raise SystemExit(str(error)) from None
