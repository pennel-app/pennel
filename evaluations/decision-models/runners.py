"""Explicit, evaluation-only decision transports. Importing this module does no I/O."""

from __future__ import annotations

import hashlib
import copy
import ipaddress
import json
import math
import os
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation


class EvaluationError(Exception):
    """A content-free error code suitable for evaluation diagnostics."""


def _decimal(value, code="invalid_cost"):
    try:
        number = Decimal(str(value))
    except (InvalidOperation, ValueError, TypeError):
        raise EvaluationError(code) from None
    if not number.is_finite() or number < 0:
        raise EvaluationError(code)
    return number


class SpendLedger:
    """Reserve before sending; preserve known bills and conservative bounds separately."""

    def __init__(self, limit_usd="2.00", checkpoint=None):
        self.limit = _decimal(limit_usd)
        if self.limit > Decimal("2.00"):
            raise EvaluationError("budget_exceeds_authorized_usd_2")
        self.spent = Decimal(0)
        self.pending = {}
        self.pending_metadata = {}
        self.conservative_upper_bounds = {}
        self.unresolved = False
        self._hard_unresolved = False
        self._unknown_tokens = set()
        self.attempts = []
        self._next = 0
        self._lock = threading.Lock()
        self.checkpoint = checkpoint

    @property
    def conservative_spend(self):
        return sum((_decimal(row['upper_bound_usd']) for row in self.conservative_upper_bounds.values()), Decimal(0))

    @property
    def total_exposure(self):
        return self.spent + self.conservative_spend + sum(self.pending.values(), Decimal(0))

    def reserve(self, amount, metadata=None):
        amount = _decimal(amount)
        with self._lock:
            if self.unresolved:
                raise EvaluationError("unresolved_charge")
            if amount <= 0 or self.total_exposure + amount > self.limit:
                raise EvaluationError("budget_exhausted")
            self._next += 1
            token = str(self._next)
            self.pending[token] = amount
            if metadata is not None:
                if not isinstance(metadata, dict): raise EvaluationError('invalid_reservation_metadata')
                self.pending_metadata[token] = copy.deepcopy(metadata)
            if self.checkpoint:
                self.checkpoint(self.snapshot())
            return token

    def settle(self, token, cost, metadata=None):
        with self._lock:
            reserved = self.pending[token]
            actual = None if cost is None else _decimal(cost)
            previous = next((row for row in self.attempts if row.get('reservation_id') == token), None)
            if previous is not None and previous.get('billed_cost_usd') is not None:
                raise EvaluationError('reservation_already_settled')
            merged = copy.deepcopy(self.pending_metadata.get(token, {}))
            merged.update(metadata or {})
            if actual is None:
                self._unknown_tokens.add(token)
                self.unresolved = True
            else:
                self.pending.pop(token)
                self.pending_metadata.pop(token, None)
                self._unknown_tokens.discard(token)
                self.spent += actual
                if actual > reserved or self.total_exposure > self.limit:
                    self._hard_unresolved = True
                self.unresolved = self._hard_unresolved or bool(self._unknown_tokens)
            row = {"reservation_id": token, "reserved_usd": str(reserved),
                "billed_cost_usd": None if actual is None else str(actual), "metadata": merged}
            if previous is None: self.attempts.append(row)
            else: self.attempts[self.attempts.index(previous)] = row
            if self.checkpoint:
                self.checkpoint(self.snapshot())

    def resolve_upper_bound(self, token, metadata, evidence):
        """Explicitly account the FULL reserve, never invent an actual response bill.

        The bucket is permanent: no automatic refund/discount follows aggregate
        counter observations. Exactly one durable snapshot moves the exposure.
        """
        if not isinstance(metadata, dict) or not isinstance(evidence, dict) or not evidence:
            raise EvaluationError('explicit_upper_bound_evidence_required')
        with self._lock:
            existing = self.conservative_upper_bounds.get(token)
            if existing is not None:
                if any(existing['metadata'].get(key) != value for key, value in metadata.items()) or existing['evidence'] != evidence:
                    raise EvaluationError('conservative_resolution_already_recorded')
                return self.snapshot()
            if token not in self.pending:
                raise EvaluationError('unknown_pending_reservation')
            original_metadata = self.pending_metadata.get(token, {})
            previous = next((row for row in self.attempts if row.get('reservation_id') == token), None)
            if previous is not None:
                if previous.get('billed_cost_usd') is not None: raise EvaluationError('reservation_already_settled')
                original_metadata = {**original_metadata, **previous.get('metadata', {})}
            for key in original_metadata.keys() & metadata.keys():
                if original_metadata[key] != metadata[key]: raise EvaluationError('reservation_identity_changed')
            merged = copy.deepcopy(metadata);merged.update(copy.deepcopy(original_metadata))
            old = (copy.deepcopy(self.pending), copy.deepcopy(self.pending_metadata), copy.deepcopy(self.conservative_upper_bounds),
                   copy.deepcopy(self.attempts), self.unresolved, self._hard_unresolved, set(self._unknown_tokens))
            if self.unresolved and not self._unknown_tokens:
                self._hard_unresolved = True
            amount = self.pending.pop(token)
            self.pending_metadata.pop(token, None)
            self._unknown_tokens.discard(token)
            self.conservative_upper_bounds[token] = {'upper_bound_usd': str(amount), 'original_reserved_usd': str(amount),
                'metadata': merged, 'evidence': copy.deepcopy(evidence), 'raw_billed_cost_unreported': True}
            row = {'reservation_id': token, 'reserved_usd': str(amount), 'billed_cost_usd': None,
                'budget_upper_bound_usd': str(amount), 'resolution': 'full_reservation_upper_bound', 'metadata': merged}
            if previous is None: self.attempts.append(row)
            else: self.attempts[self.attempts.index(previous)] = row
            # Every other pending token remains unresolved after an interruption.
            self._unknown_tokens.update(self.pending)
            self.unresolved = self._hard_unresolved or bool(self._unknown_tokens) or self.total_exposure > self.limit
            try:
                if self.checkpoint: self.checkpoint(self.snapshot())
            except Exception:
                (self.pending, self.pending_metadata, self.conservative_upper_bounds, self.attempts,
                 self.unresolved, self._hard_unresolved, self._unknown_tokens) = old
                raise
            return self.snapshot()

    def snapshot(self):
        return {"limit_usd": str(self.limit), "known_billed_usd": str(self.spent),
            "conservative_upper_bound_usd": str(self.conservative_spend),
            "total_exposure_usd": str(self.total_exposure), "reserved_usd": str(sum(self.pending.values(), Decimal(0))),
            "pending_reservations": {key: str(value) for key, value in self.pending.items()},
            "pending_metadata": copy.deepcopy(self.pending_metadata),
            "conservative_upper_bounds": copy.deepcopy(self.conservative_upper_bounds),
            "unresolved_charge": self.unresolved, "hard_unresolved_charge": self._hard_unresolved,
            "unknown_reservation_tokens": sorted(self._unknown_tokens),
            "attempt_count": len(self.attempts), "attempts": copy.deepcopy(self.attempts)}

    def to_dict(self):
        return self.snapshot()

    @classmethod
    def from_dict(cls, state, checkpoint=None):
        if not isinstance(state, dict) or "limit_usd" not in state or "known_billed_usd" not in state:
            raise EvaluationError("invalid_persisted_ledger")
        ledger = cls(state["limit_usd"], checkpoint=checkpoint)
        ledger.spent = _decimal(state["known_billed_usd"])
        pending = state.get("pending_reservations", {})
        if not isinstance(pending, dict) or any(not isinstance(key, str) for key in pending):
            raise EvaluationError("invalid_persisted_ledger")
        ledger.pending = {key: _decimal(value) for key, value in pending.items()}
        if not ledger.pending and _decimal(state.get("reserved_usd", "0")):
            ledger.pending = {"interrupted": _decimal(state["reserved_usd"])}
        ledger.pending_metadata = copy.deepcopy(state.get('pending_metadata', {}))
        if not isinstance(ledger.pending_metadata, dict) or set(ledger.pending_metadata) - set(ledger.pending):
            raise EvaluationError('invalid_persisted_ledger')
        attempts = state.get("attempts", [])
        if not isinstance(attempts, list) or any(not isinstance(value, dict) for value in attempts):
            raise EvaluationError("invalid_persisted_ledger")
        ledger.attempts = copy.deepcopy(attempts)
        bounds = state.get('conservative_upper_bounds', {})
        if not isinstance(bounds, dict): raise EvaluationError('invalid_conservative_coverage')
        for token, row in bounds.items():
            if not isinstance(token, str) or not isinstance(row, dict) or token in ledger.pending:
                raise EvaluationError('invalid_conservative_coverage')
            amount = _decimal(row.get('upper_bound_usd'))
            original = _decimal(row.get('original_reserved_usd'))
            matches = [item for item in attempts if item.get('reservation_id') == token]
            if (amount <= 0 or amount != original or len(matches) != 1 or not isinstance(row.get('metadata'), dict)
                    or not isinstance(row.get('evidence'), dict) or not row['evidence'] or row.get('raw_billed_cost_unreported') is not True):
                raise EvaluationError('invalid_conservative_coverage')
            attempt = matches[0]
            if (attempt.get('billed_cost_usd') is not None or attempt.get('resolution') != 'full_reservation_upper_bound'
                    or _decimal(attempt.get('budget_upper_bound_usd')) != amount
                    or _decimal(attempt.get('reserved_usd')) != original or attempt.get('metadata') != row['metadata']):
                raise EvaluationError('invalid_conservative_coverage')
        if any(row.get('resolution') == 'full_reservation_upper_bound' and row.get('reservation_id') not in bounds for row in attempts):
            raise EvaluationError('dropped_conservative_coverage')
        ledger.conservative_upper_bounds = copy.deepcopy(bounds)
        declared = _decimal(state.get('conservative_upper_bound_usd', '0'))
        if declared != ledger.conservative_spend:
            raise EvaluationError('invalid_conservative_coverage')
        if 'total_exposure_usd' in state and _decimal(state['total_exposure_usd']) != ledger.total_exposure:
            raise EvaluationError('invalid_total_exposure')
        # Historical stop flags with no explicit cause stay hard stops; new unknown
        # tokens can be explicitly bounded without clearing unrelated failures.
        has_causes = 'hard_unresolved_charge' in state and 'unknown_reservation_tokens' in state
        unknown_tokens = state.get('unknown_reservation_tokens', [])
        if (not isinstance(unknown_tokens, list) or any(not isinstance(token, str) for token in unknown_tokens)
                or set(unknown_tokens) - set(ledger.pending)):
            raise EvaluationError('invalid_unknown_reservation_coverage')
        ledger._hard_unresolved = bool(state.get('hard_unresolved_charge')) if has_causes else bool(state.get('unresolved_charge'))
        if has_causes and state.get('unresolved_charge') and not unknown_tokens and not ledger.pending:
            ledger._hard_unresolved = True
        ledger._hard_unresolved |= ledger.total_exposure > ledger.limit
        ledger._hard_unresolved |= any(row.get('billed_cost_usd') is not None
            and _decimal(row['billed_cost_usd']) > _decimal(row.get('reserved_usd', '0')) for row in attempts)
        ledger._unknown_tokens = set(ledger.pending)
        ledger.unresolved = ledger._hard_unresolved or bool(ledger._unknown_tokens)
        identifiers = list(ledger.pending) + list(bounds) + [value.get("reservation_id", "0") for value in attempts]
        ledger._next = max((int(value) for value in identifiers if str(value).isdigit()), default=0)
        return ledger


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise EvaluationError("redirect_rejected")


def _unique_json(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise EvaluationError("duplicate_response_key")
        result[key] = value
    return result


class HttpTransport:
    is_mock = False

    def __init__(self, max_body_bytes=4 * 1024 * 1024):
        self.max_body_bytes = max_body_bytes
        self._thread = threading.local()

    def _thread_opener(self):
        if not hasattr(self._thread, "opener"):
            self._thread.opener = urllib.request.build_opener(_NoRedirect())
        return self._thread.opener

    def __call__(self, method, url, headers, payload, timeout):
        request = urllib.request.Request(
            url,
            data=None if payload is None else json.dumps(payload, ensure_ascii=False).encode(),
            headers=headers,
            method=method,
        )
        try:
            response = self._thread_opener().open(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            response = error
        except EvaluationError:
            raise
        except (TimeoutError, OSError):
            raise EvaluationError("transport_error_or_timeout") from None
        with response:
            body = response.read(self.max_body_bytes + 1)
            if len(body) > self.max_body_bytes:
                raise EvaluationError("response_too_large")
            try:
                parsed = json.loads(body, object_pairs_hook=_unique_json)
            except (ValueError, UnicodeError):
                raise EvaluationError("invalid_response_json") from None
            return {"status": response.status, "headers": dict(response.headers), "body": parsed}


def _probability(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise EvaluationError("invalid_probability")
    if not math.isfinite(value) or not 0 <= value <= 1:
        raise EvaluationError("invalid_probability")
    return float(value)


def _distribution(value, labels):
    if not isinstance(value, dict) or set(value) != set(labels):
        raise EvaluationError("probability_label_mismatch")
    probabilities = {label: _probability(value[label]) for label in labels}
    if abs(sum(probabilities.values()) - 1) > 1e-4:
        raise EvaluationError("probabilities_do_not_sum_to_one")
    return probabilities


def validate_answers(response, questions):
    """Keep provider score semantics; never turn relevance logits into probabilities."""
    answers = response.get("answers")
    if not isinstance(answers, dict) or set(answers) != set(questions):
        raise EvaluationError("question_id_mismatch")
    normalized = {}
    for identifier, question in questions.items():
        answer = answers[identifier]
        kind = question.get("type")
        if not isinstance(answer, dict) or answer.get("type") != kind:
            raise EvaluationError("answer_type_mismatch")
        confidence = answer.get("confidence")
        if confidence is not None:
            confidence = _probability(confidence)
        if kind == "noul":
            score = _probability(answer.get("noul"))
            probabilities = {"false": 1 - score, "true": score}
            prediction = "true" if score >= 0.5 else "false"
            score_type = "binary_probability"
        elif kind == "choice":
            criteria = question.get("criteria")
            if not isinstance(criteria, dict) or len(criteria) < 2:
                raise EvaluationError("invalid_choice_criteria")
            probabilities = _distribution(answer.get("probabilities"), list(criteria))
            prediction = answer.get("choice")
            if not isinstance(prediction, str) or prediction not in criteria:
                raise EvaluationError("unknown_answer_id")
            if probabilities[prediction] + 1e-6 < max(probabilities.values()):
                raise EvaluationError("choice_distribution_disagreement")
            score, score_type = probabilities[prediction], "categorical_probabilities"
        elif kind == "score":
            criteria = question.get("criteria")
            if not isinstance(criteria, list) or len(criteria) < 2:
                raise EvaluationError("invalid_ordinal_criteria")
            labels = [str(index) for index in range(len(criteria))]
            probabilities = _distribution(answer.get("probabilities"), labels)
            score = answer.get("score")
            if isinstance(score, bool) or not isinstance(score, (int, float)) or not math.isfinite(score):
                raise EvaluationError("invalid_ordinal_score")
            expected = sum(index * probabilities[str(index)] for index in range(len(criteria)))
            if not 0 <= score <= len(criteria) - 1 or abs(score - expected) > 1e-4:
                raise EvaluationError("ordinal_distribution_disagreement")
            legend = answer.get("legend")
            if legend is not None and legend != dict(zip(labels, criteria)):
                raise EvaluationError("ordinal_legend_mismatch")
            prediction = max(labels, key=lambda label: probabilities[label])
            score_type = "ordinal_probabilities"
        else:
            raise EvaluationError("unsupported_question_type")
        normalized[identifier] = {
            "prediction": prediction,
            "probabilities": probabilities,
            "raw_score": score,
            "raw_score_type": score_type,
            "confidence": confidence,
        }
    return normalized


DIRECT = {
    "jev": ("https://api.typesafe.ai/v1/systemone", "TYPESAFE_API_KEY"),
    "perplexity": ("https://api.perplexity.ai/v1/decisions", "PERPLEXITY_API_KEY"),
    "clef": (None, "CLOUDFLARE_API_TOKEN"),
    "mercury": ("https://api.inceptionlabs.ai/v1/decisions", "INCEPTION_API_KEY"),
}


def _loopback(hostname):
    try:
        return ipaddress.ip_address(hostname or "").is_loopback
    except ValueError:
        return hostname == "localhost"


class DecisionRunner:
    def __init__(self, config, transport=None, ledger=None, environ=None, cancel_event=None):
        self.config = dict(config)
        self.transport = transport or HttpTransport()
        self.environ = os.environ if environ is None else environ
        self.ledger = ledger or SpendLedger(config.get("max_cost_usd", "2.00"))
        self.kind = config.get("transport", "openrouter")
        self.model = config.get("model")
        self.timeout = config.get("timeout_seconds", 60)
        self._ready = None
        self._preflight_lock = threading.Lock()
        self.cancel_event = cancel_event
        self._headers = {"Content-Type": "application/json"}
        self._catalog = None
        self._endpoints = None
        self._url = None
        self._accepted_models = set()
        self._price = None
        self._pricing = None
        self._pricing_source = None
        self._limitations = []

    def _request(self, method, url, payload=None):
        response = self.transport(method, url, dict(self._headers), payload, self.timeout)
        if not isinstance(response, dict):
            raise EvaluationError("invalid_transport_response")
        if "body" not in response:
            response = {"status": 200, "headers": {}, "body": response}
        if not isinstance(response.get("body"), dict):
            raise EvaluationError("invalid_response_body")
        if not 200 <= response.get("status", 200) < 300:
            raise EvaluationError("http_" + str(response.get("status")))
        return response

    def preflight(self):
        with self._preflight_lock:
            return self._preflight()

    def _preflight(self):
        if self._ready is not None:
            return dict(self._ready)
        try:
            self._prepare()
            self._ready = {"status": "ok", "model": self.model, "transport": self.kind,
                           "catalog": self._catalog, "endpoints": self._endpoints,
                           "pricing": self._pricing, "pricing_source": self._pricing_source,
                           "limitations": list(self._limitations)}
        except EvaluationError as error:
            self._ready = {"status": "unavailable", "reason": str(error),
                           "model": self.model, "transport": self.kind}
        return dict(self._ready)

    def _prepare(self):
        if self.config.get("enabled") is not True:
            raise EvaluationError("live_evaluation_disabled")
        if not isinstance(self.model, str) or not self.model or any(
            value in self.model.lower() for value in ("latest", "router", "~", ":free")
        ):
            raise EvaluationError("model_must_be_explicit_and_versioned")
        if not isinstance(self.timeout, (int, float)) or not 0 < self.timeout <= 180:
            raise EvaluationError("invalid_timeout")
        state_limit = self.config.get("max_state_bytes")
        if state_limit is not None and (isinstance(state_limit, bool) or not isinstance(state_limit, int) or state_limit <= 0):
            raise EvaluationError("max_state_bytes_must_be_positive_integer")
        if self.config.get("retries", 0) != 0 or self.config.get("fallbacks"):
            raise EvaluationError("automatic_retries_or_fallbacks_disabled")
        if self.kind == "local_decider":
            self._url = self.config.get("endpoint", "http://127.0.0.1:8080/v1/decisions")
            parsed = urllib.parse.urlsplit(self._url)
            if parsed.scheme not in ("http", "https") or not _loopback(parsed.hostname):
                raise EvaluationError("local_endpoint_must_be_loopback")
            if parsed.username or parsed.password or parsed.query or parsed.fragment:
                raise EvaluationError("invalid_endpoint")
            self._accepted_models = {self.model}
            return
        privacy = self.config.get("privacy")
        if not isinstance(privacy, dict) or not privacy.get("policy_verified") or not privacy.get("policy_url"):
            raise EvaluationError("route_policy_not_verified")
        if privacy.get("processing_region") not in ("global", "eu", "us"):
            raise EvaluationError("processing_region_must_be_explicit")
        if privacy.get("data_collection", "allow") not in ("allow", "deny"):
            raise EvaluationError("invalid_retention_requirement")
        if self.kind == "openrouter":
            credential_env = self.config.get("credential_env", "OPENROUTER_API_KEY")
            if credential_env != "OPENROUTER_API_KEY":
                raise EvaluationError("openrouter_requires_explicit_openrouter_api_key")
            root = self.config.get("base_url", "https://openrouter.ai").rstrip("/")
            region = privacy["processing_region"]
            required = {"global": "openrouter.ai", "eu": "eu.openrouter.ai", "us": "us.openrouter.ai"}[region]
            if root != "https://" + required:
                raise EvaluationError("openrouter_region_host_mismatch")
            surface = self.config.get("api_surface", "decisions")
            paths = {"decisions": "/api/alpha/decisions", "systemone": "/api/v1/systemone"}
            if surface not in paths:
                raise EvaluationError("unsupported_openrouter_api_surface")
            self._url = root + paths[surface]
        elif self.kind == "direct":
            provider = self.config.get("provider")
            if provider not in DIRECT:
                raise EvaluationError("unsupported_direct_provider")
            default_url, default_env = DIRECT[provider]
            self._url = self.config.get("endpoint", default_url)
            credential_env = self.config.get("credential_env", default_env)
            if credential_env == "OPENROUTER_API_KEY":
                raise EvaluationError("openrouter_key_cannot_be_forwarded")
            parsed = urllib.parse.urlsplit(self._url or "")
            if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
                raise EvaluationError("remote_endpoint_requires_https")
            if parsed.query or parsed.fragment:
                raise EvaluationError("invalid_endpoint")
            self._accepted_models = set(self.config.get("accepted_models", [self.model]))
            self._price = _decimal(self.config.get("input_usd_per_million", "0")) / 1_000_000
            if self._price <= 0:
                raise EvaluationError("direct_billing_rate_required")
            self._pricing = {"prompt": str(self._price), "completion": "0"}
            self._pricing_source = "configured_direct_rate"
        else:
            raise EvaluationError("unsupported_transport")
        key = self.environ.get(credential_env)
        if not key:
            raise EvaluationError("evaluation_credential_unavailable")
        self._headers["Authorization"] = "Bearer " + key
        if self.kind == "openrouter":
            catalog = self._request("GET", root + "/api/v1/models?output_modalities=decisions")["body"]
            model = next((item for item in catalog.get("data", []) if item.get("id") == self.model), None)
            if model is None or "decisions" not in model.get("architecture", {}).get("output_modalities", []):
                raise EvaluationError("requested_decision_model_unavailable")
            self._catalog = model
            self._accepted_models = {self.model}
            if model.get("canonical_slug"):
                self._accepted_models.add(model["canonical_slug"])
            expected = self.config.get("expected_returned_model")
            if expected:
                if expected not in self._accepted_models:
                    raise EvaluationError("configured_checkpoint_catalog_mismatch")
                self._accepted_models = {expected}
            details = self._request("GET", root + "/api/v1/models/" + self.model + "/endpoints")["body"]
            self._endpoints = details.get("data", {}).get("endpoints", [])
            tag = self.config.get("upstream_tag", self.config.get("provider"))
            if not isinstance(tag, str) or not tag:
                raise EvaluationError("upstream_route_required")
            endpoint = next((item for item in self._endpoints if item.get("tag") == tag), None)
            if endpoint is None:
                raise EvaluationError("pinned_upstream_unavailable")
            self._endpoint = endpoint
            if endpoint.get("pricing") is None:
                self._pricing = model.get("pricing", {})
                self._pricing_source = "catalog_fallback"
                self._limitations.append("pinned_endpoint_pricing_unreported_catalog_rate_used")
            else:
                self._pricing = endpoint["pricing"]
                self._pricing_source = "pinned_endpoint"
            if not isinstance(self._pricing, dict):
                raise EvaluationError("invalid_endpoint_pricing")
            self._price = _decimal(self._pricing.get("prompt"))
            if _decimal(self._pricing.get("completion")) != 0:
                raise EvaluationError("nonzero_output_price_requires_explicit_billing_adapter")
            required_capabilities = self.config.get("capabilities", ["noul", "choice", "score"])
            if any(value not in ("noul", "choice", "score") for value in required_capabilities):
                raise EvaluationError("unsupported_decision_capability")

    def _questions(self, case):
        questions = case.get("questions") or {"decision": case.get("question")}
        if not isinstance(questions, dict) or not questions or any(not isinstance(q, dict) for q in questions.values()):
            raise EvaluationError("invalid_questions")
        if self.kind == "local_decider" and len(questions) != 1:
            raise EvaluationError("local_decider_requires_one_question")
        questions = {identifier: dict(question) for identifier, question in questions.items()}
        label_order = case.get("label_order")
        if label_order is not None and len(questions) == 1:
            question = next(iter(questions.values()))
            criteria = question.get("criteria")
            if isinstance(criteria, dict):
                if not isinstance(label_order, list) or len(label_order) != len(set(label_order)) or set(label_order) != set(criteria):
                    raise EvaluationError("label_order_mismatch")
                question["criteria"] = {identifier: criteria[identifier] for identifier in label_order}
        allowed = self.config.get("capabilities", ["noul", "choice", "score"])
        for identifier, question in questions.items():
            if not isinstance(identifier, str) or not identifier or question.get("type") not in allowed:
                raise EvaluationError("unsupported_decision_capability")
            if not isinstance(question.get("instructions"), str):
                raise EvaluationError("question_instructions_required")
            if question["type"] == "choice" and (not isinstance(question.get("criteria"), dict) or len(question["criteria"]) < 2):
                raise EvaluationError("invalid_choice_criteria")
            if question["type"] == "score" and (not isinstance(question.get("criteria"), list) or len(question["criteria"]) < 2):
                raise EvaluationError("invalid_ordinal_criteria")
        return questions

    def _payload(self, case, questions):
        if "state" not in case:
            raise EvaluationError("state_required")
        state_limit = self.config.get("max_state_bytes")
        if state_limit is not None and len(json.dumps(case["state"], ensure_ascii=False).encode("utf-8")) > state_limit:
            raise EvaluationError("state_byte_limit_exceeded_no_truncation")
        body = {"model": self.model, "state": case["state"], "questions": questions}
        if self.kind == "openrouter":
            privacy = self.config["privacy"]
            body["provider"] = {
                "only": [self._endpoint["tag"]],
                "allow_fallbacks": False,
                "data_collection": privacy.get("data_collection", "allow"),
            }
            if privacy.get("zdr") is True:
                body["provider"]["zdr"] = True
            if self._price:
                body["provider"]["max_price"] = {"prompt": float(self._price * 1_000_000), "completion": 0}
        encoded_bytes = len(json.dumps(body, ensure_ascii=False).encode())
        context = self.config.get("max_input_tokens")
        if self.kind == "openrouter":
            known = self._endpoint.get("context_length") or self._catalog.get("context_length")
            context = min(context, known) if context and known else context or known
        if not context:
            raise EvaluationError("context_limit_required")
        # Byte count is a conservative input bound, not a claimed tokenizer measurement.
        overhead = self.config.get("billing_overhead_tokens", 2048)
        if not isinstance(overhead, int) or overhead < 0:
            raise EvaluationError("invalid_billing_overhead")
        token_bound = encoded_bytes + overhead
        if token_bound > context:
            raise EvaluationError("context_bound_exceeded_no_truncation")
        return body, token_bound * len(questions)

    def predict(self, case):
        result = {"case_id": case.get("id"), "status": "unavailable", "prediction": None,
                  "probabilities": None, "attempts": [], "limitations": list(self._limitations),
                  "measurement": "mocked_transport" if getattr(self.transport, "is_mock", True) else "measured_inference",
                  "identity": {"transport": self.kind, "requested_model": self.model,
                               "api_surface": self.config.get("api_surface", "decisions")},
                  "cost_usd": None, "usage": None, "latency_ms": None}
        if self.cancel_event is not None and self.cancel_event.is_set():
            result["reason"] = "cancelled"
            return result
        availability = self.preflight()
        result["limitations"] = list(self._limitations)
        if availability["status"] != "ok":
            result["reason"] = availability["reason"]
            return result
        try:
            questions = self._questions(case)
            payload, token_bound = self._payload(case, questions)
            reservation = None
            if self.cancel_event is not None and self.cancel_event.is_set():
                raise EvaluationError("cancelled")
            if self.kind != "local_decider":
                estimate = self._price * token_bound
                cap = _decimal(self.config.get("max_request_cost_usd", "0.10"))
                if estimate > cap:
                    raise EvaluationError("request_reservation_exceeds_configured_cap")
                reservation = self.ledger.reserve(estimate)
        except EvaluationError as error:
            result["reason"] = str(error)
            return result
        if self.cancel_event is not None and self.cancel_event.is_set():
            if reservation is not None:
                self.ledger.settle(reservation, "0", {"case_id": case.get("id"), "request_sent": False,
                                                     "reason": "cancelled_before_send"})
            result["reason"] = "cancelled"
            return result
        started = datetime.now(timezone.utc).isoformat()
        start = time.perf_counter_ns()
        response = None
        failure = None
        try:
            response = self.transport("POST", self._url, dict(self._headers), payload, self.timeout)
            if not isinstance(response, dict):
                raise EvaluationError("invalid_transport_response")
            if "body" not in response:
                response = {"status": 200, "headers": {}, "body": response}
        except EvaluationError as error:
            failure = str(error)
        except (OSError, TimeoutError):
            failure = "transport_error_or_timeout"
        elapsed = (time.perf_counter_ns() - start) / 1_000_000
        body = response.get("body", {}) if response else {}
        if not isinstance(body, dict):
            body, failure = {}, "invalid_response_body"
        if self.kind == "direct" and self.config.get("provider") == "clef" and "result" in body:
            if body.get("success") is False:
                failure = "provider_error_response"
            inner = body.get("result")
            if isinstance(inner, dict):
                body = inner
            else:
                body, failure = {}, "invalid_cloudflare_result"
        usage = body.get("usage") if isinstance(body.get("usage"), dict) else {}
        cost = usage.get("cost")
        try:
            cost = None if cost is None else _decimal(cost)
        except EvaluationError:
            cost, failure = None, "invalid_billed_cost"
        request_id = body.get("id")
        headers = response.get("headers", {}) if response else {}
        actual_model, actual_provider = body.get("model"), body.get("provider")
        if actual_model is not None and not isinstance(actual_model, str):
            actual_model, failure = None, "returned_model_invalid_type"
        if actual_provider is not None and not isinstance(actual_provider, str):
            actual_provider, failure = None, "returned_provider_invalid_type"
        if request_id is not None and not isinstance(request_id, str):
            request_id, failure = None, "returned_request_id_invalid_type"
        route = payload.get("provider")
        identity = {"transport": self.kind, "requested_model": self.model, "returned_model": actual_model,
                    "provider": actual_provider, "request_id": request_id,
                    "configured_provider": self.config.get("upstream_tag", self.config.get("provider")),
                    "upstream_request_id": headers.get("x-request-id", headers.get("X-Request-Id")),
                    "endpoint": self._url, "api_surface": self.config.get("api_surface", "decisions"),
                    "routing": route, "privacy": self.config.get("privacy"),
                    "pricing": self._pricing, "pricing_source": self._pricing_source,
                    "question_grouping": self.config.get("question_grouping", "one_case_per_request"),
                    "question_ids": list(questions),
                    "label_order": {identifier: list(question.get("criteria", {})) for identifier, question in questions.items()},
                    "request_sha256": hashlib.sha256(json.dumps(payload, ensure_ascii=False).encode()).hexdigest(),
                    "catalog_sha256": None if self._catalog is None else hashlib.sha256(json.dumps(self._catalog, sort_keys=True).encode()).hexdigest(),
                    "endpoint_sha256": None if self._endpoints is None else hashlib.sha256(json.dumps(self._endpoint, sort_keys=True).encode()).hexdigest(),
                    "quantization": self._endpoint.get("quantization") if self.kind == "openrouter" else self.config.get("precision"),
                    "tokenizer": self._catalog.get("architecture", {}).get("tokenizer") if self._catalog else self.config.get("tokenizer")}
        attempt = {"started_at": started, "finished_at": datetime.now(timezone.utc).isoformat(),
                   "latency_ms": elapsed, "http_status": response.get("status", 200) if response else None,
                   "identity": identity, "usage": usage or None,
                   "billed_cost_usd": None if cost is None else str(cost),
                   "input_token_upper_bound": token_bound, "group_size": len(questions)}
        if reservation is not None:
            self.ledger.settle(reservation, cost, {"case_id": case.get("id"), "request_id": request_id})
        result.update(identity=identity, usage=usage or None, cost_usd=attempt["billed_cost_usd"],
                      latency_ms=elapsed, attempts=[attempt], status="error")
        if failure is None and response and not 200 <= response.get("status", 200) < 300:
            failure = "http_" + str(response.get("status"))
        if failure is None and "error" in body:
            failure = "provider_error_response"
        if failure is None and actual_model not in self._accepted_models:
            failure = "returned_model_mismatch" if actual_model else "returned_model_missing"
        if failure is None and self.kind == "openrouter":
            expected_provider = self._endpoint.get("provider_name")
            if actual_provider is None:
                result["limitations"].append("actual_provider_unreported")
            elif expected_provider and actual_provider != expected_provider:
                failure = "returned_provider_mismatch"
            if actual_model == self.model and self._catalog.get("canonical_slug") != self.model:
                result["limitations"].append("served_revision_unverifiable")
        for field in ("input_tokens", "output_tokens"):
            count = usage.get(field)
            if count is None:
                result["limitations"].append(field + "_unreported")
            elif isinstance(count, bool) or not isinstance(count, int) or count < 0:
                failure = "invalid_token_usage"
        if not request_id:
            result["limitations"].append("request_id_unreported")
        if cost is None and self.kind != "local_decider":
            result["limitations"].append("billed_cost_unknown_subsequent_paid_work_halted")
        if failure is None:
            try:
                answers = validate_answers(body, questions)
            except EvaluationError as error:
                failure = str(error)
            else:
                result["answers"] = answers
                result.update(answers[next(iter(questions))])
                result["status"] = "ok"
        if failure:
            result["reason"] = failure
        if failure in ("returned_model_mismatch", "returned_model_missing", "returned_provider_mismatch",
                       "returned_model_invalid_type", "returned_provider_invalid_type"):
            self._ready = {"status": "unavailable", "reason": "identity_drift_requires_revalidation",
                           "model": self.model, "transport": self.kind}
        result["reproducibility"] = "complete" if not result["limitations"] and not failure else "partial"
        return result


OpenRouterRunner = DecisionRunner
