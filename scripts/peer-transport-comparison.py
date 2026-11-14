#!/usr/bin/env python3
"""Compare isolated peer transports using synthetic, byte-verified fixtures.

This controller never opens a meeting library. Private identity, ticket and device
configuration files stay in the newly created output directory. Host measurements,
physical-device evidence and unsupported scenarios are reported separately.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import math
import os
from pathlib import Path
import re
import secrets
import selectors
import signal
import statistics
import subprocess
import sys
import threading
import time


MAX_CONTROL = 16 * 1024
MAX_RESULT = 2 * 1024 * 1024
BLOCK_SIZES = (128 * 1024, 256 * 1024, 1024 * 1024)
TAIL = b"Pennel qualification tail"
ID = re.compile(r"^[0-9a-f]{64}$")
RUN_ID = re.compile(r"^[A-Za-z0-9-]{1,64}$")


class QualificationError(RuntimeError):
    """A sanitized controller failure; never embeds peer output or capabilities."""


class ProbeOperationError(QualificationError):
    def __init__(self, code):
        allowed = {"transport", "unauthorized", "invalid_config", "closed", "invalid_manifest", "source_changed",
                   "invalid_block", "storage", "cancelled", "invalid_protocol", "limit_exceeded"}
        self.code = code if isinstance(code, str) and code in allowed else "unknown_probe_error"
        super().__init__(self.code)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_private_json(path, value, *, replace=False):
    path = Path(path)
    temporary = path.with_name(path.name + ".pending-" + secrets.token_hex(8))
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        with os.fdopen(os.open(temporary, flags, 0o600), "w") as output:
            json.dump(value, output, indent=2, allow_nan=False)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        if replace:
            os.replace(temporary, path)
        else:
            os.link(temporary, path)
        descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    finally:
        temporary.unlink(missing_ok=True)


def new_directory(path):
    path = Path(path).resolve()
    repository = Path(__file__).resolve().parents[1]
    if path.is_relative_to(repository):
        ignored = subprocess.run(["git", "check-ignore", "--quiet", "--", str(path.relative_to(repository))],
                                 cwd=repository, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if ignored.returncode != 0:
            raise QualificationError("evidence_directory_must_be_git_ignored")
    path.mkdir(mode=0o700, parents=True, exist_ok=False)
    os.chmod(path, 0o700)
    return path


def make_fixtures(root, *, mib=8):
    if type(mib) is not int or not 1 <= mib <= 256:
        raise QualificationError("fixture_size_out_of_range")
    root = new_directory(root)
    binary = root / "synthetic.bin"
    with binary.open("xb") as stream:
        for index in range(mib):
            stream.write(bytes([index % 251]) * (1024 * 1024))
        stream.write(TAIL)
        stream.flush()
        os.fsync(stream.fileno())
    fixtures = {"binary": binary, "empty": root / "empty.bin",
                "one_byte": root / "one-byte.bin", "note": root / "note.txt"}
    fixtures["empty"].write_bytes(b"")
    fixtures["one_byte"].write_bytes(b"x")
    fixtures["note"].write_bytes(b"Synthetic project note.\nNo meeting content.\n")
    for path in fixtures.values():
        path.chmod(0o600)
    return fixtures


def nonnegative_int(value):
    return type(value) is int and value >= 0


def validate_transfer(report, source, target, *, expected_bytes=None, complete=True):
    fields = {"complete", "bytes_sent", "bytes_received", "blocks_transferred", "total_bytes", "sha256"}
    if not isinstance(report, dict) or set(report) != fields or report.get("complete") is not complete:
        raise QualificationError("incorrect_completion_receipt")
    for field in ("bytes_sent", "bytes_received", "blocks_transferred", "total_bytes"):
        if not nonnegative_int(report.get(field)):
            raise QualificationError("incorrect_transfer_counts")
    if report["total_bytes"] != Path(source).stat().st_size or report.get("sha256") != sha256(source):
        raise QualificationError("incorrect_fixture_receipt")
    if expected_bytes is not None and report["bytes_received"] != expected_bytes:
        raise QualificationError("unexpected_content_bytes")
    if complete:
        if not Path(target).is_file() or sha256(target) != report["sha256"]:
            raise QualificationError("destination_not_verified")
    elif Path(target).exists():
        raise QualificationError("incomplete_file_was_published")


def validate_device_metrics(value):
    fields = {"scope", "method", "idle_rss_bytes", "peak_rss_bytes", "cpu_seconds", "sample_count", "elapsed_seconds"}
    if not isinstance(value, dict) or set(value) != fields:
        raise QualificationError("invalid_device_metrics")
    if value["scope"] != "owned_app_process" or value["method"] != "mach_task_basic_info+getrusage":
        raise QualificationError("incorrect_device_metrics_scope")
    for field in ("idle_rss_bytes", "peak_rss_bytes", "sample_count"):
        if not nonnegative_int(value[field]) or value[field] == 0:
            raise QualificationError("invalid_device_metric_counts")
    if value["peak_rss_bytes"] < value["idle_rss_bytes"]:
        raise QualificationError("invalid_device_metric_memory")
    for field in ("cpu_seconds", "elapsed_seconds"):
        number = value[field]
        try:
            finite = type(number) in (int, float) and math.isfinite(number)
        except OverflowError:
            finite = False
        if not finite or number < 0 or field == "elapsed_seconds" and number == 0:
            raise QualificationError("invalid_device_metric_timing")
    return {key: value[key] for key in fields}


def public_summary(value):
    # Construct the public projection rather than trying to redact arbitrary logs.
    allowed = {"schema_version", "complete", "host_cases_complete", "qualification_complete", "candidate", "route", "results", "gaps",
               "fixture_sha256", "fixture_bytes", "source_revision", "created_at_utc", "controller_sha256", "candidate_binary_sha256",
               "scope", "case", "fixture", "block_size", "repeat", "direction", "elapsed_seconds", "timing_scope", "receipt",
               "bytes_sent", "bytes_received", "blocks_transferred", "total_bytes", "sha256", "unchanged_content_bytes", "metrics",
               "endpoints", "relay", "network_bytes", "network_bytes_reason", "relay_forwarded_bytes", "sample_count", "peak_rss_bytes",
               "sampled_cpu_seconds", "method", "first_content_bytes", "resumed_content_bytes", "file_published", "quinn", "native",
               "relay_binary_sha256", "durable_content_bytes_before_restart", "opaque_forwarded_bytes_after_checkpoint",
               "next_block_durable_at_kill", "receiver_exit_code", "idle_rss_bytes", "cpu_seconds",
               "first_bytes", "resumed_bytes", "unchanged_bytes", "returned_bytes"}
    def project(item):
        if isinstance(item, dict):
            if item.get("scope") == "owned_app_process":
                try:
                    return validate_device_metrics(item)
                except QualificationError:
                    return {}
            return {key: project(child) for key, child in item.items() if key in allowed}
        if isinstance(item, list):
            return [project(child) for child in item]
        return item
    return project(value)


def _cpu_seconds(value):
    days, _, rest = value.partition("-")
    if not rest:
        rest, days = days, "0"
    parts = rest.split(":")
    if len(parts) not in (2, 3):
        raise ValueError("invalid ps cpu time")
    seconds = float(parts[-1]) + int(parts[-2]) * 60
    if len(parts) == 3:
        seconds += int(parts[0]) * 3600
    return seconds + int(days) * 86400


class Metrics:
    def __init__(self, pid):
        self.pid = pid
        self.lock = threading.Lock()
        self.samples = 0
        self.peak_rss = 0
        self.cpu = None
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _run(self):
        while not self.stop.is_set():
            try:
                result = subprocess.run(["ps", "-o", "rss=,time=", "-p", str(self.pid)],
                                        capture_output=True, text=True, timeout=2)
                parts = result.stdout.split()
                if len(parts) == 2:
                    rss, cpu = int(parts[0]) * 1024, _cpu_seconds(parts[1])
                    with self.lock:
                        self.samples += 1
                        self.peak_rss = max(self.peak_rss, rss)
                        self.cpu = cpu
            except (OSError, ValueError, subprocess.TimeoutExpired):
                pass
            self.stop.wait(0.1)

    def snapshot(self):
        with self.lock:
            return {"sample_count": self.samples, "peak_rss_bytes": self.peak_rss if self.samples else None,
                    "sampled_cpu_seconds": self.cpu,
                    "scope": "owned_process_lifetime_only", "method": "ps_100ms_samples"}

    def close(self):
        self.stop.set()
        self.thread.join(timeout=3)


class JsonProcess:
    def __init__(self, command, stderr_path):
        self.binary_sha256 = sha256(command[0])
        self.stderr_path = Path(stderr_path)
        descriptor = os.open(self.stderr_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        self.stderr = os.fdopen(descriptor, "wb")
        try:
            self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=self.stderr, bufsize=0)
        except BaseException:
            self.stderr.close()
            raise
        self.buffer = bytearray()
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.metrics = Metrics(self.process.pid)
        self.command_lock = threading.Lock()

    def read(self, timeout=90, require_ok=False):
        deadline = time.monotonic() + timeout
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise QualificationError("probe_timeout")
            data = os.read(self.process.stdout.fileno(), min(64 * 1024, MAX_RESULT + 1 - len(self.buffer)))
            if not data:
                raise QualificationError("probe_closed_without_result")
            self.buffer.extend(data)
            if len(self.buffer) > MAX_RESULT:
                raise QualificationError("probe_response_limit")
        line, _, remaining = self.buffer.partition(b"\n")
        self.buffer = bytearray(remaining)
        try:
            record = json.loads(line, parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
        except (UnicodeError, ValueError, TypeError):
            raise QualificationError("probe_response_invalid") from None
        if not isinstance(record, dict) or require_ok and record.get("ok") is not True:
            raise QualificationError("probe_operation_failed")
        return record

    def call(self, command, **arguments):
        with self.command_lock:
            if self.process.poll() is not None:
                raise QualificationError("probe_not_running")
            encoded = (json.dumps({"command": command, **arguments}, allow_nan=False) + "\n").encode()
            if len(encoded) > MAX_RESULT:
                raise QualificationError("probe_request_limit")
            try:
                self.process.stdin.write(encoded)
                self.process.stdin.flush()
            except (OSError, ValueError):
                raise QualificationError("probe_write_failed") from None
            response = self.read()
            if response.get("ok") is not True:
                raise ProbeOperationError(response.get("error"))
            return response.get("result")

    def close(self, graceful=True):
        if self.process.poll() is None:
            if graceful and self.command_lock.acquire(blocking=False):
                try:
                    self.process.stdin.write(b'{"command":"close"}\n')
                    self.process.stdin.flush()
                    self.process.wait(timeout=3)
                except (OSError, ValueError, subprocess.TimeoutExpired):
                    pass
                finally:
                    self.command_lock.release()
            if self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=3)
        self.metrics.close()
        self.selector.close()
        for stream in (self.process.stdin, self.process.stdout, self.stderr):
            stream.close()


class Probe(JsonProcess):
    def __init__(self, binary, candidate, identity_file, run_id, stderr_path,
                 *, relay_config=None, bind="127.0.0.1:0", start=True):
        self.candidate = candidate
        self.route = "forced_relay" if relay_config else "direct"
        self.identity_id = None
        command = [str(binary), "--candidate", candidate, "--identity-file", str(identity_file),
                   "--run-id", run_id, "--bind", bind]
        if relay_config:
            command += ["--relay-config", str(relay_config)]
        super().__init__(command, stderr_path)
        if start:
            try:
                self.ready()
            except BaseException:
                self.close(graceful=False)
                raise

    def ready(self):
        ready = self.read(require_ok=True)
        result = ready.get("result")
        if ready.get("event") != "ready" or not isinstance(result, dict):
            raise QualificationError("invalid_probe_startup")
        identity = result.get("identity_id")
        if not isinstance(identity, str) or not ID.fullmatch(identity):
            raise QualificationError("invalid_probe_identity")
        if result.get("candidate") != self.candidate or result.get("route") != self.route:
            raise QualificationError("incorrect_probe_route")
        self.identity_id = identity
        self.address_json = normalize_address(result.get("address_json"), identity)
        return result


def normalize_address(value, expected_identity):
    try:
        address = json.loads(value) if isinstance(value, str) else value
    except ValueError:
        raise QualificationError("invalid_probe_address") from None
    if not isinstance(address, dict) or address.get("identity_id") != expected_identity:
        raise QualificationError("incorrect_address_identity")
    encoded = json.dumps(address, separators=(",", ":"), allow_nan=False)
    if len(encoded.encode()) > MAX_CONTROL:
        raise QualificationError("address_limit")
    return encoded


class Relay(JsonProcess):
    def __init__(self, binary, root, candidate, identities, run_id, *, bind="127.0.0.1", rate=0):
        command = [str(binary), "--bind", f"{bind}:0", "--advertise-host", bind,
                   "--allow", identities[0], "--allow", identities[1], "--candidate", candidate,
                   "--run-id", run_id]
        if rate:
            command += ["--rate-bytes-per-second", str(rate)]
        super().__init__(command, Path(root) / "relay.stderr.log")
        try:
            ready = self.read(timeout=30)
            if ready.get("event") != "ready" or ready.get("candidate") != candidate:
                raise QualificationError("invalid_relay_startup")
            clients = ready.get("clients")
            if not isinstance(clients, dict) or set(clients) != set(identities) or ready.get("payload_storage") is not False:
                raise QualificationError("invalid_relay_clients")
            self.configs = []
            for index, identity in enumerate(identities):
                config = clients[identity]
                if not isinstance(config, dict) or not all(config.get(key) for key in ("host", "port", "server_cert_der_b64", "ticket")):
                    raise QualificationError("invalid_relay_config")
                file = Path(root) / f"relay-client-{index}.json"
                write_private_json(file, config)
                self.configs.append(file)
            write_private_json(Path(root) / "relay-private-ready.json", ready)
        except BaseException:
            self.close(graceful=False)
            raise

    def stats(self):
        with self.command_lock:
            self.process.stdin.write(b'{"command":"stats"}\n')
            self.process.stdin.flush()
            response = self.read(timeout=5)
        if response.get("event") != "stats" or not isinstance(response.get("result"), dict):
            raise QualificationError("invalid_relay_statistics")
        result = response["result"]
        if not nonnegative_int(result.get("forwarded_bytes")):
            raise QualificationError("invalid_relay_statistics")
        return result

    def set_paused(self, paused):
        if type(paused) is not bool or self.process.poll() is not None:
            raise QualificationError("relay_pause_unavailable")
        command, event = ("pause", "paused") if paused else ("resume", "resumed")
        with self.command_lock:
            self.process.stdin.write((json.dumps({"command": command}) + "\n").encode())
            self.process.stdin.flush()
            response = self.read(timeout=5)
        result = response.get("result")
        if (response.get("event") != event or not isinstance(result, dict)
                or set(result) != {"paused"} or result["paused"] is not paused):
            raise QualificationError("invalid_relay_pause_receipt")
        return {"paused": paused}

    def close(self, graceful=True):
        # Relay uses shutdown, while probes use close; termination is also bounded.
        if graceful and self.process.poll() is None:
            try:
                self.process.stdin.write(b'{"command":"shutdown"}\n')
                self.process.stdin.flush()
                self.process.wait(timeout=3)
            except (OSError, ValueError, subprocess.TimeoutExpired):
                pass
        super().close(graceful=False)


class HostSession:
    def __init__(self, binary, candidate, route, root, run_id, relay_binary=None, *, relay_rate=0):
        self.binary, self.candidate, self.route = binary, candidate, route
        self.root = new_directory(root)
        self.run_id, self.relay_binary, self.relay_rate = run_id, relay_binary, relay_rate
        self.identities = [self.root / "identity-a.json", self.root / "identity-b.json"]
        self.probes, self.relay, self.generation = [], None, 0
        self.identity_ids = None
        try:
            self._start("direct")
            self.identity_ids = [probe.identity_id for probe in self.probes]
            if route == "forced_relay":
                self.restart()
        except BaseException:
            self.close()
            raise

    def _start(self, route):
        self.generation += 1
        generation = new_directory(self.root / f"session-{self.generation}")
        configs = [None, None]
        if route == "forced_relay":
            if not self.relay_binary or not self.identity_ids:
                raise QualificationError("relay_not_configured")
            self.relay = Relay(self.relay_binary, generation, self.candidate, self.identity_ids,
                               self.run_id, rate=self.relay_rate)
            configs = self.relay.configs
        # Both children must exist before either relay-ready barrier is awaited.
        for index in range(2):
            self.probes.append(Probe(self.binary, self.candidate, self.identities[index],
                f"peer-probe-{self.run_id}-{index}", generation / f"probe-{index}.stderr.log",
                relay_config=configs[index], start=False))
        for index, probe in enumerate(self.probes):
            probe.ready()
            if self.identity_ids and probe.identity_id != self.identity_ids[index]:
                raise QualificationError("identity_changed_on_restart")
        for index, probe in enumerate(self.probes):
            probe.call("approve", peer_id=self.probes[1 - index].identity_id)

    def restart(self):
        self.close()
        self._start(self.route)

    def close(self):
        for probe in self.probes:
            probe.close()
        self.probes = []
        if self.relay:
            self.relay.close()
            self.relay = None

    def offer(self, sender, source, block_size):
        manifest = self.probes[sender].call("offer", path=str(source), block_size=block_size)
        if isinstance(manifest, str):
            manifest = json.loads(manifest)
        if not isinstance(manifest, dict) or manifest.get("sha256") != sha256(source):
            raise QualificationError("offer_does_not_match_fixture")
        address = self.probes[sender].call("address")
        return normalize_address(address, self.probes[sender].identity_id)

    def receive(self, receiver, address, stage, target, max_blocks=None):
        return self.probes[receiver].call("receive", address_json=address, staging=str(stage),
                                          target=str(target), max_blocks=max_blocks)

    def statistics(self):
        result = {"endpoints": [probe.metrics.snapshot() for probe in self.probes],
                  "network_bytes": None, "network_bytes_reason": "transport_has_no_wire_counter"}
        if self.relay:
            result["relay"] = self.relay.metrics.snapshot()
            result["relay_binary_sha256"] = self.relay.binary_sha256
            result["relay_forwarded_bytes"] = self.relay.stats().get("forwarded_bytes")
        return result


def host_case(session, source, root, block_size, repeat, sender, fixture_name):
    root = new_directory(root)
    receiver = 1 - sender
    address = session.offer(sender, source, block_size)
    target, stage = root / "received.bin", root / "partial"
    began = time.monotonic()
    report = session.receive(receiver, address, stage, target)
    elapsed = time.monotonic() - began
    validate_transfer(report, source, target, expected_bytes=source.stat().st_size)
    unchanged = session.receive(receiver, address, stage, target)
    validate_transfer(unchanged, source, target, expected_bytes=0)
    return {"case": "transfer", "fixture": fixture_name, "block_size": block_size,
            "repeat": repeat, "direction": "a_to_b" if sender == 0 else "b_to_a",
            "complete": True, "elapsed_seconds": elapsed, "receipt": report,
            "timing_scope": "receive_request_to_verified_durable_ack",
            "unchanged_content_bytes": unchanged["bytes_received"], "metrics": session.statistics()}


def recovery_case(session, source, root, block_size, *, corrupt=False, sender=0):
    root = new_directory(root)
    target, stage = root / "received.bin", root / "partial"
    receiver = 1 - sender
    address = session.offer(sender, source, block_size)
    first = session.receive(receiver, address, stage, target, max_blocks=1)
    validate_transfer(first, source, target, expected_bytes=block_size, complete=False)
    session.restart()
    if corrupt:
        block = stage / "block-00000000.part"
        if not block.is_file():
            raise QualificationError("durable_block_not_found")
        with block.open("r+b") as stream:
            first_byte = stream.read(1)
            if not first_byte:
                raise QualificationError("durable_block_empty")
            stream.seek(0)
            stream.write(bytes([first_byte[0] ^ 0xff]))
            stream.flush()
            os.fsync(stream.fileno())
    address = session.offer(sender, source, block_size)
    resumed = session.receive(receiver, address, stage, target)
    expected = source.stat().st_size - first["bytes_received"] + (block_size if corrupt else 0)
    validate_transfer(resumed, source, target, expected_bytes=expected)
    repeated = session.receive(receiver, address, stage, target)
    validate_transfer(repeated, source, target, expected_bytes=0)
    return {"case": "corrupt_durable_block" if corrupt else "process_restart", "complete": True,
            "direction": "a_to_b" if sender == 0 else "b_to_a",
            "block_size": block_size, "first_content_bytes": first["bytes_received"],
            "resumed_content_bytes": resumed["bytes_received"], "unchanged_content_bytes": repeated["bytes_received"]}


def authorization_case(session, source, root, block_size, *, sender=0):
    root = new_directory(root)
    receiver = 1 - sender
    address = session.offer(sender, source, block_size)
    session.probes[receiver].call("revoke", peer_id=session.probes[sender].identity_id)
    try:
        session.receive(receiver, address, root / "partial", root / "received.bin")
    except ProbeOperationError as error:
        if error.code != "unauthorized":
            raise QualificationError("revocation_not_proven") from None
        if (root / "received.bin").exists():
            raise QualificationError("revoked_peer_published_file")
        return {"case": "revoked_peer", "complete": True, "file_published": False,
                "direction": "a_to_b" if sender == 0 else "b_to_a"}
    finally:
        session.probes[receiver].call("approve", peer_id=session.probes[sender].identity_id)
    raise QualificationError("revoked_peer_was_accepted")


def verified_staging_bytes(stage, source, block_size):
    total = 0
    source = Path(source)
    size = source.stat().st_size
    for part in Path(stage).glob("block-*.part"):
        if part.is_symlink():
            raise QualificationError("staging_block_is_symbolic")
        match = re.fullmatch(r"block-([0-9]{8})\.part", part.name)
        if not match or not part.is_file():
            continue
        index = int(match[1])
        offset = index * block_size
        expected_size = min(block_size, size - offset)
        if expected_size <= 0 or part.stat().st_size != expected_size:
            continue
        with source.open("rb") as original:
            original.seek(offset)
            expected = hashlib.sha256(original.read(expected_size)).hexdigest()
        if sha256(part) == expected:
            total += expected_size
    return total


def process_death_case(session, source, root, *, sender=0):
    """Kill the owned receiver in an observed paced transfer, before its next block commits."""
    if session.route != "forced_relay":
        raise QualificationError("mid_block_requires_controlled_relay")
    root = new_directory(root)
    block_size = 1024 * 1024
    target, stage = root / "received.bin", root / "partial"
    receiver = 1 - sender
    original_rate = session.relay_rate
    session.relay_rate = 1024 * 1024
    session.restart()
    address = session.offer(sender, source, block_size)
    outcome = {}
    def receiving():
        try:
            outcome["report"] = session.receive(receiver, address, stage, target)
        except (OSError, ValueError, QualificationError):
            outcome["interrupted"] = True
    worker = threading.Thread(target=receiving, daemon=True)
    worker.start()
    deadline = time.monotonic() + 30
    forwarded_at_first = None
    forwarded_delta = None
    killed = False
    try:
        while time.monotonic() < deadline:
            if target.exists() or not worker.is_alive():
                raise QualificationError("mid_block_window_not_observed")
            first = stage / "block-00000000.part"
            second = stage / "block-00000001.part"
            if first.is_file() and not second.exists():
                forwarded = session.relay.stats()["forwarded_bytes"]
                if forwarded_at_first is None:
                    if verified_staging_bytes(stage, source, block_size) != block_size:
                        raise QualificationError("first_crash_checkpoint_not_verified")
                    forwarded_at_first = forwarded
                elif forwarded - forwarded_at_first >= block_size // 4:
                    forwarded_delta = forwarded - forwarded_at_first
                    if second.exists() or target.exists():
                        raise QualificationError("mid_block_window_changed")
                    session.probes[receiver].process.kill()
                    session.probes[receiver].process.wait(timeout=3)
                    killed = True
                    break
            time.sleep(0.01)
        if not killed:
            raise QualificationError("mid_block_window_not_observed")
        worker.join(timeout=5)
        if worker.is_alive() or target.exists() or (stage / "block-00000001.part").exists():
            raise QualificationError("crash_before_next_commit_not_proven")
        exit_code = session.probes[receiver].process.returncode
        if exit_code != -signal.SIGKILL:
            raise QualificationError("abrupt_receiver_death_not_proven")
        if outcome.get("report", {}).get("complete"):
            raise QualificationError("killed_receiver_reported_complete")
        durable = verified_staging_bytes(stage, source, block_size)
        if durable != block_size:
            raise QualificationError("unexpected_crash_checkpoint")
        session.relay_rate = original_rate
        session.restart()
        address = session.offer(sender, source, block_size)
        resumed = session.receive(receiver, address, stage, target)
        validate_transfer(resumed, source, target, expected_bytes=source.stat().st_size - durable)
        unchanged = session.receive(receiver, address, stage, target)
        validate_transfer(unchanged, source, target, expected_bytes=0)
        return {"case": "process_death_mid_block", "complete": True,
                "direction": "a_to_b" if sender == 0 else "b_to_a", "block_size": block_size,
                "durable_content_bytes_before_restart": durable,
                "opaque_forwarded_bytes_after_checkpoint": forwarded_delta,
                "next_block_durable_at_kill": False, "receiver_exit_code": exit_code,
                "resumed_content_bytes": resumed["bytes_received"], "unchanged_content_bytes": 0,
                "scope": "forced_relay_paced_in_flight_interruption"}
    finally:
        session.relay_rate = original_rate
        if worker.is_alive():
            child = session.probes[receiver].process
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)
            worker.join(timeout=5)


def run_host(args):
    root = new_directory(args.output)
    fixtures = make_fixtures(root / "fixtures")
    result = {"schema_version": 1, "scope": "host_matrix", "host_cases_complete": False, "qualification_complete": False,
              "created_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "fixture_bytes": fixtures["binary"].stat().st_size, "fixture_sha256": sha256(fixtures["binary"]),
              "results": [], "gaps": ["physical_device_acceptance", "capture_interference", "suspend_network_transitions",
                                       "preauthentication_flood_measurements", "process_death_mid_block"]}
    result["controller_sha256"] = sha256(Path(__file__))
    result["candidate_binary_sha256"] = {candidate: sha256(args.quinn_binary if candidate == "quinn" else args.native_binary)
                                            for candidate in args.candidates
                                            if (args.quinn_binary if candidate == "quinn" else args.native_binary) is not None}
    try:
        result["source_revision"] = subprocess.check_output(["git", "rev-parse", "HEAD"],
            cwd=Path(__file__).resolve().parents[1], text=True, stderr=subprocess.DEVNULL).strip()
    except (OSError, subprocess.SubprocessError):
        result["source_revision"] = None
    write_private_json(root / "private-run-config.json", {key: str(value) if isinstance(value, Path) else value
                                                          for key, value in vars(args).items()})
    try:
        for candidate in args.candidates:
            binary = args.quinn_binary if candidate == "quinn" else args.native_binary
            if binary is None:
                raise QualificationError("candidate_binary_missing")
            for route in args.routes:
                lane = root / f"{candidate}-{route}"
                session = HostSession(binary, candidate, route, lane, args.run_id, args.relay_binary)
                lane_results = []
                try:
                    for block_size in args.block_sizes:
                        for repeat in range(args.repeats):
                            for name, source in fixtures.items():
                                for sender in range(2):
                                    case_name = f"transfer-{block_size}-{repeat}-{name}-{sender}"
                                    lane_results.append(host_case(session, source, lane / case_name, block_size, repeat, sender, name))
                    for sender in range(2):
                        for corrupt in (False, True):
                            case_name = ("corruption" if corrupt else "restart") + f"-{sender}"
                            lane_results.append(recovery_case(session, fixtures["binary"], lane / case_name,
                                                              1024 * 1024, corrupt=corrupt, sender=sender))
                        lane_results.append(authorization_case(session, fixtures["binary"], lane / f"revocation-{sender}",
                                                               1024 * 1024, sender=sender))
                        if route == "forced_relay" and getattr(args, "mid_block_crash", False):
                            lane_results.append(process_death_case(session, fixtures["binary"], lane / f"mid-block-death-{sender}", sender=sender))
                finally:
                    session.close()
                    extra_crash_cases = 2 if route == "forced_relay" and getattr(args, "mid_block_crash", False) else 0
                    result["results"].append({"candidate": candidate, "route": route, "results": lane_results,
                                               "host_cases_complete": len(lane_results) == len(args.block_sizes) * args.repeats * len(fixtures) * 2 + 6 + extra_crash_cases})
                    write_private_json(root / "result.json", result, replace=True)
        result["host_cases_complete"] = True
        if getattr(args, "mid_block_crash", False):
            result["gaps"].remove("process_death_mid_block")
            result["gaps"].append("process_death_mid_block_direct")
    except (OSError, ValueError, QualificationError) as error:
        result["failure"] = str(error) if isinstance(error, QualificationError) else "controller_failure"
        raise
    finally:
        write_private_json(root / "result.json", result, replace=True)
        write_private_json(root / "public-summary.json", public_summary(result), replace=True)
    return result


class DeviceEvidence:
    def __init__(self, source, candidate, route, block_size):
        self.source, self.candidate, self.route, self.block_size = Path(source), candidate, route, block_size

    def validate(self, result):
        allowed = {"candidate", "route", "complete", "first_bytes", "resumed_bytes", "unchanged_bytes",
                   "returned_bytes", "sha256", "elapsed_seconds"}
        if not isinstance(result, dict) or set(result) not in (allowed, allowed | {"metrics"}):
            raise QualificationError("invalid_device_result")
        if "metrics" in result:
            validate_device_metrics(result["metrics"])
        if result["candidate"] != self.candidate or result["route"] != self.route or result["complete"] is not True:
            raise QualificationError("incorrect_device_route")
        if result["sha256"] != sha256(self.source):
            raise QualificationError("device_digest_mismatch")
        counts = ("first_bytes", "resumed_bytes", "unchanged_bytes", "returned_bytes")
        if any(not nonnegative_int(result[key]) for key in counts):
            raise QualificationError("invalid_device_counts")
        size = self.source.stat().st_size
        if (result["first_bytes"] != min(self.block_size, size)
                or result["first_bytes"] + result["resumed_bytes"] != size
                or result["unchanged_bytes"] != 0 or result["returned_bytes"] != size):
            raise QualificationError("incorrect_device_byte_evidence")
        elapsed = result["elapsed_seconds"]
        if type(elapsed) not in (int, float) or not math.isfinite(elapsed) or elapsed < 0:
            raise QualificationError("invalid_device_timing")


class DeviceController:
    def __init__(self, args):
        self.args = args
        self.root = new_directory(args.output)
        self.fixtures = make_fixtures(self.root / "fixtures", mib=args.mib)
        self.source = self.fixtures["binary"]
        self.token = secrets.token_urlsafe(32)
        self.peer, self.fixture, self.result = None, None, None
        self.lock = threading.RLock()
        self.probe = Probe(args.binary, args.candidate, self.root / "identity.json", f"peer-probe-{args.run_id}",
                           self.root / "probe-initial.stderr.log", bind=f"{args.bind}:0")
        self.identity = self.probe.identity_id
        self.relay = None
        self.preparation_error = None
        self.preparation_thread = None
        self.capture_results = {}

    def prepare(self, peer):
        if not isinstance(peer, str) or not ID.fullmatch(peer):
            raise QualificationError("invalid_device_identity")
        with self.lock:
            if self.peer and not hmac.compare_digest(self.peer, peer):
                raise QualificationError("device_identity_already_pinned")
            if self.peer:
                return self.preparation_config
            self.peer = peer
            relay_config = None
            if self.args.route == "forced_relay":
                self.probe.close()
                self.relay = Relay(self.args.relay_binary, self.root, self.args.candidate,
                                   [self.identity, peer], self.args.run_id, bind=self.args.bind,
                                   rate=self.args.relay_rate_bytes_per_second)
                relay_config = json.loads(self.relay.configs[1].read_text())
                self.probe = Probe(self.args.binary, self.args.candidate, self.root / "identity.json",
                    f"peer-probe-{self.args.run_id}", self.root / "probe-relay.stderr.log",
                    relay_config=self.relay.configs[0], bind=f"{self.args.bind}:0", start=False)
            self.preparation_config = {"candidate": self.args.candidate, "route": self.args.route,
                "source_identity": self.identity, "relay_config_json": json.dumps(relay_config) if relay_config else None}
            self.preparation_thread = threading.Thread(target=self._finish_prepare, daemon=True)
            self.preparation_thread.start()
            return self.preparation_config

    def _finish_prepare(self):
        try:
            if self.args.route == "forced_relay":
                self.probe.ready()
            if self.probe.identity_id != self.identity:
                raise QualificationError("host_identity_changed")
            self.probe.call("approve", peer_id=self.peer)
            manifest = self.probe.call("offer", path=str(self.source), block_size=self.args.block_size)
            if isinstance(manifest, str):
                manifest = json.loads(manifest)
            if not isinstance(manifest, dict) or manifest.get("sha256") != sha256(self.source):
                raise QualificationError("host_fixture_mismatch")
            address = normalize_address(self.probe.call("address"), self.identity)
            self.fixture = {"candidate": self.args.candidate, "route": self.args.route,
                "source_identity": self.identity, "source_address_json": address,
                "block_size": self.args.block_size, "total_bytes": self.source.stat().st_size,
                "sha256": sha256(self.source)}
        except (OSError, ValueError, QualificationError):
            self.preparation_error = "device_preparation_failed"

    def get_fixture(self):
        if self.preparation_error:
            raise QualificationError(self.preparation_error)
        if self.fixture is None:
            raise QualificationError("fixture_not_ready")
        return self.fixture

    def upload(self, address_json):
        if self.fixture is None or self.peer is None:
            raise QualificationError("fixture_not_ready")
        address = normalize_address(address_json, self.peer)
        target = self.root / "returned.bin"
        report = self.probe.call("receive", address_json=address, staging=str(self.root / "returned.partial"),
                                 target=str(target), max_blocks=None)
        validate_transfer(report, self.source, target, expected_bytes=self.source.stat().st_size)
        self.return_verified = True
        return report

    def save_result(self, value):
        DeviceEvidence(self.source, self.args.candidate, self.args.route, self.args.block_size).validate(value)
        if not getattr(self, "return_verified", False):
            raise QualificationError("host_return_not_verified")
        returned = self.root / "returned.bin"
        if not returned.is_file() or sha256(returned) != sha256(self.source):
            raise QualificationError("host_return_changed")
        value = json.loads(json.dumps(value, allow_nan=False))
        with self.lock:
            if self.result is not None:
                if self.result != value:
                    raise QualificationError("different_device_result_already_saved")
                return {"saved": True}
            private_path = self.root / "private-device-result.json"
            for path, document in (
                (private_path, value),
                (self.root / "device-result.json", value),
                (self.root / "public-device-result.json", public_summary({**value, "scope": "device_transfer", "qualification_complete": False})),
            ):
                if path.exists() or path.is_symlink():
                    if (path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_CONTROL
                            or json.loads(path.read_text()) != document):
                        raise QualificationError("different_device_result_already_saved")
                elif path.name == "device-result.json":
                    # Preserve the original receipt filename as an exact compatibility alias.
                    os.link(private_path, path)
                    descriptor = os.open(self.root, os.O_RDONLY)
                    try:
                        os.fsync(descriptor)
                    finally:
                        os.close(descriptor)
                else:
                    write_private_json(path, document)
            self.result = value
            return {"saved": True}

    def save_capture_result(self, value):
        required = {"candidate", "route", "phase", "transport_ongoing", "measurements"}
        if not isinstance(value, dict) or set(value) != required:
            raise QualificationError("invalid_capture_result")
        phase = value["phase"]
        if (value["candidate"] != self.args.candidate or value["route"] != self.args.route
                or phase not in ("baseline", "active", "stalled")):
            raise QualificationError("incorrect_capture_scope")
        if type(value["transport_ongoing"]) is not bool or value["transport_ongoing"] != (phase != "baseline"):
            raise QualificationError("capture_transport_state_not_proven")
        rows = value["measurements"]
        if not isinstance(rows, list) or len(rows) != 10:
            raise QualificationError("capture_requires_ten_measurements")
        for row in rows:
            if not isinstance(row, dict) or set(row) != {"start_ms", "stop_ms", "samples", "callback_after_stop"}:
                raise QualificationError("invalid_capture_measurement")
            for field in ("start_ms", "stop_ms"):
                if type(row[field]) not in (int, float) or not math.isfinite(row[field]) or row[field] < 0:
                    raise QualificationError("invalid_capture_timing")
            if not nonnegative_int(row["samples"]) or row["samples"] == 0:
                raise QualificationError("capture_samples_missing")
            if row["callback_after_stop"] is not False:
                raise QualificationError("capture_callback_after_stop")
        # Canonical copying prevents caller mutation from changing an already accepted receipt.
        value = json.loads(json.dumps(value, allow_nan=False))
        with self.lock:
            existing = self.capture_results.get(phase)
            if existing is not None:
                if existing != value:
                    raise QualificationError("different_capture_result_already_saved")
                return {"saved": True, "phase": phase, "qualification_complete": False}
            path = self.root / f"capture-{phase}-result.json"
            if path.exists() or path.is_symlink():
                if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_CONTROL:
                    raise QualificationError("capture_receipt_unavailable")
                if json.loads(path.read_text()).get("result") != value:
                    raise QualificationError("different_capture_result_already_saved")
            else:
                write_private_json(path, {"schema_version": 1, "qualification_complete": False,
                    "scope": "device_reported_capture", "result": value})
            phases = {**self.capture_results, phase: value}
            summaries = {}
            for name, result in phases.items():
                measurements = result["measurements"]
                summaries[name] = {"start_median_ms": statistics.median(row["start_ms"] for row in measurements),
                    "start_max_ms": max(row["start_ms"] for row in measurements),
                    "stop_median_ms": statistics.median(row["stop_ms"] for row in measurements),
                    "stop_max_ms": max(row["stop_ms"] for row in measurements)}
            write_private_json(self.root / "private-capture-result.json", {"schema_version": 1,
                "candidate": self.args.candidate, "route": self.args.route, "qualification_complete": False,
                "capture_phases_complete": set(phases) == {"baseline", "active", "stalled"},
                "scope": "device_reported_capture", "phases": phases, "summaries": summaries}, replace=True)
            self.capture_results = phases
            return {"saved": True, "phase": phase, "qualification_complete": False}

    def set_relay_paused(self, paused):
        with self.lock:
            if self.relay is None or self.fixture is None or self.peer is None:
                raise QualificationError("relay_session_not_prepared")
            return self.relay.set_paused(paused)

    def close(self):
        self.probe.close()
        if self.relay:
            self.relay.close()
        if self.preparation_thread:
            self.preparation_thread.join(timeout=3)


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(15)

    def log_message(self, *_):
        pass

    def reply(self, status, value):
        encoded = json.dumps(value, allow_nan=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(encoded)

    def authorized(self):
        expected = "Bearer " + self.server.controller.token
        if not hmac.compare_digest(self.headers.get("Authorization", "").encode(), expected.encode()):
            self.close_connection = True
            self.reply(403, {"error": "unauthorized"})
            return False
        return True

    def do_GET(self):
        if not self.authorized():
            return
        if self.path == "/status":
            self.reply(200, {"complete": self.server.controller.result is not None,
                             "capture_phases": sorted(getattr(self.server.controller, "capture_results", {}))})
        elif self.path == "/fixture":
            try:
                self.reply(200, self.server.controller.get_fixture())
            except QualificationError as error:
                self.reply(503, {"error": str(error)})
        else:
            self.reply(404, {"error": "not_found"})

    def do_POST(self):
        if not self.authorized():
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if self.headers.get("Transfer-Encoding") or len(self.headers.get_all("Content-Length", [])) != 1 or not 0 < length <= MAX_CONTROL:
                self.close_connection = True
                self.reply(413, {"error": "request_limit"})
                return
            raw = self.rfile.read(length)
            if len(raw) != length:
                raise QualificationError("truncated_request")
            body = json.loads(raw, parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
            if not isinstance(body, dict):
                raise QualificationError("invalid_request")
            state = self.server.controller
            if self.path == "/prepare":
                result = state.prepare(body.get("peer_id"))
            elif self.path == "/upload":
                result = state.upload(body.get("address_json"))
            elif self.path == "/result":
                result = state.save_result(body)
            elif self.path == "/capture-result":
                result = state.save_capture_result(body)
            elif self.path in ("/pause", "/resume"):
                if body:
                    raise QualificationError("relay_control_requires_empty_body")
                result = state.set_relay_paused(self.path == "/pause")
            elif self.path == "/shutdown":
                result = {"stopping": True}
                threading.Thread(target=self.server.shutdown, daemon=True).start()
            else:
                self.reply(404, {"error": "not_found"})
                return
            self.reply(200, result)
        except (ValueError, TypeError, QualificationError):
            self.reply(400, {"error": "invalid_request"})
        except OSError:
            self.reply(503, {"error": "qualification_failed"})


def validate_bind(value):
    ip = ipaddress.IPv4Address(value)
    private = any(ip in ipaddress.ip_network(cidr) for cidr in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"))
    if not ip.is_loopback and not private:
        raise argparse.ArgumentTypeError("Use an explicit loopback or RFC1918 IPv4 address")
    return str(ip)


class BoundedHTTPServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, handler):
        self.admissions = threading.BoundedSemaphore(16)
        super().__init__(address, handler)

    def process_request(self, request, client_address):
        if not self.admissions.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self.admissions.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.admissions.release()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="mode", required=True)
    host = subparsers.add_parser("host", help="Run the host correctness/recovery and timing matrix")
    host.add_argument("--quinn-binary", type=Path)
    host.add_argument("--native-binary", type=Path)
    host.add_argument("--candidates", nargs="+", choices=("quinn", "native"), default=["quinn", "native"])
    host.add_argument("--routes", nargs="+", choices=("direct", "forced_relay"), default=["direct", "forced_relay"])
    host.add_argument("--block-sizes", nargs="+", type=int, choices=BLOCK_SIZES, default=list(BLOCK_SIZES))
    host.add_argument("--repeats", type=int, choices=range(1, 4), default=3)
    host.add_argument("--mid-block-crash", action="store_true", help="Observe paced forced-relay SIGKILL/recovery in both directions")
    device = subparsers.add_parser("device", help="Serve a bounded Mac/iPhone qualification control session")
    device.add_argument("--candidate", choices=("quinn", "native"), required=True)
    device.add_argument("--route", choices=("direct", "forced_relay"), required=True)
    device.add_argument("--binary", type=Path, required=True)
    device.add_argument("--bind", type=validate_bind, required=True)
    device.add_argument("--port", type=int, default=0)
    device.add_argument("--block-size", type=int, choices=BLOCK_SIZES, default=1024 * 1024)
    device.add_argument("--mib", type=int, default=8, help="Synthetic binary fixture size, 1–256 MiB plus the retained tail")
    device.add_argument("--relay-rate-bytes-per-second", type=int, default=0, help="Test-only relay pacing; 0 means unlimited")
    for command in (host, device):
        command.add_argument("--relay-binary", type=Path)
        command.add_argument("--output", type=Path, required=True, help="A new private evidence directory")
        command.add_argument("--run-id", required=True)
    args = parser.parse_args(argv)
    if not RUN_ID.fullmatch(args.run_id) or len(args.run_id) > 45:
        parser.error("Use a 1–45 character alphanumeric/hyphen run ID")
    if args.mode == "device" and not 1 <= args.mib <= 256:
        parser.error("Use a synthetic fixture between 1 and 256 MiB")
    if args.mode == "device" and not 0 <= args.relay_rate_bytes_per_second <= 64 * 1024 * 1024:
        parser.error("Relay pacing must be between 0 and 67108864 bytes per second")
    if args.mode == "host" and args.mid_block_crash and "forced_relay" not in args.routes:
        parser.error("Mid-block process-death testing requires the forced-relay route")
    for field in ("binary", "quinn_binary", "native_binary", "relay_binary"):
        if getattr(args, field, None):
            setattr(args, field, getattr(args, field).resolve(strict=True))
    if (getattr(args, "route", None) == "forced_relay" or "forced_relay" in getattr(args, "routes", [])) and not args.relay_binary:
        parser.error("Forced relay requires --relay-binary")
    os.umask(0o077)
    # Terminating this controller must also retire only its owned child processes.
    def interrupted(_signal, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    if args.mode == "host":
        result = run_host(args)
        print(json.dumps({"host_cases_complete": result["host_cases_complete"], "qualification_complete": False,
                          "public_summary": str(args.output / "public-summary.json")}))
        return 0
    state = DeviceController(args)
    server = None
    try:
        server = BoundedHTTPServer((args.bind, args.port), Handler)
        server.controller = state
        config = {"coordinator_url": f"http://{args.bind}:{server.server_port}", "token": state.token,
                  "candidate": args.candidate, "route": args.route}
        config_path = state.root / "private-client-config.json"
        write_private_json(config_path, config)
        print(json.dumps({"ready": True, "private_config": str(config_path)}), flush=True)
        server.serve_forever()
    finally:
        if server:
            server.server_close()
        state.close()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, QualificationError):
        print("Peer transport comparison failed; inspect the private evidence directory.", file=sys.stderr)
        raise SystemExit(1)
    except KeyboardInterrupt:
        raise SystemExit(130)
