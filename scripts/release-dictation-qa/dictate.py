"""Verify a new History entry through the release UI, using isolated virtual input only."""

import hashlib
import json
import os
from pathlib import Path
import platform
import re
import sqlite3
import subprocess
import sys
import time

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS / "release-install-qa"))
import verify as install

MODEL = "whisper-small"
PHRASE = "The purple bicycle is waiting beside the silver bridge."
KEYWORDS = ("purple", "bicycle", "silver", "bridge")
APP = install.APP
DATA = install.DATA
TEMP = Path(os.environ["RUNNER_TEMP"])
OUTPUT = TEMP / "dictation-receipt.json"


def save(receipt):
    OUTPUT.write_text(json.dumps(receipt, indent=2) + "\n")


def control(action="audio-state", label=None):
    args = [install.observer(), action]
    if label: args.append(label)
    return json.loads(install.text(*args))


def snapshot(receipt, stage, current):
    fields = {key: current.get(key) for key in ("observer_permitted", "web_area_visible", "app_window_count", "installed_pid")}
    # App text only ever comes from this disposable profile. Do not publish the transcript.
    is_phrase = lambda value: any(word in value.lower() for word in KEYWORDS)
    fields["enabled_buttons"] = sorted({word for word in current.get("enabled_buttons", []) if not is_phrase(word)})[:65]
    fields["status"] = [value for value in current.get("document_text", []) if not is_phrase(value) and
                        any(word in value.lower() for word in ("microphone", "dictation", "local", "permission", "model", "working", "ready"))][:20]
    receipt[stage] = fields
    save(receipt)


def fresh_app(receipt):
    install.require(not APP.exists() and not DATA.exists(), "fresh Mac and application data required")
    tap_hash = install.RELEASES["0.1.52"][0]
    environment = {**os.environ, "HOMEBREW_NO_AUTO_UPDATE": "1", "HOMEBREW_NO_ANALYTICS": "1"}
    install.run("brew", "tap", install.TAP, env=environment)
    tap = Path(install.text("brew", "--repository", install.TAP))
    install.run("git", "-C", tap, "fetch", "--depth=1", "origin", tap_hash)
    install.run("git", "-C", tap, "checkout", "--detach", tap_hash)
    install.verify_tap(tap, "0.1.52")
    install.run("brew", "install", "--cask", install.CASK, env=environment, timeout=360)
    package = receipt["release"] = {}
    install.package("0.1.52", tap, package)
    install.launch(package)
    install.require(package.get("native", {}).get("finished_launching") and install.HISTORY.exists(), "first launch failed")
    install.stop(package)
    install.require(package.get("normal_quit"), "clean app did not quit")
    save(receipt)


def setup_local_model(receipt):
    catalog = json.loads((SCRIPTS.parent / "core/src/catalog.json").read_text())["models"]
    model = next(model for model in catalog if model["id"] == MODEL)
    artifact = model["files"][model["quant"]]
    url = f'https://huggingface.co/{model["repository"]}/resolve/{model["revision"]}/{artifact["filename"]}?download=true'
    model_dir = DATA / "models"
    model_dir.mkdir(exist_ok=True)
    path = model_dir / (MODEL + ".gguf")
    install.run("curl", "--fail", "--show-error", "--silent", "--location", "--retry", "2", "--proto", "=https",
                "--proto-redir", "=https", "--output", path, url, timeout=220)
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    install.require(digest == artifact["sha256"] and path.stat().st_size == artifact["size"], "release catalog model mismatch")
    receipt["model"] = {"id": MODEL, "url": url, "sha256": digest, "bytes": path.stat().st_size}
    # Provision only local model preferences. History is always opened in read-only mode.
    pref = DATA / "settings.json"
    settings = json.loads(pref.read_text()) if pref.is_file() else {
        "providers": [ {"id":"local","kind":"local","name":"Local","base_url":""} ],
        "llm": {"provider_id":"", "model":""}, "dictionary":[], "style_rules":[],
        "history_retention_days":30, "hotkey":"fn", "local_unload_minutes":5,
    }
    local = next(p for p in settings["providers"] if p["kind"] == "local")
    choice = {"provider_id":local["id"], "model": MODEL}
    settings.update(dictation_stt=choice, meeting_stt=choice, mode="fast", language="en", ui_language="en",
                    dictionary=[], live_text_enabled=False, auto_detect=False, cloud_sync=False, calendar_enabled=False,
                    onboarding={"version":1, "status":"completed", "resume_step":"practice", "goal":"dictation"})
    pref.write_text(json.dumps(settings))
    receipt["profile_preparation"] = "Released model bytes and local-only Fast settings staged after the clean first launch."
    save(receipt)


def history():
    with sqlite3.connect(install.HISTORY.as_uri() + "?mode=ro", uri=True) as db:
        return db.execute("SELECT id, created_at_ms, raw, text, status, error FROM dictations ORDER BY created_at_ms").fetchall()


def await_button(label, seconds=12):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        current = control()
        if label in current.get("enabled_buttons", []): return current
        time.sleep(0.4)
    return control()


def request_microphone(receipt):
    current = await_button("Start dictation", 3)
    if any(label in current.get("enabled_buttons", []) for label in ("Start dictation", "Try it here", "Hands-free")):
        return
    if "Finish setup" in current.get("enabled_buttons", []):
        receipt["opened_permission_setup"] = control("audio-press", "Finish setup").get("pressed")
    current = await_button("Allow access", 8)
    snapshot(receipt, "before_permission_request", current)
    if "Allow access" in current.get("enabled_buttons", []):
        receipt["ordinary_app_microphone_request"] = control("audio-press", "Allow access").get("pressed")
        for _ in range(15):
            observed = control("audio-microphone-consent")
            if observed.get("allowed"):
                receipt["ordinary_os_microphone_confirmation"] = observed
                break
            time.sleep(0.4)
        save(receipt)
        stopped = {}
        install.stop(stopped)
        install.require(stopped.get("normal_quit"), "could not relaunch after microphone permission")
        install.launch({"personal_developer_id": True})


def exercise(receipt):
    fresh_app(receipt)
    setup_local_model(receipt)
    install.require(not history(), "a clean app must have zero prior History entries")
    startup = {"personal_developer_id": True}
    install.launch(startup)
    receipt["configured_launch"] = startup
    state = control()
    if not state.get("web_area_visible"):
        control("audio-reopen")
    current = control()
    if "Dictation" in current.get("enabled_buttons", []): control("audio-press", "Dictation")
    snapshot(receipt, "dictation_home", control())
    request_microphone(receipt)
    state = await_button("Start dictation", 10)
    if "Start dictation" not in state.get("enabled_buttons", []) and "Try it here" in state.get("enabled_buttons", []):
        control("audio-press", "Try it here")
        state = await_button("Start dictation", 4)
    snapshot(receipt, "before_start", state)
    start_label = next((label for label in ("Start dictation", "Hands-free") if label in state.get("enabled_buttons", [])), None)
    install.require(start_label, "release UI did not offer a usable dictation control")
    phrase_audio = TEMP / "pennel-public-utterance.aiff"
    install.run("/usr/bin/say", "-r", "145", "-o", phrase_audio, PHRASE)
    install.require(phrase_audio.stat().st_size > 16_000, "the synthesized public utterance contains no useful duration")
    receipt["phrase_audio"] = {"generator":"macOS say, 145 words per minute", "sha256":hashlib.sha256(phrase_audio.read_bytes()).hexdigest(),
                              "public_reference":PHRASE}
    before = history()
    install.require(not before, "History changed before the actual dictation")
    receipt["history_before"] = len(before)
    began = int(time.time() * 1000)
    receipt["start_control"] = start_label
    install.require(control("audio-press", start_label).get("pressed"), "ordinary Start dictation action unavailable")
    listening = await_button("Stop dictation", 12)
    snapshot(receipt, "listening", listening)
    install.require("Stop dictation" in listening.get("enabled_buttons", []), "signed application did not start listening")
    time.sleep(0.5)
    install.run("/usr/bin/afplay", phrase_audio, timeout=40)
    time.sleep(0.65)
    install.require(control("audio-press", "Stop dictation").get("pressed"), "ordinary Stop dictation action unavailable")
    deadline = time.monotonic() + 95
    rows = []
    while time.monotonic() < deadline:
        rows = history()
        if rows: break
        time.sleep(0.4)
    snapshot(receipt, "after_stop", control())
    receipt["history_after"] = len(rows)
    install.require(len(rows) == 1, "expected exactly one new History entry")
    entry_id, created, raw, text, status, error = rows[0]
    raw_words = set(re.findall(r"[a-z]+", raw.lower()))
    final_words = set(re.findall(r"[a-z]+", text.lower()))
    receipt.update(history_entry_created_after_start=created >= began, history_status=status, raw_length=len(raw),
                   final_length=len(text), error_present=bool(error), raw_sha256=hashlib.sha256(raw.encode()).hexdigest(),
                   final_sha256=hashlib.sha256(text.encode()).hexdigest(),
                   raw_reference_matches={word: word in raw_words for word in KEYWORDS},
                   final_reference_matches={word: word in final_words for word in KEYWORDS})
    save(receipt)
    install.require(bool(entry_id) and created >= began and status == "done" and not error and
                    all(word in raw_words and word in final_words for word in KEYWORDS),
                    "new app-generated recognition did not match the controlled phrase")
    receipt["dictation_verified"] = True
    if control("audio-press", "All history").get("pressed"):
        current = control()
        whole = " ".join(current.get("buttons", []) + current.get("document_text", [])).lower()
        receipt["result_visible_in_history_ui"] = all(word in whole for word in KEYWORDS)
    install.stop(receipt)
    install.require(receipt.get("normal_quit"), "release did not quit normally")


if __name__ == "__main__":
    install.require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
                    and os.environ.get("GITHUB_REPOSITORY") == "pennel-app/pennel" and
                    os.environ.get("GITHUB_REF") == "refs/heads/codex/mac-release-loopback-dictation"
                    and platform.system() == "Darwin" and platform.machine() == "arm64", "disposable hosted Apple Silicon required")
    result = {"dictation_verified":False, "capture_input":"BlackHole 2ch", "physical_microphone":False,
              "macos":install.text("sw_vers", "-productVersion"), "build":install.text("sw_vers", "-buildVersion")}
    try:
        exercise(result)
    except Exception as exc:
        result["stopped"] = str(exc)
        raise
    finally:
        save(result)
        print(json.dumps(result), flush=True)
