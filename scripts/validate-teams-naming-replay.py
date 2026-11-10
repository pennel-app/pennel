#!/usr/bin/env python3
"""Exercise shipped guarded naming against private real rows and explicitly authored observations.

No observation here was captured from Teams. Sparse user labels may establish safe abstention,
not live-reader accuracy or whole-call naming coverage. Run with --require-qualified for an
acceptance gate that remains red until real captured evidence and sufficient truth exist.
"""

import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import uuid


ORIGIN = "authored_fixture_not_captured_teams_evidence"
MODEL_ORIGIN = "model_derived_sortformer_timeline_not_captured_teams_evidence"
MARKER = "pennel-authored-teams-naming-replay-v1\n"
TEAMS = "com.microsoft.teams2"


def encoded(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(encoded(value))


def union(intervals):
    result = []
    for start, end in sorted(intervals):
        if end <= start:
            continue
        if result and start <= result[-1][1]:
            result[-1] = (result[-1][0], max(end, result[-1][1]))
        else:
            result.append((start, end))
    return result


def subtract(intervals, blockers):
    for start, end in union(blockers):
        intervals = [(a, min(b, start)) for a, b in intervals if a < start and b > a] + [
            (max(a, end), b) for a, b in intervals if b > end and b > a
        ]
    return union(intervals)


def overlap(a, b, c, d):
    return max(0, min(b, d) - max(a, c))


def canonical_clean_turns(rows, include_mixed=False):
    """Independent score projection: actual clipped turns, union duplicates, remove competitors."""
    observed, candidates = [], {}
    for row in rows:
        if (row["source_id"] != "system_audio" or row.get("failed", False)
                or not row.get("nonempty_text", True)):
            continue
        attribution = row["attribution"]
        for turn in attribution.get("turns", []):
            start = max(row["start_ms"], turn["start_ms"])
            end = min(row["end_ms"], turn["end_ms"])
            voice = turn["voice"]
            if start >= end:
                continue
            observed.append((voice, start, end))
            clean_row = (not attribution["provisional"] and not attribution["self_voice"]
                         and not row["manual"] and not row["unresolved_manual"])
            single = (attribution["kind"] == "single" and attribution["voices"] == [voice]
                      and row["voice"] == voice)
            acoustic = (include_mixed and attribution["kind"] in ["mixed", "overlap"]
                        and voice is not None and voice in attribution["voices"])
            if clean_row and (single or acoustic):
                candidates.setdefault(voice, []).append((start, end))
    return {voice: subtract(union(turns), [(a, b) for v, a, b in observed if v != voice])
            for voice, turns in candidates.items()}


def observation_fixture(recording, turns):
    samples = {0: {"at_ms": 0, "available": False, "speaking": []}}
    for turn in turns:
        start, end, speaker = turn["start_ms"], turn["end_ms"], turn["speaker"]
        for at in range(start + 400, end + 400, 500):
            samples[at] = {"at_ms": at, "available": True, "speaking": [speaker]}
        samples[end + 400] = {"at_ms": end + 400, "available": False, "speaking": []}
    return {
        "app_id": TEAMS, "availability": "available", "unavailable_reason": None,
        "duplicate_names": False,
        "participants": sorted({turn["speaker"] for turn in turns}),
        "samples": [samples[key] for key in sorted(samples)],
        "binding": {"recording_uid": recording["meeting_uid"],
                    "recording_started_at": recording["started_at"],
                    "process_id": 42, "process_started_at_ms": 1,
                    "window_id": "AUTHORED-ORACLE-FIXTURE-NOT-A-REAL-TEAMS-WINDOW"},
    }


def skeleton(profile, meeting_id, template_recording, template_settings, uid):
    profile.mkdir(mode=0o700, parents=True)
    (profile / ".teams-naming-replay").write_text(MARKER)
    library = profile / "Meetings"
    meeting = library / meeting_id
    (meeting / ".pennel").mkdir(parents=True)
    recording = copy.deepcopy(template_recording)
    recording["meeting_uid"] = uid
    save(meeting / ".pennel/recording.json", recording)
    (meeting / "notes.md").write_text("Owned naming validation notes.\n")
    (meeting / "meeting.md").write_text(
        "---\ntitle: Authored naming fixture\nstart: " + recording["started_at"]
        + "\nduration_min: 1\ntemplate: algemeen\npennel_id: " + uid + "\n---\n")
    settings = copy.deepcopy(template_settings)
    settings["meetings_dir"] = str(library.resolve())
    save(profile / "settings.json", settings)
    return meeting, recording


def timestamp(ms):
    seconds, millis = divmod(ms, 1000)
    hours, remainder = divmod(seconds, 3600)
    minutes, seconds = divmod(remainder, 60)
    result = f"{hours}:{minutes:02}:{seconds:02}" if hours else f"{minutes}:{seconds:02}"
    return result + (f".{millis:03}" if millis else "")


def write_authored_rows(meeting, uid, rows):
    metadata, transcript = [], []
    for index, row in enumerate(sorted(rows, key=lambda row: row["start"])):
        start, end, voice = row["start"], row["end"], row["voice"]
        source, text = row.get("source", "system_audio"), f"Authored fixture turn {index}."
        text_hash = digest(text.encode())
        fingerprint = digest(encoded([source, start, end, text_hash]))
        voice_text = f"Anderen {voice}" if voice else "Anderen"
        transcript.append(f"[{timestamp(start)}] **{voice_text}:** {text}\n")
        item = {
            "utterance_id": str(uuid.uuid5(uuid.UUID(uid), f"row-{index}")),
            "source_id": source, "start_ms": start, "end_ms": end,
            "text_fingerprint": text_hash, "source_fingerprint": fingerprint, "spans": [],
            "attribution_revision": 1, "lineage": row.get("lineage", []), "manual": None,
            "attribution": {"kind": row.get("kind", "single"),
                            "voices": row.get("voices", [voice] if voice else []), "self_voice": False,
                            "provisional": row.get("provisional", False),
                            "turns": row.get("turns", [{"start_ms": start, "end_ms": end,
                                                       "voice": voice}])},
        }
        if row.get("manual"):
            item["manual"] = {"name": "Retained correction", "voice": voice,
                              "source_fingerprint": fingerprint,
                              "review_required": True, "self_voice": False}
        metadata.append(item)
    transcript_bytes, metadata_bytes = "".join(transcript).encode(), encoded(metadata)
    (meeting / "transcript.md").write_bytes(transcript_bytes)
    part_hash = digest(metadata_bytes)
    (meeting / f"transcript.meta.{part_hash}.json").write_bytes(metadata_bytes)
    save(meeting / "transcript.meta.json", {
        "schema_version": 5, "meeting_uid": uid, "transcript_sha256": digest(transcript_bytes),
        "revision": 1, "sources": sorted({row.get("source", "system_audio") for row in rows}),
        "parts": [{"sha256": part_hash, "rows": len(rows)}],
    })


def run(binary, profile, meeting_id, scenario, stem):
    scenario.setdefault("observation_origin", ORIGIN)
    scenario_file, receipt = profile / f"{stem}-scenario.json", profile / f"{stem}-result.json"
    save(scenario_file, scenario)
    result = subprocess.run([str(binary), str(profile), meeting_id, str(scenario_file), str(receipt)],
                            capture_output=True, text=True)
    if result.returncode:
        raise AssertionError(f"Guarded core naming replay failed in {profile.name}/{stem}: "
                             + result.stderr[-1200:])
    return json.loads(receipt.read_bytes())


def check(receipt, names, statuses=None, error=False):
    actual = {(name["voice"], name["name"]) for name in receipt["names"]}
    assert actual == set(names), f"Unexpected automatic/manual names: {actual!r}"
    assert bool(receipt["error"]) == error, "Unexpected guarded commit outcome"
    if statuses:
        assert receipt["applied"]["status"] in statuses, receipt["applied"]["status"]
    for field in ["words_sources_attribution_and_manual_work_preserved", "notes_preserved",
                  "voice_enrollment_unchanged", "provenance_valid", "idempotent"]:
        assert receipt[field], field


def controls(binary, output, template, template_settings):
    positive = [{"start": start, "end": start + 4000, "voice": 1} for start in [0, 6000, 12000]]
    positive.append({"start": 0, "end": 16000, "voice": 2, "source": "microphone"})
    results = []

    def case(name, modify=None, expected=(), statuses=None, error=False):
        uid = str(uuid.uuid5(uuid.NAMESPACE_URL, "pennel-authored-naming-" + name))
        profile, meeting_id = output / name, "Owned naming fixture"
        meeting, recording = skeleton(profile, meeting_id, template, template_settings, uid)
        rows = copy.deepcopy(positive)
        evidence = observation_fixture(recording, [{"start_ms": 0, "end_ms": 20000,
                                                   "speaker": "Fixture participant"}])
        scenario = {"evidence": evidence, "enabled": True, "policy": "automatic"}
        if modify:
            modify(rows, scenario)
        write_authored_rows(meeting, uid, rows)
        if error:
            path = meeting / "transcript.md"
            path.write_text(path.read_text().replace("Authored fixture turn 0.", "Changed source."))
        receipt = run(binary, profile, meeting_id, scenario, "initial")
        check(receipt, expected, statuses, error)
        results.append({"case": name, "passed": True,
                        "status": receipt["applied"]["status"] if receipt["applied"] else "refused",
                        "automatic_names": len(receipt["names"]), "receipt": str(profile / "initial-result.json")})
        return profile, meeting_id, scenario, receipt

    profile, meeting_id, scenario, receipt = case(
        "positive", expected=[(1, "Fixture participant")], statuses=["applied"])
    assert sum(bool(row["automatic_name"]) for row in receipt["rows"]) == 3
    scenario["enabled"] = False
    check(run(binary, profile, meeting_id, scenario, "opt-out-after-applied"), [], ["disabled"])
    results.append({"case": "opt-out-after-applied", "passed": True})

    profile, meeting_id, scenario, _ = case(
        "evidence-replaced-after-auto", expected=[(1, "Fixture participant")], statuses=["applied"])
    scenario["evidence"].update(availability="unavailable", unavailable_reason="call_changed")
    receipt = run(binary, profile, meeting_id, scenario, "stale-evidence-retracted")
    check(receipt, [], ["unavailable"])
    assert not any(row["automatic_name"] for row in receipt["rows"])
    results.append({"case": "stale-evidence-retracted", "passed": True})

    case("opt-out-before", lambda rows, s: s.update(enabled=False), statuses=["disabled"])
    case("suggestions-only", lambda rows, s: s.update(policy="suggest"), statuses=["suggested"])
    case("permission-loss", lambda rows, s: s["evidence"].update(availability="permission_denied"),
         statuses=["permission_denied"])
    case("unbound", lambda rows, s: s["evidence"].update(binding=None), statuses=["unbound"])
    case("other-recording", lambda rows, s: s["evidence"]["binding"].update(
        recording_uid=str(uuid.uuid5(uuid.NAMESPACE_URL, "another-recording"))), statuses=["unbound"])
    case("stale-start", lambda rows, s: s["evidence"]["binding"].update(
        recording_started_at="2020-01-01T00:00:00Z"), statuses=["unbound"])
    case("changed-call", lambda rows, s: s["evidence"].update(
        availability="unavailable", unavailable_reason="call_changed"), statuses=["unavailable"])
    case("unsupported-app", lambda rows, s: s["evidence"].update(app_id="us.zoom.xos"),
         statuses=["unsupported"])
    case("duplicate-names", lambda rows, s: s["evidence"].update(
        participants=["Fixture participant", "fixture PARTICIPANT"]), statuses=["conflict"])

    def competing(rows, scenario):
        for sample in scenario["evidence"]["samples"]:
            if sample["at_ms"] % 1000 == 400:
                sample["speaking"] = ["Conflicting participant"]
    case("conflicting-highlights", competing)
    case("multiple-highlights", lambda rows, s: [sample.update(speaking=["A", "B"])
         for sample in s["evidence"]["samples"]])

    # All case modifiers receive the already bound evidence; retain its binding for long fixtures.
    def coarse_bound(rows, scenario):
        binding = scenario["evidence"]["binding"]
        rows[:] = [{"start": start, "end": start + 15000, "voice": 1,
                    "turns": [{"start_ms": start, "end_ms": start + 1000, "voice": 1}]}
                   for start in [0, 30000, 60000]]
        scenario["evidence"]["samples"] = observation_fixture(template, [
            {"start_ms": 0, "end_ms": 80000, "speaker": "Fixture participant"}])["samples"]
        scenario["evidence"]["binding"] = binding
    case("coarse-silence-does-not-count", coarse_bound)
    case("missing-acoustic-provenance", lambda rows, s: [row.update(turns=[]) for row in rows])
    lineage = str(uuid.uuid5(uuid.NAMESPACE_URL, "one-asr-lineage"))
    case("one-lineage-split-three-ways", lambda rows, s: [row.update(lineage=[lineage]) for row in rows])
    case("provisional-rows", lambda rows, s: [row.update(provisional=True) for row in rows])
    case("room-source", lambda rows, s: [row.update(source="room_microphone") for row in rows])
    case("source-identity-reuse", lambda rows, s: rows[-1].update(voice=1), statuses=["conflict"])
    case("manual-review-survives", lambda rows, s: rows[0].update(manual=True), statuses=["conflict"])
    case("all-speech-overlapped", lambda rows, s: rows.extend(
        [{"start": start, "end": start + 4000, "voice": None, "kind": "overlap"}
         for start in [0, 6000, 12000]]))
    case("genuine-overlap-excluded-from-evidence", lambda rows, s: rows.append(
        {"start": 0, "end": 1000, "voice": 3}), expected=[(1, "Fixture participant")],
        statuses=["applied"])

    def mixed_support(rows, scenario, target=True):
        rows[:] = [{"start": start, "end": start + 15000, "voice": None,
                    "kind": "mixed", "voices": [1, 3],
                    "turns": [{"start_ms": start, "end_ms": start + 4000, "voice": 1},
                              {"start_ms": start + 6000, "end_ms": start + 10000, "voice": 3}]}
                   for start in [0, 20000, 40000]]
        if target:
            rows.append({"start": 60000, "end": 60500, "voice": 1})
        turns = []
        for start in [0, 20000, 40000]:
            turns.extend([{"start_ms": start, "end_ms": start + 4000,
                           "speaker": "Fixture participant"},
                          {"start_ms": start + 6000, "end_ms": start + 10000,
                           "speaker": "Other participant"}])
        binding = scenario["evidence"]["binding"]
        scenario["evidence"] = observation_fixture(template, turns)
        scenario["evidence"]["binding"] = binding
    case("mixed-acoustic-learning-keeps-mixed-words", mixed_support,
         expected=[(1, "Fixture participant")], statuses=["applied"])
    case("all-mixed-without-single-proof-target", lambda rows, s: mixed_support(rows, s, False))
    case("low-coverage", lambda rows, s: s["evidence"].update(samples=
        observation_fixture(template, [{"start_ms": 0, "end_ms": 4000,
                                         "speaker": "Fixture participant"}])["samples"]))

    def shared_device(rows, scenario):
        rows.extend({"start": start, "end": start + 4000, "voice": 3}
                    for start in [30000, 36000, 42000])
        scenario["evidence"]["samples"] = observation_fixture(template, [
            {"start_ms": 0, "end_ms": 50000, "speaker": "Fixture participant"}])["samples"]
    case("shared-device-without-comparable-prints", shared_device, statuses=["conflict"])
    case("stale-transcript-metadata", error=True)
    profile, meeting_id, scenario, _ = case(
        "manual-after-auto", expected=[(1, "Fixture participant")], statuses=["applied"])
    scenario["manual_rename"] = {"voice": 1, "name": "User corrected"}
    receipt = run(binary, profile, meeting_id, scenario, "manual-override")
    check(receipt, [(1, "User corrected")])
    assert not any(row["automatic_name"] for row in receipt["rows"])
    results.append({"case": "manual-override-after-automatic", "passed": True})
    return results


def clone_real_profile(source, output, meeting_id):
    if (source / ".speaker-quality-fixture").read_text() != "pennel-speaker-quality-fixture-v1\n":
        raise ValueError("Real-row source must already be an owned speaker-quality fixture")
    source_meeting = source / "Meetings" / meeting_id
    manifest = json.loads((source_meeting / "transcript.meta.json").read_bytes())
    selected = ["meeting.md", "notes.md", "transcript.md", "transcript.meta.json",
                ".pennel/recording.json"]
    selected += [f'transcript.meta.{part["sha256"]}.json' for part in manifest["parts"]]
    before = {name: sha(source_meeting / name) for name in selected}
    output.mkdir(mode=0o700)
    (output / ".teams-naming-replay").write_text(MARKER)
    meeting = output / "Meetings" / meeting_id
    for name in selected:
        path = meeting / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source_meeting / name, path)
    settings = json.loads((source / "settings.json").read_bytes())
    settings["meetings_dir"] = str((output / "Meetings").resolve())
    save(output / "settings.json", settings)
    voices = source / "voices/meetings"
    if voices.exists():
        shutil.copytree(voices, output / "voices/meetings")
    return source_meeting, before


def meeting_id_for_profile(profile, selected=None):
    library = profile / "Meetings"
    if not library.is_dir():
        raise ValueError("The source profile has no Meetings folder.")
    candidates = sorted(path.name for path in library.iterdir()
                        if path.is_dir() and not path.name.startswith(".")
                        and (path / "meeting.md").is_file())
    if selected is not None:
        if selected not in candidates:
            raise ValueError("The selected meeting must be a visible meeting folder in its source profile.")
        return selected
    if len(candidates) != 1:
        raise ValueError("Expected exactly one visible meeting folder; use --meeting-1002 or --meeting-0930 to select one.")
    return candidates[0]


def model_mapped_metrics(clean, names, timeline):
    model_turns = timeline["turns"]
    mapped_ms = sum(end - start for start, end in union(
        [(turn["start_ms"], turn["end_ms"]) for turn in model_turns]))
    eligible_ms = sum(end - start for turns in clean.values() for start, end in turns)
    named_ms = sum(end - start for voice, turns in clean.items()
                   if voice in names for start, end in turns)
    model_covered_ms = named_mapped_ms = matching_ms = 0
    per_voice = {}
    for voice, intervals in clean.items():
        tally = {"eligible_ms": sum(b - a for a, b in intervals), "model_mapped_ms": 0,
                 "automatic_name": names.get(voice), "by_model_name_ms": {}}
        for turn in model_turns:
            support = sum(overlap(a, b, turn["start_ms"], turn["end_ms"]) for a, b in intervals)
            tally["by_model_name_ms"][turn["speaker"]] = (
                tally["by_model_name_ms"].get(turn["speaker"], 0) + support)
            tally["model_mapped_ms"] += support
            model_covered_ms += support
            if voice in names:
                named_mapped_ms += support
                if names[voice] == turn["speaker"]:
                    matching_ms += support
        per_voice[voice] = tally
    return {
        "comparison_is_model_agreement_not_human_ground_truth": True,
        "model_mapped_ms": mapped_ms, "canonical_clean_eligible_ms": eligible_ms,
        "canonical_clean_model_covered_ms": model_covered_ms,
        "automatically_named_eligible_ms": named_ms,
        "automatically_named_model_mapped_ms": named_mapped_ms,
        "matching_model_name_ms": matching_ms,
        "named_share_of_canonical_clean_speech": named_ms / eligible_ms if eligible_ms else None,
        "named_share_of_all_model_mapped_speech": named_mapped_ms / mapped_ms if mapped_ms else None,
        "name_agreement_where_both_have_labels": matching_ms / named_mapped_ms if named_mapped_ms else None,
        "per_voice": per_voice,
    }


def single_row_text_metrics(rows, names):
    total = named = mixed = unresolved = 0
    for row in rows:
        if row["source_id"] != "system_audio" or row["failed"]:
            continue
        words = row["recognized_word_count"]
        total += words
        attribution = row["attribution"]
        if attribution["kind"] in ["mixed", "overlap"]:
            mixed += words
        if row["unresolved_manual"] or attribution["provisional"]:
            unresolved += words
        if (attribution["kind"] == "single" and row["voice"] in names
                and row["automatic_name"] and not attribution["provisional"]):
            named += words
    return {"unit": "whitespace words; no word-level alignment is inferred",
            "all_recognized_system_words": total,
            "automatically_named_single_row_words": named,
            "mixed_or_overlap_words_preserved_without_one_speaker": mixed,
            "provisional_or_unresolved_words": unresolved,
            "named_single_row_word_share": named / total if total else None}


def real_replay(binary, source, output, meeting_id, reference_path, timeline_path=None):
    source_meeting, source_hashes = clone_real_profile(source, output, meeting_id)
    reference = json.loads(reference_path.read_bytes())
    recording = json.loads((source_meeting / ".pennel/recording.json").read_bytes())
    timeline = json.loads(timeline_path.read_bytes()) if timeline_path else reference
    origin = MODEL_ORIGIN if timeline_path else ORIGIN
    physical_system = source_meeting / "audio/system.m4a"
    physical_sha256 = sha(physical_system) if physical_system.is_file() else None
    if timeline_path:
        assert timeline["observation_origin"] == MODEL_ORIGIN
        assert timeline["decoded_wav_sha256"] == reference["source_audio_sha256"]
        if physical_sha256:
            assert timeline["source_audio_sha256"] == physical_sha256
        ordered = sorted(timeline["turns"], key=lambda turn: (turn["start_ms"], turn["end_ms"]))
        assert all(turn["start_ms"] < turn["end_ms"] for turn in ordered)
        assert all(a["end_ms"] <= b["start_ms"] for a, b in zip(ordered, ordered[1:])), "Model observation timeline must exclude overlap"
    stem = "model-derived-sortformer" if timeline_path else "oracle-exact-confirmed-intervals"
    receipt = run(binary, output, meeting_id, {
        "enabled": True, "policy": "automatic",
        "observation_origin": origin,
        "evidence": observation_fixture(recording, timeline["turns"]),
    }, stem)
    assert not receipt["error"] and receipt["metadata_status"] == "valid"
    assert source_hashes == {name: sha(source_meeting / name) for name in source_hashes}
    clean = canonical_clean_turns(receipt["rows"], include_mixed=True)
    single_clean = canonical_clean_turns(receipt["rows"])
    automatic_voices = {row["voice"] for row in receipt["rows"] if row["automatic_name"]}
    names = {name["voice"]: name["name"] for name in receipt["names"]
             if name["voice"] in automatic_voices}
    stats = {}
    for turn in reference["turns"]:
        person = stats.setdefault(turn["speaker"], {"reference_window_ms": 0, "eligible_speech_ms": 0,
                                                   "correctly_named_ms": 0, "wrongly_named_ms": 0,
                                                   "correctly_named_single_row_speech_ms": 0})
        person["reference_window_ms"] += turn["end_ms"] - turn["start_ms"]
        for voice, intervals in clean.items():
            eligible = sum(overlap(a, b, turn["start_ms"], turn["end_ms"]) for a, b in intervals)
            person["eligible_speech_ms"] += eligible
            if voice in names:
                person["correctly_named_ms" if names[voice] == turn["speaker"] else "wrongly_named_ms"] += eligible
                if names[voice] == turn["speaker"]:
                    person["correctly_named_single_row_speech_ms"] += sum(
                        overlap(a, b, turn["start_ms"], turn["end_ms"])
                        for a, b in single_clean.get(voice, []))
    for person in stats.values():
        assert person["wrongly_named_ms"] == 0, "Wrong automatic name on independently confirmed speech"
        person["named_coverage"] = (person["correctly_named_ms"] / person["eligible_speech_ms"]
                                    if person["eligible_speech_ms"] else None)
        person["named_acoustic_share_of_reference_window"] = person["correctly_named_ms"] / person["reference_window_ms"]
        person["named_single_row_speech_share_of_reference_window"] = person["correctly_named_single_row_speech_ms"] / person["reference_window_ms"]
    return {"source_profile": str(source), "reference_sha256": sha(reference_path),
            "source_artifacts_sha256": source_hashes, "source_unchanged": True,
            "decoded_reference_wav_sha256": reference["source_audio_sha256"],
            "retained_physical_system_m4a_sha256": physical_sha256,
            "observation_origin": origin, "live_teams_capture_validated": False,
            "status": receipt["applied"]["status"],
            "automatic_names": [{"voice": voice, "name": name} for voice, name in names.items()],
            "correctness_when_abstaining_is_not_a_precision_measure": True,
            "per_person": stats,
            "single_row_text_metrics": single_row_text_metrics(receipt["rows"], names),
            "model_timeline_path": str(timeline_path) if timeline_path else None,
            "model_reference_role": timeline.get("reference_role") if timeline_path else None,
            "model_timeline_sha256": sha(timeline_path) if timeline_path else None,
            "acoustic_model_mapped_metrics": model_mapped_metrics(clean, names, timeline) if timeline_path else None,
            "single_row_model_mapped_metrics": model_mapped_metrics(single_clean, names, timeline) if timeline_path else None,
            "automatic_names_qualified": False,
            "qualification_reason": ("Model-derived observations are a fusion diagnostic; no historic Teams capture or dense human truth"
                                     if timeline_path else "No saved Teams observations; exact user-labeled windows are too sparse for whole-call naming gates"),
            "receipt": str(output / f"{stem}-result.json")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--private-root", type=Path, default=Path("build/speaker-quality-private"))
    parser.add_argument("--profile-1002", type=Path)
    parser.add_argument("--profile-0930", type=Path)
    parser.add_argument("--meeting-1002", metavar="NAME", help="Select a meeting folder in the first profile")
    parser.add_argument("--meeting-0930", metavar="NAME", help="Select a meeting folder in the second profile")
    parser.add_argument("--derived-1002", type=Path, help="Explicitly model-derived Sortformer timeline JSON")
    parser.add_argument("--derived-0930", type=Path, help="Explicitly model-derived Sortformer timeline JSON")
    parser.add_argument("--require-qualified", action="store_true")
    args = parser.parse_args()
    root = args.private_root.resolve()
    binary = args.binary.resolve(strict=True)
    source_1002 = (args.profile_1002 or root / "quality-replays/1002").resolve()
    source_0930 = (args.profile_0930 or root / "quality-replays/0930").resolve()
    try:
        meeting_1002 = meeting_id_for_profile(source_1002, args.meeting_1002)
        meeting_0930 = meeting_id_for_profile(source_0930, args.meeting_0930)
    except ValueError as error:
        parser.error(str(error))
    output = Path(tempfile.mkdtemp(prefix="teams-naming-replay-", dir=root))
    retained_binary = output / "teams_naming_replay"
    shutil.copyfile(binary, retained_binary)
    retained_binary.chmod(0o500)
    binary = retained_binary
    template = json.loads((source_1002 / "Meetings" / meeting_1002 / ".pennel/recording.json").read_bytes())
    template_settings = json.loads((source_1002 / "settings.json").read_bytes())
    control_results = controls(binary, output, template, template_settings)
    recordings = {
        "1002": real_replay(binary, source_1002, output / "real-1002", meeting_1002,
                            root / "acoustic-evaluation/anchor-analysis/frozen-user-reference/combined-abcdef.json"),
        "0930": real_replay(binary, source_0930, output / "real-0930", meeting_0930,
                            root / "acoustic-evaluation-0930/frozen-user-reference/independently-confirmed-ghij.json"),
    }
    model_replays = {}
    for key, source, meeting_id, timeline in [("1002", source_1002, meeting_1002, args.derived_1002),
                                             ("0930", source_0930, meeting_0930, args.derived_0930)]:
        if timeline:
            reference_path = (root / "acoustic-evaluation/anchor-analysis/frozen-user-reference/combined-abcdef.json"
                              if key == "1002" else root / "acoustic-evaluation-0930/frozen-user-reference/independently-confirmed-ghij.json")
            model_replays[key] = real_replay(binary, source, output / f"model-derived-{key}",
                                             meeting_id, reference_path,
                                             timeline.resolve(strict=True))
    report = {"schema_version": 1, "core_binary_sha256": sha(binary),
              "retained_core_binary": str(binary),
              "validator_sha256": sha(Path(__file__).resolve()),
              "replay_example_sha256": sha(Path(__file__).resolve().parents[1] / "core/examples/teams_naming_replay.rs"),
              "authored_controls": control_results, "recordings": recordings,
              "model_derived_replays": model_replays,
              "live_teams_capture_validated": False, "automatic_names_qualified": False}
    save(output / "report.json", report)
    print(f"Passed {len(control_results)} guarded-API controls; replayed both real canonical transcripts.")
    print(f"Private report: {output / 'report.json'}")
    if args.require_qualified:
        raise SystemExit("Acceptance remains open: authored observations cannot qualify live Teams names.")


if __name__ == "__main__":
    main()
