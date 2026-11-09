#!/usr/bin/env python3
"""Create explicitly model-derived diagnostic name intervals, never captured Teams evidence."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def consensus(default, offline, mappings, duration):
    outputs = [default["raw"]["turns"], offline["raw"]["turns"]]
    boundaries = sorted({0, duration} | {
        max(0, min(duration, turn[key]))
        for turns in outputs for turn in turns for key in ("start_ms", "end_ms")
    })
    accepted, unavailable = [], []
    for start, end in zip(boundaries, boundaries[1:]):
        active = [{str(turn["speaker"]) for turn in turns
                   if turn["start_ms"] <= start < turn["end_ms"]}
                  for turns in outputs]
        names = [mapping.get(next(iter(voices))) if len(voices) == 1 else None
                 for voices, mapping in zip(active, mappings)]
        if all(len(voices) == 1 for voices in active) and names[0] and names[0] == names[1]:
            if accepted and accepted[-1]["end_ms"] == start and accepted[-1]["speaker"] == names[0]:
                accepted[-1]["end_ms"] = end
            else:
                accepted.append({"start_ms": start, "end_ms": end, "speaker": names[0]})
            continue
        if any(len(voices) > 1 for voices in active):
            reason = "model_overlap"
        elif not any(active):
            reason = "both_models_silence"
        elif not all(active):
            reason = "only_one_model_has_speech"
        elif any(name is None for name in names):
            reason = "unmapped_voice"
        else:
            reason = "models_disagree"
        if unavailable and unavailable[-1]["end_ms"] == start and unavailable[-1]["reason"] == reason:
            unavailable[-1]["end_ms"] = end
        else:
            unavailable.append({"start_ms": start, "end_ms": end, "reason": reason})
    return accepted, unavailable


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--default-output", type=Path, required=True)
    parser.add_argument("--offline-output", type=Path, required=True)
    parser.add_argument("--calibration", type=Path, required=True)
    parser.add_argument("--source-audio", type=Path, required=True, help="original copied physical system-audio source")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    if output.is_relative_to(root):
        ignored = subprocess.run(["git", "check-ignore", "-q", "--", str(output.relative_to(root))], cwd=root)
        if ignored.returncode:
            parser.error("private timeline must be outside the repository or gitignored")
    spec = importlib.util.spec_from_file_location("speaker_quality", root / "scripts/speaker-quality-evaluate.py")
    quality = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(quality)
    default, offline, calibration = [json.loads(path.read_text()) for path in
                                     (args.default_output, args.offline_output, args.calibration)]
    quality.load_reference(calibration)
    if default["preset"] != "default" or offline["preset"] != "offline":
        parser.error("requires independent default and offline preset outputs")
    if any(default[key] != offline[key] for key in
           ("duration_ms", "audio_sha256", "sortformer_sha256", "embedding_sha256")):
        parser.error("both presets must use the same audio and models")
    if calibration.get("source_audio_sha256") != default["audio_sha256"]:
        parser.error("calibration must be tied to this exact decoded recording")
    duration = default["duration_ms"]
    mappings = [quality.score(calibration, value["raw"]["turns"])["hypothesis_to_reference"]
                for value in (default, offline)]
    accepted, unavailable = consensus(default, offline, mappings, duration)
    unavailable_ms = {}
    for interval in unavailable:
        reason = interval["reason"]
        unavailable_ms[reason] = unavailable_ms.get(reason, 0) + interval["end_ms"] - interval["start_ms"]
    result = {
        "schema_version": 1,
        "observation_origin": "model_derived_sortformer_timeline_not_captured_teams_evidence",
        "reference_role": "Diagnostic model-derived consensus only. No historical Teams AX or highlights were captured. Outside exact frozen user-labeled calibration intervals, names and speaking activity are model predictions, not human truth.",
        "physical_source": "system_audio", "duration_ms": duration,
        "source_audio_sha256": digest(args.source_audio),
        "decoded_wav_sha256": default["audio_sha256"],
        "source_fingerprints": {"default_output_sha256": digest(args.default_output),
                                "offline_output_sha256": digest(args.offline_output),
                                "calibration_sha256": digest(args.calibration),
                                "timeline_builder_sha256": digest(Path(__file__)),
                                "mapping_scorer_sha256": digest(root / "scripts/speaker-quality-evaluate.py")},
        "model_provenance": {"sortformer_sha256": default["sortformer_sha256"],
                             "embedding_sha256": default["embedding_sha256"],
                             "presets": ["default", "offline"],
                             "capability_max_speakers": 4,
                             "configured_speaker_count": None},
        "mapping_policy": "For each preset independently, learn one globally optimal one-to-one mapping only from this recording's frozen calibration. Do not remap held-out or individual intervals.",
        "frozen_preset_mappings": {"default": mappings[0], "offline": mappings[1]},
        "turns": accepted, "unavailable_intervals": unavailable,
        "unavailable_ms_by_reason": unavailable_ms,
        "consensus_name_ms": sum(turn["end_ms"] - turn["start_ms"] for turn in accepted),
        "samples_captured_from_teams": 0, "app_or_voice_enrollment_performed": False,
        "audio_uploaded": False,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    with open(output, "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as target:
        json.dump(result, target, indent=2)
        target.write("\n")
    print(json.dumps({"consensus_name_ms": result["consensus_name_ms"],
                      "intervals": len(accepted), "unavailable_ms_by_reason": unavailable_ms,
                      "captured_teams_samples": 0}))


if __name__ == "__main__":
    main()
