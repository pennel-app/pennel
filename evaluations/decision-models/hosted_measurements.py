"""Serial OpenRouter timing panel, admitted only when its whole cost bound fits."""

from decimal import Decimal
import hashlib
import json
import time

from measurements import _cold_cases, _panel, _selected, _valid_number
from metrics import timing_metrics
from runners import DecisionRunner


def _configs(config):
    values = config if isinstance(config, list) else config.get("runners", [config])
    return [item for item in values if item.get("enabled") is True and item.get("transport") == "openrouter"]


def panel_manifest(config, corpus, ranking_cases):
    cases = _panel(corpus, ranking_cases)
    manifest = {"version": 1, "split": "heldout", "latency_concurrency": 1,
                "cold_client_samples_per_language": 5, "warm_samples_per_case": 30,
                "excluded_warmups_per_case": 1, "runners": [],
                "cases_sha256": hashlib.sha256(json.dumps(cases, sort_keys=True, ensure_ascii=False).encode()).hexdigest()}
    for item in _configs(config):
        subset = _selected(item, cases)
        manifest["runners"].append({"runner_id": item.get("id", item["model"]),
                                    "case_ids": [case["id"] for case in subset],
                                    "cold_case_ids": [case["id"] for case in _cold_cases(subset)]})
    manifest["panel_hash"] = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
    return manifest


def _request_cases(case, decision_rank):
    if case["task"] != "ranking":
        return [case]
    if not case.get("passages"):
        raise ValueError("empty_ranking_candidates")
    # Capture the actual ranking prompts; placeholder scores are discarded, never reported.
    class Collector:
        def __init__(self):
            self.cases = []
        def predict(self, request):
            self.cases.append(request)
            return {"status": "ok", "probabilities": {"true": 0.5}, "cost_usd": "0", "attempts": []}
    collector = Collector()
    decision_rank(collector, case)
    return collector.cases


def _budget(ledger):
    state = ledger.snapshot()
    if state.get("unresolved_charge") or Decimal(state.get("reserved_usd", "0")):
        return None
    return Decimal(state["limit_usd"]) - Decimal(state.get("total_exposure_usd", str(Decimal(state["known_billed_usd"]) + Decimal(state.get("conservative_upper_bound_usd", "0")))))


def measure_hosted(config, corpus, ranking_cases, ledger, runner_factory=DecisionRunner, cancel_event=None, checkpoint=None):
    from run import decision_rank
    manifest, cases = panel_manifest(config, corpus, ranking_cases), _panel(corpus, ranking_cases)
    report = {"panel": manifest, "status": "unmeasured", "models": [], "total_bound_usd": "0",
              "remaining_before_usd": None, "latency_concurrency": 1,
              "ledger": ledger.snapshot(),
              "limitations": ["Cold-client samples are first inference requests after fresh-client metadata preflight; not server/model-cold samples.",
                              "Provider server load time, memory and model/native artifact size are unmeasured."]}
    if cancel_event is not None and cancel_event.is_set():
        report["reason"] = "cancelled"
        return report
    if _budget(ledger) is None:
        report["reason"] = "unreconciled_budget"
        return report
    plans, total_bound = [], Decimal(0)
    for item in _configs(config):
        row = {"runner_id": item.get("id", item["model"]), "model": item["model"], "status": "unmeasured",
               "measurement": "measured_openrouter_end_to_end" if runner_factory is DecisionRunner else "mocked_runner",
               "server_load_ms": None, "server_memory_bytes": None, "download_bytes": None, "native_binary_size_bytes": None,
               "records": [], "cases": [], "client_setup_samples": []}
        report["models"].append(row)
        started = time.perf_counter()
        try:
            runner = runner_factory(item, ledger=ledger, cancel_event=cancel_event)
            availability = runner.preflight()
        except Exception as error:
            row.update(reason=type(error).__name__)
            continue
        row["planning_setup_ms"] = (time.perf_counter() - started) * 1000
        row["preflight"] = availability
        if availability.get("status") != "ok":
            row["reason"] = availability.get("reason", "runner_unavailable")
            continue
        subset, costs = _selected(item, cases), {}
        for case in subset:
            cell = {"case_id": case["id"], "task": case["task"], "language": case["language"],
                    "cold_client_samples": [], "warm_samples": [], "cold_client_outcomes": [], "warm_outcomes": [],
                    "client_setup_samples": []}
            row["cases"].append(cell)
            try:
                requests = _request_cases(case, decision_rank)
                bounds = [runner._payload(request, runner._questions(request))[1] * runner._price for request in requests]
                if any(bound > Decimal(str(item.get("max_request_cost_usd", "0.10"))) for bound in bounds):
                    raise ValueError("request_bound_exceeds_cap")
                costs[case["id"]] = sum(bounds, Decimal(0))
                cell["bound_per_invocation_usd"], cell["requests_per_invocation"] = str(costs[case["id"]]), len(requests)
            except Exception as error:
                cell["unmeasured_reason"] = str(error) if type(error).__name__ == "EvaluationError" else type(error).__name__
        cold_ids = {case["id"] for case in _cold_cases(subset)}
        bound = sum((cost * (31 + (5 if identifier in cold_ids else 0)) for identifier, cost in costs.items()), Decimal(0))
        row["bound_usd"] = str(bound)
        row["planned_paid_requests"] = sum(cell["requests_per_invocation"] * (31 + (5 if cell["case_id"] in cold_ids else 0))
                                            for cell in row["cases"] if cell["case_id"] in costs)
        total_bound += bound
        plans.append((item, runner, subset, cold_ids, costs, row))
    remaining = _budget(ledger)
    report.update(total_bound_usd=str(total_bound), remaining_before_usd=None if remaining is None else str(remaining))
    admitted = remaining is not None and total_bound <= remaining
    report["reason"] = "panel_fits_reconciled_budget" if admitted else "panel_bound_exceeds_remaining_budget"
    for item, runner, subset, cold_ids, costs, row in plans:
        cells = {cell["case_id"]: cell for cell in row["cases"]}
        stopped = not admitted or _budget(ledger) is None or (cancel_event is not None and cancel_event.is_set())

        def infer(client, case, phase):
            if _budget(ledger) is None or (cancel_event is not None and cancel_event.is_set()):
                return False
            try:
                result = decision_rank(client, case) if case["task"] == "ranking" else client.predict(case)
            except Exception as error:
                result = {"status": "error", "reason": type(error).__name__}
            record = {"case_id": case["id"], "task": case["task"], "language": case["language"], "phase": phase, **result}
            row["records"].append(record)
            status = result.get("status", "error")
            if status == "ok" and not _valid_number(result.get("latency_ms")):
                status, record["status"] = "error", "error"
                record["reason"] = "latency_unreported"
            if checkpoint is not None:
                checkpoint({"runner_id": row["runner_id"], **record})
            if phase != "warmup":
                cells[case["id"]][phase + "_outcomes"].append(status)
                if status == "ok":
                    cells[case["id"]][phase + "_samples"].append(result["latency_ms"])
            return status == "ok" and _budget(ledger) is not None

        for case in subset:
            if stopped or case["id"] not in cold_ids or case["id"] not in costs:
                continue
            for _ in range(5):
                if cancel_event is not None and cancel_event.is_set():
                    stopped = True
                    break
                started = time.perf_counter()
                try:
                    fresh = runner_factory(item, ledger=ledger, cancel_event=cancel_event)
                    status = fresh.preflight()
                    setup = (time.perf_counter() - started) * 1000
                    row["client_setup_samples"].append(setup)
                    cells[case["id"]]["client_setup_samples"].append(setup)
                    canonical = lambda value: (getattr(value, "_catalog", None) or {}).get("canonical_slug")
                    if status.get("status") != "ok" or fresh._price != runner._price or canonical(fresh) != canonical(runner):
                        row["reason"], stopped = "fresh_client_unavailable_or_route_changed", True
                        break
                    stopped = not infer(fresh, case, "cold_client")
                except Exception as error:
                    row["reason"], stopped = type(error).__name__, True
                if stopped:
                    break
        for case in subset:
            if stopped or case["id"] not in costs:
                continue
            if not infer(runner, case, "warmup"):
                stopped = True
                break
            for _ in range(30):
                if cancel_event is not None and cancel_event.is_set():
                    stopped = True
                    break
                if not infer(runner, case, "warm"):
                    stopped = True
                    break
        row["client_setup"] = timing_metrics(row.pop("client_setup_samples"))
        for cell in row["cases"]:
            cell["client_setup"] = timing_metrics(cell.pop("client_setup_samples"))
            for phase in ("cold_client", "warm"):
                cell[phase] = timing_metrics(cell.pop(phase + "_samples"))
                outcomes = cell.pop(phase + "_outcomes")
                cell[phase + "_status_counts"] = {status: outcomes.count(status) for status in sorted(set(outcomes))}
            if not admitted:
                cell["unmeasured_reason"] = report["reason"]
        row["status"] = "ok" if row["cases"] and all(cell["warm"]["n"] == 30 for cell in row["cases"]) and not stopped else "unmeasured_or_partial"
        row["known_billed_usd"] = str(sum((Decimal(record["cost_usd"]) for record in row["records"] if record.get("cost_usd") is not None), Decimal(0)))
        row["cost_complete"] = all(record.get("cost_usd") is not None for record in row["records"] if record.get("attempts"))
    for row in report["models"]:
        if "client_setup_samples" in row:
            row["client_setup"] = timing_metrics(row.pop("client_setup_samples"))
    report["remaining_after_usd"] = None if _budget(ledger) is None else str(_budget(ledger))
    report["planned_paid_requests"] = sum(row.get("planned_paid_requests", 0) for row in report["models"])
    report["ledger"] = ledger.snapshot()
    report["status"] = "ok" if report["models"] and all(row["status"] == "ok" for row in report["models"]) else "unmeasured_or_partial"
    return report
