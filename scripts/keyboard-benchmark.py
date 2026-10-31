#!/usr/bin/env python3
"""Validate authored synthetic tasks and summarize explicit offline human trials.

This tool never records keyboard input, audio, or network traffic. Supplied
durations describe finished-message tasks, not physical key or haptic latency.
"""

import argparse
from collections import Counter, defaultdict
import csv
import json
import math
from pathlib import Path
import re
import statistics
import sys
import unicodedata


DEFAULT_CORPUS = Path(__file__).resolve().parents[1] / "docs/evidence/keyboard-benchmark-corpus.json"
REQUIRED = ("trial_id", "participant", "keyboard", "device", "os", "task_id",
            "elapsed_seconds", "corrected_errors", "harmful_corrections")
LANGUAGES = ("nl", "en", "mixed")
MODES = ("typing", "dictation")
RATINGS = ("immediacy", "comfort", "annoyance", "fatigue")
OPTIONAL = ("final_text", *RATINGS)


class BenchmarkError(ValueError):
    pass


def word_tokens(text):
    normalized = unicodedata.normalize("NFC", text).casefold().replace("’", "'")
    return re.findall(r"\w+(?:'\w+)*", normalized)


def distance(left, right):
    if len(left) < len(right):
        left, right = right, left
    previous = list(range(len(right) + 1))
    for row, item in enumerate(left, 1):
        current = [row]
        for column, other in enumerate(right, 1):
            current.append(min(current[-1] + 1, previous[column] + 1, previous[column - 1] + (item != other)))
        previous = current
    return previous[-1]


def validate_corpus(data):
    if not isinstance(data, dict) or data.get("schema_version") != 1 or data.get("synthetic") is not True:
        raise BenchmarkError("Corpus must explicitly identify schema 1 synthetic tasks.")
    if data.get("kind") != "synthetic_keyboard_benchmark" or data.get("human_benchmark_required") is not True:
        raise BenchmarkError("Corpus must distinguish synthetic tasks from human benchmark evidence.")
    entries = data.get("tasks")
    if not isinstance(entries, list) or len(entries) != 90:
        raise BenchmarkError("Corpus must contain exactly 90 tasks.")
    tasks, themes = {}, defaultdict(set)
    for task in entries:
        if not isinstance(task, dict) or task.get("language") not in LANGUAGES:
            raise BenchmarkError("Corpus task has an invalid language.")
        identifier, language = task.get("id"), task["language"]
        if not isinstance(identifier, str) or not re.fullmatch(re.escape(language) + r"-\d{3}", identifier) or identifier in tasks:
            raise BenchmarkError("Corpus has an invalid or duplicate task ID.")
        if task.get("mode") not in MODES or not isinstance(task.get("theme"), str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,63}", task["theme"]):
            raise BenchmarkError("Corpus task needs a mode and aligned theme.")
        for field in ("source_text", "reference_text"):
            text = task.get(field)
            if not isinstance(text, str) or not text.strip() or len(text) > 1024 or "\0" in text:
                raise BenchmarkError("Corpus task text must be nonempty and bounded.")
        if not word_tokens(task["reference_text"]):
            raise BenchmarkError("Corpus references must contain words.")
        tasks[identifier] = task
        themes[language].add(task["theme"])
    if any(sum(task["language"] == language for task in tasks.values()) != 30 or len(themes[language]) != 30 for language in LANGUAGES):
        raise BenchmarkError("Corpus must contain 30 distinct themes per language.")
    if not themes["nl"] == themes["en"] == themes["mixed"]:
        raise BenchmarkError("Corpus themes must align across all three language slices.")
    return tasks


def load_corpus(path):
    try:
        return validate_corpus(json.loads(Path(path).read_text(encoding="utf-8")))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise BenchmarkError("Cannot read a valid UTF-8 JSON corpus.") from error


def read_trials(path, tasks):
    trials, identifiers = [], set()
    try:
        with Path(path).open(newline="", encoding="utf-8-sig") as source:
            reader = csv.DictReader(source)
            fields = reader.fieldnames or []
            if len(fields) != len(set(fields)) or not set(REQUIRED) <= set(fields) or set(fields) - set(REQUIRED) - set(OPTIONAL):
                raise BenchmarkError("CSV requires the template headers, with only supported optional text and manual-rating columns.")
            for number, row in enumerate(reader, 2):
                if None in row or any(value is None for value in row.values()):
                    raise BenchmarkError(f"CSV row {number} has missing or extra cells.")
                trial = {}
                for field in ("trial_id", "participant", "keyboard", "device", "os"):
                    value = row[field].strip()
                    pattern = r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}" if field in ("trial_id", "participant") else r"[A-Za-z0-9][A-Za-z0-9 ._+()-]{0,79}"
                    if not re.fullmatch(pattern, value):
                        raise BenchmarkError(f"CSV row {number} has an invalid {field} label; use pseudonymous identifiers.")
                    trial[field] = value.casefold()
                if trial["trial_id"] in identifiers:
                    raise BenchmarkError(f"CSV row {number} repeats a trial ID.")
                identifiers.add(trial["trial_id"])
                task_id = row["task_id"].strip()
                if task_id not in tasks:
                    raise BenchmarkError(f"CSV row {number} has an unknown synthetic task ID.")
                trial["task_id"] = task_id
                try:
                    elapsed = float(row["elapsed_seconds"])
                except ValueError as error:
                    raise BenchmarkError(f"CSV row {number} has an invalid elapsed duration.") from error
                if not math.isfinite(elapsed) or elapsed <= 0:
                    raise BenchmarkError(f"CSV row {number} needs a finite positive elapsed duration.")
                trial["elapsed_seconds"] = elapsed
                for field in ("corrected_errors", "harmful_corrections"):
                    if not re.fullmatch(r"[0-9]+", row[field].strip()):
                        raise BenchmarkError(f"CSV row {number} needs a nonnegative integer {field} count.")
                    try:
                        trial[field] = int(row[field])
                    except ValueError as error:
                        raise BenchmarkError(f"CSV row {number} has an invalid error count.") from error
                if "final_text" in fields:
                    if len(row["final_text"]) > 4096 or "\0" in row["final_text"]:
                        raise BenchmarkError(f"CSV row {number} has an oversized or invalid synthetic output.")
                    trial["final_text"] = row["final_text"]
                for field in RATINGS:
                    value = row.get(field, "").strip()
                    if not value:
                        continue
                    try:
                        rating = float(value)
                    except ValueError as error:
                        raise BenchmarkError(f"CSV row {number} has an invalid {field} rating.") from error
                    if not math.isfinite(rating) or not 1 <= rating <= 7:
                        raise BenchmarkError(f"CSV row {number} needs a finite {field} rating from 1 to 7.")
                    trial[field] = rating
                trials.append(trial)
    except (OSError, UnicodeError, csv.Error) as error:
        raise BenchmarkError("Cannot read a valid UTF-8 CSV results file.") from error
    if not trials:
        raise BenchmarkError("Results contain no trial rows.")
    return trials


def rate(numerator, denominator):
    if not denominator:
        return None
    try:
        value = numerator / denominator
        if not math.isfinite(value):
            raise BenchmarkError("Measurements exceed the supported numeric range.")
        return round(value, 6)
    except OverflowError as error:
        raise BenchmarkError("Measurements exceed the supported numeric range.") from error


def group_summary(keyboard, language, trials, tasks):
    words = sum(len(word_tokens(tasks[row["task_id"]]["reference_text"])) for row in trials)
    word_errors = character_errors = scored_words = scored_characters = scored = 0
    for row in trials:
        if "final_text" not in row:
            continue
        reference = unicodedata.normalize("NFC", tasks[row["task_id"]]["reference_text"])
        output = unicodedata.normalize("NFC", row["final_text"])
        reference_words = word_tokens(reference)
        word_errors += distance(reference_words, word_tokens(output))
        character_errors += distance(reference, output)
        scored_words += len(reference_words)
        scored_characters += len(reference)
        scored += 1
    harmful = sum(row["harmful_corrections"] for row in trials)
    modes = {tasks[row["task_id"]]["mode"] for row in trials}
    ratings = {}
    for field in RATINGS:
        values = [row[field] for row in trials if field in row]
        ratings[field] = {"count": len(values), "median": statistics.median(values) if values else None}
    return {"keyboard": keyboard, "language": language, "trials": len(trials),
            "mode": next(iter(modes)) if len(modes) == 1 else "mixed",
            "participants": len({row["participant"] for row in trials}),
            "distinct_tasks": len({row["task_id"] for row in trials}),
            "median_elapsed_seconds": statistics.median(row["elapsed_seconds"] for row in trials),
            "corrected_errors": sum(row["corrected_errors"] for row in trials),
            "harmful_corrections": harmful, "reference_words": words,
            "harmful_corrections_per_1000_reference_words": rate(1000 * harmful, words),
            "text_scored_trials": scored, "wer": rate(word_errors, scored_words),
            "cer": rate(character_errors, scored_characters), "subjective_ratings": ratings}


def _paired_for_mode(tasks, trials, apple_keyboard, candidates):
    cells = defaultdict(lambda: defaultdict(lambda: defaultdict(list)))
    for row in trials:
        context = (row["participant"], row["device"], row["os"], tasks[row["task_id"]]["language"])
        cells[context][row["keyboard"]][row["task_id"]].append(row)
    comparisons = []
    for keyboard in sorted(candidates):
        for language in LANGUAGES:
            contexts = [(key, value) for key, value in cells.items() if key[3] == language and (keyboard in value or apple_keyboard in value)]
            if not contexts:
                continue
            reasons = Counter()
            changes, participants = [], set()
            matched_sets = reference_words = candidate_harmful = apple_harmful = 0
            for context, keyboards in contexts:
                if apple_keyboard not in keyboards:
                    reasons["missing_apple"] += 1
                    continue
                if keyboard not in keyboards:
                    reasons["missing_candidate"] += 1
                    continue
                candidate, apple = keyboards[keyboard], keyboards[apple_keyboard]
                if candidate.keys() != apple.keys():
                    reasons["different_task_sets"] += 1
                    continue
                matched_sets += 1
                participants.add(context[0])
                for task_id in sorted(candidate):
                    candidate_time = statistics.median(row["elapsed_seconds"] for row in candidate[task_id])
                    apple_time = statistics.median(row["elapsed_seconds"] for row in apple[task_id])
                    changes.append(rate(100 * candidate_time, apple_time) - 100)
                    reference_words += len(word_tokens(tasks[task_id]["reference_text"]))
                    candidate_harmful += statistics.mean(row["harmful_corrections"] for row in candidate[task_id])
                    apple_harmful += statistics.mean(row["harmful_corrections"] for row in apple[task_id])
            comparisons.append({"keyboard": keyboard, "baseline": apple_keyboard, "language": language,
                                "matched_participant_device_os_sets": matched_sets, "matched_participants": len(participants),
                                "matched_task_pairs": len(changes), "unpaired_sets": sum(reasons.values()),
                                "unpaired_reasons": dict(sorted(reasons.items())),
                                "median_elapsed_change_percent": round(statistics.median(changes), 6) if changes else None,
                                "candidate_harmful_corrections_per_1000_reference_words": rate(1000 * candidate_harmful, reference_words),
                                "apple_harmful_corrections_per_1000_reference_words": rate(1000 * apple_harmful, reference_words),
                                "harmful_corrections_difference_per_1000_reference_words": rate(1000 * (candidate_harmful - apple_harmful), reference_words)})
    return comparisons


def paired_comparisons(tasks, trials, apple_keyboard, candidates=None):
    if candidates is None:
        candidates = {row["keyboard"] for row in trials} - {apple_keyboard}
    comparisons = []
    for mode in MODES:
        rows = [row for row in trials if tasks[row["task_id"]]["mode"] == mode]
        comparisons.extend({"mode": mode, **item} for item in _paired_for_mode(tasks, rows, apple_keyboard, candidates))
    return comparisons


def grouped_summaries(tasks, trials):
    groups = defaultdict(list)
    for row in trials:
        groups[(row["keyboard"], tasks[row["task_id"]]["language"])].append(row)
    return [group_summary(keyboard, language, rows, tasks) for (keyboard, language), rows in sorted(groups.items())]


def summarize(tasks, trials, apple_keyboard=None, mode="all"):
    if mode not in ("all", *MODES):
        raise BenchmarkError("Task mode must be all, typing or dictation.")
    supplied = len(trials)
    trials = [row for row in trials if mode == "all" or tasks[row["task_id"]]["mode"] == mode]
    if not trials:
        raise BenchmarkError("No observations match the requested task mode.")
    baseline = apple_keyboard.casefold() if apple_keyboard is not None else None
    candidates = {row["keyboard"] for row in trials} - {baseline}
    by_mode = {}
    for name in MODES:
        if mode not in ("all", name):
            continue
        rows = [row for row in trials if tasks[row["task_id"]]["mode"] == name]
        by_mode[name] = {"included_trials": len(rows), "groups": grouped_summaries(tasks, rows),
                         "paired_comparisons": paired_comparisons(tasks, rows, baseline, candidates) if baseline is not None else None}
    return {"schema_version": 1, "source": "explicit_offline_human_trial_csv", "synthetic_tasks": True,
            "supplied_trials": supplied, "included_trials": len(trials), "excluded_trials": supplied - len(trials),
            "mode_filter": mode, "contains_mixed_modes": len({tasks[row["task_id"]]["mode"] for row in trials}) > 1,
            "human_benchmark_required": True,
            "evidence_scope": "Descriptive synthetic-task results; no production acceptance or physical haptic latency is established.",
            "mode_policy": "Combined groups explicitly marked mixed are descriptive only. Assess typing and dictation separately using by_mode; paired percentages never combine modes.",
            "text_scoring": "WER uses NFC/casefolded word tokens, normalizes curly apostrophes and ignores punctuation. CER preserves case, punctuation and whitespace after NFC normalization.",
            "pairing_policy": "Exact participant/device/OS/language/mode and exact task-ID sets within that mode. Repeats use median duration and mean error count per task. Negative elapsed change is faster. No statistical significance is inferred.",
            "subjective_rating_scale": "Optional user-consented manual ratings from 1 to 7. Higher means more immediate, more comfortable, more annoying or more fatiguing respectively. Blank or absent ratings are not imputed.",
            "groups": grouped_summaries(tasks, trials), "by_mode": by_mode,
            "paired_comparisons": paired_comparisons(tasks, trials, baseline) if baseline is not None else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Examples:\n  python3 scripts/keyboard-benchmark.py validate\n  python3 scripts/keyboard-benchmark.py template --with-ratings > /tmp/keyboard-trials.csv\n  python3 scripts/keyboard-benchmark.py summarize /tmp/keyboard-trials.csv --mode typing --compare-apple\n\nOnly supply pseudonyms, synthetic task outputs and user-consented manual measurements. Optional immediacy/comfort/annoyance/fatigue ratings range from 1 to 7; leave missing ratings empty. If the optional final_text column exists, even empty cells are scored as empty outputs. Omit that column to omit WER/CER. No input is recorded and no network is used.")
    commands = parser.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("validate", help="Validate the 90 authored tasks; this runs no human trials.")
    validate.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    template = commands.add_parser("template", help="Print only CSV headers, without fabricated measurements.")
    template.add_argument("--with-ratings", action="store_true", help="Include optional 1–7 manual rating columns; blank means missing, never zero.")
    template.add_argument("--with-text", action="store_true", help="Include synthetic output text scoring; empty cells then count as empty outputs.")
    summary = commands.add_parser("summarize", help="Print an aggregate-only JSON report from explicit offline observations.")
    summary.add_argument("results", type=Path)
    summary.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    summary.add_argument("--mode", choices=("all", *MODES), default="all", help="Select a task mode. All keeps explicit per-mode breakdowns; comparisons never mix modes.")
    summary.add_argument("--compare-apple", action="store_true", help="Compare only matched complete task sets; report all unmatched coverage.")
    summary.add_argument("--apple-keyboard", default="apple", help="CSV keyboard label for Apple (default: apple); identifiers are case-insensitive.")
    args = parser.parse_args()
    try:
        if args.command == "template":
            fields = REQUIRED + (("final_text",) if args.with_text else ()) + (RATINGS if args.with_ratings else ())
            csv.writer(sys.stdout, lineterminator="\n").writerow(fields)
            return 0
        tasks = load_corpus(args.corpus)
        if args.command == "validate":
            report = {"source": "authored_synthetic_corpus", "task_count": len(tasks),
                      "languages": dict(Counter(task["language"] for task in tasks.values())),
                      "modes": dict(Counter(task["mode"] for task in tasks.values())),
                      "measured_trials": 0, "human_benchmark_required": True}
        else:
            report = summarize(tasks, read_trials(args.results, tasks), args.apple_keyboard if args.compare_apple else None, mode=args.mode)
        try:
            encoded = json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False)
        except (OverflowError, ValueError) as error:
            raise BenchmarkError("Measurements exceed the supported numeric range.") from error
        print(encoded)
        return 0
    except BenchmarkError as error:
        print(f"keyboard benchmark: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
