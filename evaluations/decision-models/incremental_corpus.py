"""Owned temporal fixtures; gold is authored at each cutoff, not borrowed from a later turn."""

from collections import Counter
import hashlib
import json


LANGUAGES = ("en", "nl", "mixed")
SPLITS = ("development", "calibration", "heldout")
USE_CASES = ("manual_finished_suggestions", "manual_live_suggestions", "source_assessment", "background_live_suggestions")
PREFIXES = (0, 1, 2, 3, 4, 6, 8, 10)
WHITE_SPACE = frozenset(chr(c) for c in [*range(9, 14), 32, 133, 160, 5760, *range(8192, 8203), 8232, 8233, 8239, 8287, 12288])


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def normalized(text):
    spaced = "".join(" " if char in WHITE_SPACE else char for char in text)
    return " ".join(part for part in spaced.split(" ") if part)


def source_fingerprint(source):
    return digest([source["utterance_id"], source.get("source_id"), source["start_ms"], source["end_ms"],
                   "" if source.get("failed") else normalized(source["text"]), bool(source.get("failed", False))])


def _lines(language, topic, owner, alternate, unknown, no_due, retract, retract_decision, answer):
    due = {"en": "on Thursday", "nl": "donderdag", "mixed": "on donderdag"}[language]
    actor = {"en": "Someone", "nl": "Iemand", "mixed": "Someone"}[language] if unknown else owner
    deadline = "" if no_due else " " + due
    if language == "nl":
        lines = [f"Misschien kunnen we de {topic} versturen; dit is nog geen toezegging.",
                 f"{actor} zal de {topic}{deadline} versturen. Dit is toegezegd.",
                 f"Wijziging: {alternate} neemt het versturen van de {topic} over; de eerdere eigenaar is vervangen.",
                 f"We trekken het versturen van de {topic} in; niemand hoeft dit nog te doen." if retract else f"De afspraak over de {topic} blijft gelden; er is geen intrekking.",
                 f"Voorstel: gebruik de blauwe kast voor {topic}; nog geen beslissing.",
                 f"Definitieve beslissing: gebruik de blauwe kast voor {topic}.",
                 f"Herhaling zonder nieuwe afspraak: de blauwe kast is gekozen voor {topic}.",
                 f"Wie bewaart de reservesleutel voor {topic}? Dit blijft onbeantwoord.",
                 f"Wie zou dat nu niet weten? Dat is retorisch. De kastkeuze voor {topic} is ingetrokken." if retract_decision else "Wie zou dat nu niet weten? Dat is een retorische vraag, geen open actie.",
                 f"Antwoord: {owner} bewaart de reservesleutel; de vraag is opgelost." if answer else "We hebben nog steeds geen antwoord over de reservesleutel."]
    elif language == "mixed":
        lines = [f"Maybe kunnen we de {topic} sturen; this is not a commitment.",
                 f"{actor} will de {topic}{deadline} sturen. Dit is confirmed.",
                 f"Change: {alternate} neemt sending van de {topic} over; de previous owner is replaced.",
                 f"We retract sending de {topic}; nobody hoeft dit nog te doen." if retract else f"The commitment voor de {topic} remains; geen retraction.",
                 f"Proposal: gebruik de blue cabinet voor {topic}; not decided yet.",
                 f"Final decision: gebruik de blue cabinet voor {topic}.",
                 f"Repeated, no new decision: de blue cabinet is chosen voor {topic}.",
                 f"Who bewaart de backup key voor {topic}? Nog unanswered.",
                 f"Who would not know that? Retorisch. The cabinet decision voor {topic} is retracted." if retract_decision else "Who would not know that? Retorisch, geen unresolved question.",
                 f"Answer: {owner} bewaart de backup key; de question is resolved." if answer else "We still hebben geen answer over de backup key."]
    else:
        lines = [f"Perhaps we could send the {topic}; this is not a commitment.",
                 f"{actor} will send the {topic}{deadline}. This is a confirmed commitment.",
                 f"Change: {alternate} takes over sending the {topic}; the earlier owner is replaced.",
                 f"We retract sending the {topic}; nobody has to do it now." if retract else f"The commitment for the {topic} remains; there is no retraction.",
                 f"Proposal: use the blue cabinet for {topic}; no decision yet.",
                 f"Final decision: use the blue cabinet for {topic}.",
                 f"Repeated without a new decision: the blue cabinet is chosen for {topic}.",
                 f"Who keeps the backup key for {topic}? This remains unanswered.",
                 f"Who would not know that? This is rhetorical. The cabinet decision for {topic} is retracted." if retract_decision else "Who would not know that? This is rhetorical, not an unresolved question.",
                 f"Answer: {owner} keeps the backup key; the question is resolved." if answer else "We still have no answer about the backup key."]
    return lines, due


def _anchor(source, text=None):
    text = source["text"] if text is None else text
    start = source["text"].index(text)
    return {"utterance_id": source["utterance_id"], "revision": source["revision"], "source_id": source["source_id"],
            "source_fingerprint": source["source_fingerprint"], "start_char": start, "end_char": start + len(text),
            "text": text, "start_ms": source["start_ms"], "end_ms": source["end_ms"]}


def _item(key, kind, status, sources, owner=None, due=None, supersedes=None):
    last = sources[-1]
    return {"key": key, "kind": kind, "status": status, "text": last["text"], "evidence": [_anchor(source) for source in sources],
            "owner": {"text": owner, "evidence": [_anchor(source, owner) for source in reversed(sources) if owner in source["text"]][:1]} if owner else None,
            "due_phrase": {"text": due, "evidence": [_anchor(source, due) for source in sources if due in source["text"]][:1]} if due else None,
            "supersedes": supersedes}


def build_corpus():
    prefixes, cases = [], []
    topics = {"development": "seed checklist", "calibration": "repair checklist", "heldout": "canal checklist"}
    for split in SPLITS:
        for family in range(10):
            group = f"incremental-{split}-{family:02}"
            unknown, no_due = family % 4 == 0, family % 3 == 0
            retract, retract_decision, answer = family % 3 == 0, family % 4 == 1, family % 3 != 1
            for language in LANGUAGES:
                for variant in ("clean", "asr"):
                    owner, alternate = "Mira", "Noël" if variant == "clean" else "Noel"
                    meeting_id = f"{group}-{language}-{variant}"
                    lines, due = _lines(language, f"{topics[split]}-{family:02}", owner, alternate, unknown, no_due,
                                        retract, retract_decision, answer)
                    ambiguous_deadline = family % 5 == 4
                    if ambiguous_deadline:
                        lines[0] += {"en": " We are unsure whether the deadline was Thursday or Friday.",
                                     "nl": " Het is onzeker of de deadline donderdag of vrijdag was.",
                                     "mixed": " We zijn unsure of de deadline Thursday of vrijdag was."}[language]
                    changed_owner = family % 3 != 2
                    if not changed_owner:
                        lines[2] = {"en": f"No owner change: {owner} keeps the confirmed checklist commitment.",
                                    "nl": f"Geen andere eigenaar: {owner} behoudt de toegezegde checklistactie.",
                                    "mixed": f"No owner change: {owner} behoudt de confirmed checklist commitment."}[language]
                        if unknown:
                            lines[2] = {"en": "No owner change: the confirmed checklist commitment still has no stated name.",
                                        "nl": "Geen andere eigenaar: voor de toegezegde checklistactie is nog geen naam genoemd.",
                                        "mixed": "No owner change: de confirmed checklist commitment heeft nog geen stated name."}[language]
                    if variant == "asr":
                        lines = ["uh, " + text.replace("checklist", "check list") for text in lines]
                    sources = []
                    for index, text in enumerate(lines):
                        source = {"utterance_id": f"{meeting_id}-u{index + 1}", "revision": 1,
                                  "source_id": "microphone" if index % 2 else "system_audio", "start_ms": index * 5_000,
                                  "end_ms": (index + 1) * 5_000, "text": text, "failed": False, "finalized": True,
                                  "speaker": "unknown" if index % 3 == 0 else f"speaker_{index % 2 + 1}"}
                        source["source_fingerprint"] = source_fingerprint(source)
                        sources.append(source)
                    for count in PREFIXES:
                        cutoff = count * 5_000
                        prefix_id = f"{meeting_id}-p{count:02}"
                        prefix = {"id": prefix_id, "meeting_uid": meeting_id, "session_id": meeting_id + "-session",
                                  "group_id": group, "language": language, "variant": variant, "split": split,
                                  "transcript_revision": count, "included_through_ms": cutoff,
                                  "utterances": [source.copy() for source in sources[:count]], "notes": [],
                                  "provenance": "owned_synthetic; ASR-like text perturbations are not natural recording acceptance"}
                        if count >= 4:
                            note = {"en": "Private draft; verify the owner.", "nl": "Privékladversie; controleer de eigenaar.",
                                    "mixed": "Private draft; controleer de owner."}[language]
                            prefix["notes"] = [{"id": meeting_id + "-note", "revision": 1, "text": note,
                                                "acknowledged": True, "available_ms": 20_000}]
                        prefixes.append(prefix)
                        items = []
                        labels = {"action": "none", "decision": "none", "question": "none"}
                        if count >= 2:
                            key, actor, supporting, kind, status = "action-checklist", None if unknown else owner, [sources[1]], "action", "confirmed"
                            if count >= 3 and changed_owner:
                                actor, supporting, kind, status = alternate, [sources[1], sources[2]], "changed_commitment", "changed"
                            if count >= 4 and retract:
                                actor, supporting, kind, status = None, [sources[1], sources[2], sources[3]], "changed_commitment", "retracted"
                            items.append(_item(key, kind, status, supporting, actor, None if no_due or status == "retracted" else due,
                                               "action-checklist" if count >= 3 and changed_owner else None))
                            labels["action"] = kind
                        if count >= 6:
                            kind, status, supporting = "decision", "confirmed", [sources[5]]
                            if count >= 10 and retract_decision:
                                kind, status, supporting = "changed_commitment", "retracted", [sources[5], sources[8]]
                            items.append(_item("decision-cabinet", kind, status, supporting, supersedes="decision-cabinet" if status == "retracted" else None))
                            labels["decision"] = kind
                        if count >= 8 and not (count >= 10 and answer):
                            items.append(_item("question-key", "open_question", "unresolved", [sources[7]]))
                            labels["question"] = "open_question"
                        for use_case in USE_CASES:
                            if use_case == "manual_finished_suggestions" and count != 10:
                                continue
                            case = {"id": prefix_id + ":" + use_case, "prefix_id": prefix_id, "group_id": group,
                                    "language": language, "split": split, "use_case": use_case,
                                    "tags": ["prefix_cutoff", "negation", "retraction", "owner_change", "deduplication", "unknown_attribution", variant]}
                            if use_case == "source_assessment":
                                claim = {"en": "The checklist is currently a confirmed commitment.",
                                         "nl": "De checklistactie is momenteel definitief toegezegd.",
                                         "mixed": "De checklist is currently een confirmed commitment."}[language]
                                support = "supported" if count >= 2 and not (count >= 4 and retract) else "contradicted" if count >= 1 else "insufficient"
                                answerability = "answerable" if count >= 1 else "absent"
                                if ambiguous_deadline:
                                    claim = {"en": "The current deadline is Thursday.", "nl": "De huidige deadline is donderdag.",
                                             "mixed": "De current deadline is on donderdag."}[language]
                                    if count == 0:
                                        support, answerability = "insufficient", "absent"
                                    elif count == 1:
                                        support, answerability = "uncertain", "uncertain"
                                    elif count >= 4 and retract:
                                        support, answerability = "contradicted", "answerable"
                                    elif no_due:
                                        support, answerability = "uncertain", "partial"
                                    else:
                                        support, answerability = "supported", "answerable"
                                case.update(claim=claim, claim_provenance="authored_fixture",
                                            expected_components={"claim_support": support, "answerability": answerability}, expected_items=[])
                            else:
                                case.update(expected_components=labels.copy(), expected_items=items)
                            case["expected_review"] = {"accepted_edits": {"action-checklist": "User-edited wording remains unchanged."} if count >= 3 else {},
                                                       "dismissed": ["decision-cabinet"] if count >= 8 else []}
                            cases.append(case)
    result = {"schema_version": 1, "corpus_id": "incremental-meeting-v1", "prefixes": prefixes, "cases": cases,
              "fixture_events": [{"id": "provisional-revision", "finalized": False, "text": "PROVISIONAL_FUTURE_SENTINEL"}],
              "unmeasured": ["natural_recording_asr", "actual_generated_answer_claims", "deadline_normalization", "native_recording_resource_impact"]}
    validate_corpus(result)
    return result


def validate_corpus(corpus):
    if corpus.get("schema_version") != 1 or not corpus.get("prefixes") or not corpus.get("cases"):
        raise ValueError("invalid_incremental_corpus")
    groups, prefixes, meeting_groups = {}, {}, {}
    for prefix in corpus["prefixes"]:
        if prefix["id"] in prefixes or prefix["language"] not in LANGUAGES or prefix["split"] not in SPLITS:
            raise ValueError("duplicate_or_invalid_prefix")
        prefixes[prefix["id"]] = prefix
        if meeting_groups.setdefault(prefix["meeting_uid"], prefix["group_id"]) != prefix["group_id"]:
            raise ValueError("meeting_identity_family_leakage")
        if groups.setdefault(prefix["group_id"], prefix["split"]) != prefix["split"]:
            raise ValueError("meeting_family_split_leakage")
        seen = set()
        for source in prefix["utterances"]:
            if source["utterance_id"] in seen or source["finalized"] is not True or source.get("failed") is not False:
                raise ValueError("provisional_or_duplicate_evidence")
            seen.add(source["utterance_id"])
            if (any(not isinstance(source.get(field), int) or isinstance(source[field], bool) for field in ("revision", "start_ms", "end_ms"))
                    or source["revision"] < 1 or not isinstance(source["utterance_id"], str) or not source["utterance_id"]
                    or not isinstance(source["text"], str)):
                raise ValueError("invalid_source_identity")
            if not 0 <= source["start_ms"] < source["end_ms"] <= prefix["included_through_ms"]:
                raise ValueError("future_or_invalid_source_timing")
            if source_fingerprint(source) != source["source_fingerprint"]:
                raise ValueError("source_fingerprint_changed")
        for note in prefix["notes"]:
            if note["acknowledged"] is not True or note["available_ms"] > prefix["included_through_ms"]:
                raise ValueError("future_or_unacknowledged_notes")
    seen_cases = set()
    for case in corpus["cases"]:
        if case["id"] in seen_cases or case["prefix_id"] not in prefixes or case["use_case"] not in USE_CASES:
            raise ValueError("invalid_or_duplicate_case")
        seen_cases.add(case["id"])
        prefix = prefixes[case["prefix_id"]]
        if any(case[field] != prefix[field] for field in ("group_id", "language", "split")):
            raise ValueError("case_prefix_identity_mismatch")
        available = {source["utterance_id"]: source for source in prefix["utterances"]}
        for item in case["expected_items"]:
            for anchor in item["evidence"]:
                source = available.get(anchor["utterance_id"])
                if (source is None or source["text"][anchor["start_char"]:anchor["end_char"]] != anchor["text"]
                        or any(anchor.get(field) != source[field] for field in ("revision", "source_id", "source_fingerprint", "start_ms", "end_ms"))):
                    raise ValueError("future_gold_evidence")
    counts = Counter((prefix["language"], prefix["split"]) for prefix in prefixes.values())
    if any(not counts[language, split] for language in LANGUAGES for split in SPLITS):
        raise ValueError("missing_language_split")
