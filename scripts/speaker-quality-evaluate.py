#!/usr/bin/env python3
"""Replay local speaker options and score explicit, private annotation regions.

No networking, recognition, model downloads, names, or app/profile mutation. WAVs must already be
16 kHz mono PCM16. A reference has schema_version=1, annotated_regions=[{start_ms,end_ms}], and
turns=[{start_ms,end_ms,speaker}], using opaque speaker IDs. Regions explicitly include annotated
silence; unannotated recording time is excluded, never treated as known silence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import time


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def intervals(values: list[dict], *, speaker: bool) -> list[dict]:
    if not isinstance(values, list):
        raise ValueError("intervals must be an array")
    result = []
    for value in values:
        if not isinstance(value, dict):
            raise ValueError("interval must be an object")
        start, end = value.get("start_ms"), value.get("end_ms")
        if type(start) is not int or type(end) is not int or not 0 <= start < end:
            raise ValueError("interval times must be nonnegative integer milliseconds with start < end")
        item = {"start_ms": start, "end_ms": end}
        if speaker:
            label = value.get("speaker")
            if type(label) not in (str, int) or not str(label).strip():
                raise ValueError("speaker must be a nonempty opaque string or integer")
            item["speaker"] = str(label)
        result.append(item)
    return sorted(result, key=lambda item: (item["start_ms"], item["end_ms"], item.get("speaker", "")))


def load_reference(value: dict) -> tuple[list[dict], list[dict]]:
    if value.get("schema_version") != 1 or not value.get("annotated_regions"):
        raise ValueError("reference requires schema_version=1 and explicit annotated_regions")
    regions = intervals(value["annotated_regions"], speaker=False)
    if any(a["end_ms"] > b["start_ms"] for a, b in zip(regions, regions[1:])):
        raise ValueError("annotation regions must not overlap")
    reference = intervals(value.get("turns", []), speaker=True)
    for turn in reference:
        if not any(region["start_ms"] <= turn["start_ms"] < turn["end_ms"] <= region["end_ms"] for region in regions):
            raise ValueError("every reference turn must be contained in an annotated region")
    return regions, reference


def best_mapping(overlap: list[list[int]]) -> dict[int, int]:
    """Maximum-weight one-to-one assignment; square Hungarian algorithm, including dummy labels."""
    height = len(overlap)
    width = len(overlap[0]) if height else 0
    size = max(height, width)
    if not size:
        return {}
    maximum = max((max(row, default=0) for row in overlap), default=0)
    cost = [[maximum - (overlap[i][j] if i < height and j < width else 0) for j in range(size)] for i in range(size)]
    u, v, assigned, predecessor = ([0] * (size + 1) for _ in range(4))
    for row in range(1, size + 1):
        assigned[0] = row
        column = 0
        minimum = [float("inf")] * (size + 1)
        visited = [False] * (size + 1)
        while True:
            visited[column] = True
            current = assigned[column]
            delta, following = float("inf"), 0
            for candidate in range(1, size + 1):
                if visited[candidate]:
                    continue
                reduced = cost[current - 1][candidate - 1] - u[current] - v[candidate]
                if reduced < minimum[candidate]:
                    minimum[candidate], predecessor[candidate] = reduced, column
                if minimum[candidate] < delta:
                    delta, following = minimum[candidate], candidate
            for candidate in range(size + 1):
                if visited[candidate]:
                    u[assigned[candidate]] += delta
                    v[candidate] -= delta
                else:
                    minimum[candidate] -= delta
            column = following
            if assigned[column] == 0:
                break
        while column:
            previous = predecessor[column]
            assigned[column] = assigned[previous]
            column = previous
    return {assigned[column] - 1: column - 1 for column in range(1, size + 1)
            if assigned[column] <= height and column <= width}


def score(reference: dict, hypothesis: list[dict]) -> dict:
    regions, turns = load_reference(reference)
    hypothesis = [turn for turn in intervals(hypothesis, speaker=True)
                  if any(turn["start_ms"] < region["end_ms"] and turn["end_ms"] > region["start_ms"]
                         for region in regions)]
    references = sorted({turn["speaker"] for turn in turns})
    guesses = sorted({turn["speaker"] for turn in hypothesis})
    cells = []
    for region in regions:
        start, end = region["start_ms"], region["end_ms"]
        relevant = [turn for turn in turns + hypothesis if turn["start_ms"] < end and turn["end_ms"] > start]
        boundaries = sorted({start, end} | {max(start, min(end, turn[key])) for turn in relevant for key in ("start_ms", "end_ms")})
        for left, right in zip(boundaries, boundaries[1:]):
            truth = {turn["speaker"] for turn in turns if turn["start_ms"] <= left < turn["end_ms"]}
            prediction = {turn["speaker"] for turn in hypothesis if turn["start_ms"] <= left < turn["end_ms"]}
            cells.append((right - left, truth, prediction))
    overlap = [[sum(duration for duration, truth, prediction in cells if guess in prediction and person in truth)
                for person in references] for guess in guesses]
    mapping = {guesses[guess]: references[person] for guess, person in best_mapping(overlap).items()}
    totals = {"reference_speaker_ms": 0, "missed_speaker_ms": 0, "false_alarm_speaker_ms": 0,
              "confusion_speaker_ms": 0, "non_overlap_reference_ms": 0, "non_overlap_confusion_ms": 0,
              "non_overlap_error_ms": 0, "non_overlap_correct_ms": 0}
    for duration, truth, prediction in cells:
        correct = len(truth & {mapping.get(guess) for guess in prediction})
        missed = max(0, len(truth) - len(prediction))
        false_alarm = max(0, len(prediction) - len(truth))
        confused = min(len(truth), len(prediction)) - correct
        totals["reference_speaker_ms"] += duration * len(truth)
        totals["missed_speaker_ms"] += duration * missed
        totals["false_alarm_speaker_ms"] += duration * false_alarm
        totals["confusion_speaker_ms"] += duration * confused
        if len(truth) == 1:
            totals["non_overlap_reference_ms"] += duration
            totals["non_overlap_confusion_ms"] += duration * confused
            totals["non_overlap_error_ms"] += duration * (missed + false_alarm + confused)
            totals["non_overlap_correct_ms"] += duration * correct
    denominator = totals["reference_speaker_ms"]
    non_overlap = totals["non_overlap_reference_ms"]
    return {**totals,
            "annotated_wall_ms": sum(region["end_ms"] - region["start_ms"] for region in regions),
            "reference_speaker_count": len(references), "hypothesis_speaker_count": len(guesses),
            "diarization_error_rate": (totals["missed_speaker_ms"] + totals["false_alarm_speaker_ms"] + totals["confusion_speaker_ms"]) / denominator if denominator else None,
            "non_overlap_confusion_rate": totals["non_overlap_confusion_ms"] / non_overlap if non_overlap else None,
            "non_overlap_error_rate": totals["non_overlap_error_ms"] / non_overlap if non_overlap else None,
            "non_overlap_correct_coverage": totals["non_overlap_correct_ms"] / non_overlap if non_overlap else None,
            "hypothesis_to_reference": mapping,
            "scoring": "exact intervals, globally optimal one-to-one mapping, zero collar, overlap scored, explicit regions only"}


def peak_rss(stderr: str) -> int | None:
    mac = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
    linux = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", stderr)
    return int(mac[1]) if mac else int(linux[1]) * 1024 if linux else None


def run_owned(command: list[str], cwd: Path, seconds: float) -> tuple[subprocess.CompletedProcess, bool]:
    """A timeout stops the owned process group, including the native child of /usr/bin/time."""
    if seconds <= 0:
        return subprocess.CompletedProcess(command, 124, "", "absolute evaluation deadline reached\n"), True
    try:
        process = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, start_new_session=True)
    except OSError as error:
        return subprocess.CompletedProcess(command, 127, "", f"{type(error).__name__}: {error}\n"), False
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            stdout, stderr = process.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = process.communicate()
    return subprocess.CompletedProcess(command, 124 if timed_out else process.returncode, stdout, stderr), timed_out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--output", type=Path, required=True, help="new private directory outside version control")
    parser.add_argument("--runner", type=Path, help="already-built evaluate_recording binary; otherwise build offline")
    parser.add_argument("--window-shifts", type=float, nargs="+", default=[0.25, 0.1])
    parser.add_argument("--cluster-thresholds", type=float, nargs="+", default=[0.6])
    parser.add_argument("--deadline-seconds", type=float, default=600, help="absolute build-and-evaluation budget")
    parser.add_argument("--voice-diagnostics", action="store_true", help="sample clean-turn coherence without exporting voiceprints")
    args = parser.parse_args()
    if not 0 < args.deadline_seconds <= 3_600:
        parser.error("deadline must be positive and at most 3600 seconds")
    if any(not math.isfinite(value) or not 0 < value <= 1 for value in args.window_shifts):
        parser.error("window shifts must be finite values in (0, 1]")
    if any(not math.isfinite(value) or not 0 <= value <= 2 for value in args.cluster_thresholds):
        parser.error("cluster thresholds must be finite cosine distances in [0, 2]")
    deadline = time.monotonic() + args.deadline_seconds
    root = Path(__file__).resolve().parents[1]
    audio = args.audio.resolve(strict=True)
    segmentation = (args.models / "segmentation.onnx").resolve(strict=True)
    embedding = (args.models / "embedding.onnx").resolve(strict=True)
    reference = json.loads(args.reference.read_text()) if args.reference else None
    if reference is not None:
        load_reference(reference)
    output = args.output.resolve()
    if output.is_relative_to(root):
        ignored = subprocess.run(["git", "check-ignore", "-q", "--", str(output.relative_to(root))], cwd=root)
        if ignored.returncode:
            parser.error("private evaluation output must be outside the repository or in a gitignored directory")
    output.mkdir(parents=True, exist_ok=False)
    output.chmod(0o700)
    runner = args.runner
    toolchain = root / "scripts/with-toolchain.sh"
    manifest = root / "core/speakers/Cargo.toml"
    if runner is None:
        command = [str(toolchain), "cargo", "build", "--manifest-path", str(manifest), "--example", "evaluate_recording", "--features", "onnx", "--offline"]
        built, _ = run_owned(command, root, deadline - time.monotonic())
        (output / "build.stdout").write_text(built.stdout)
        (output / "build.stderr").write_text(built.stderr)
        if built.returncode:
            print(f"offline evaluator build failed; retained output in {output}", file=sys.stderr)
            return built.returncode
        metadata_result, _ = run_owned([str(toolchain), "cargo", "metadata", "--manifest-path", str(manifest), "--no-deps", "--offline", "--format-version", "1"], root, deadline - time.monotonic())
        if metadata_result.returncode:
            (output / "metadata.stderr").write_text(metadata_result.stderr)
            return metadata_result.returncode
        metadata = json.loads(metadata_result.stdout)
        runner = Path(metadata["target_directory"]) / "debug/examples/evaluate_recording"
    runner = runner.resolve(strict=True)
    report = {"schema_version": 1, "audio_sha256": digest(audio),
              "models": {"segmentation_sha256": digest(segmentation), "embedding_sha256": digest(embedding)},
              "runner_sha256": digest(runner), "reference_sha256": digest(args.reference) if args.reference else None,
              "source_sha256": {path: digest(root / path) for path in ["core/speakers/src/lib.rs", "core/speakers/src/pipeline.rs", "core/speakers/examples/evaluate_recording.rs", "scripts/speaker-quality-evaluate.py"]},
              "reference_status": "annotated_regions_only" if reference is not None else "unannotated_no_accuracy_claim",
              "deadline_seconds": args.deadline_seconds,
              "attempts": []}
    failed = False
    for shift in args.window_shifts:
        for threshold in args.cluster_thresholds:
            stem = f"shift-{shift:g}-cluster-{threshold:g}"
            command = [str(runner), str(audio), str(segmentation), str(embedding), str(shift), str(threshold)]
            if args.voice_diagnostics:
                command.append("--voice-diagnostics")
            timer = Path("/usr/bin/time")
            if timer.exists():
                command = [str(timer), "-l" if platform.system() == "Darwin" else "-v", *command]
            started = time.monotonic()
            result, timed_out = run_owned(command, root, deadline - time.monotonic())
            elapsed = time.monotonic() - started
            (output / f"{stem}.stdout").write_text(result.stdout)
            (output / f"{stem}.stderr").write_text(result.stderr)
            attempt = {"window_shift": shift, "cluster_threshold": threshold, "exit_code": result.returncode,
                       "wall_seconds": elapsed, "peak_rss_bytes": peak_rss(result.stderr), "timed_out": timed_out}
            if result.returncode == 0:
                try:
                    evaluated = json.loads(result.stdout)
                    if reference is not None:
                        if max(region["end_ms"] for region in reference["annotated_regions"]) > evaluated["duration_ms"]:
                            raise ValueError("reference extends beyond audio")
                        for stage in ("raw", "reconciled"):
                            evaluated[stage]["metrics"] = score(reference, evaluated[stage]["turns"])
                    attempt["result"] = evaluated
                except (ValueError, KeyError) as error:
                    attempt["invalid_result"] = str(error)
                    failed = True
            else:
                failed = True
            report["attempts"].append(attempt)
            (output / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
            print(json.dumps({"attempt": stem, "exit_code": result.returncode, "wall_seconds": round(elapsed, 3),
                              "peak_rss_bytes": attempt["peak_rss_bytes"],
                              "speakers": attempt.get("result", {}).get("reconciled", {}).get("speaker_count"),
                              "reference_status": report["reference_status"]}))
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
