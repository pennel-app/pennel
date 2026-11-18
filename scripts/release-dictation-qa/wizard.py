"""Follow fresh onboarding using normal release controls and Pennel's own model downloader."""
import hashlib
import json
import os
import platform
import time

import dictate as qa
import verify as install

PRACTICE = "The yellow lantern shines beside the quiet garden."
PRACTICE_WORDS = ("yellow", "lantern", "quiet", "garden")


def stage(condition, seconds):
    until = time.monotonic() + seconds
    while True:
        state = qa.control()
        if condition(state) or time.monotonic() >= until: return state
        time.sleep(0.7)


def copy(state):
    return " ".join(state.get("buttons", []) + state.get("document_text", []))


def click(receipt, label):
    install.require(qa.control("audio-press", label).get("pressed"), f"wizard's {label} button unavailable")
    receipt["wizard_actions"].append(label)
    qa.save(receipt)


def observe_settings():
    path = qa.DATA / "settings.json"
    return json.loads(path.read_text()) if path.is_file() else {}


def verify_selected_model(receipt):
    settings = observe_settings()
    selected = settings["dictation_stt"]
    local = next((p for p in settings["providers"] if p["id"] == selected["provider_id"]), {})
    install.require(local.get("kind") == "local" and settings["mode"] == "fast", "wizard did not select local, speech-only recognition")
    catalog = json.loads((qa.SCRIPTS.parent / "core/src/catalog.json").read_text())["models"]
    model = next(m for m in catalog if m["id"] == selected["model"])
    artifact = model["files"][model["quant"]]
    path = qa.DATA / "models" / (model["id"] + ".gguf")
    with path.open("rb") as data:
        sha = hashlib.file_digest(data, "sha256").hexdigest()
    install.require(sha == artifact["sha256"] and path.stat().st_size == artifact["size"], "wizard model did not match release catalog")
    receipt["app_downloaded_model"] = {"id":model["id"], "sha256":sha, "bytes":path.stat().st_size,
                                       "revision":model["revision"], "repository":model["repository"]}
    qa.save(receipt)


def exercise(receipt):
    qa.fresh_app(receipt, keep_running=True)
    install.require(not qa.history(), "fresh release already contained a dictation")
    receipt["initial_onboarding_status"] = observe_settings().get("onboarding", {}).get("status")
    install.require(receipt["initial_onboarding_status"] in ("pending", None), "fresh user expected")
    current = stage(lambda s: "Try dictation" in s.get("enabled_buttons", []), 10)
    qa.snapshot(receipt, "welcome", current)
    click(receipt, "Try dictation")
    click(receipt, "Continue")
    state = stage(lambda s: "Download and continue" in s.get("enabled_buttons", []), 14)
    qa.snapshot(receipt, "model_offer", state)
    install.require("Download and continue" in state.get("enabled_buttons", []), "fresh release did not offer a model download")
    click(receipt, "Download and continue")
    ready = stage(lambda s: "Let Pennel listen and type." in copy(s) or
                  "The model could not be downloaded" in copy(s), 240)
    qa.snapshot(receipt, "permissions_page", ready)
    install.require("Let Pennel listen and type." in copy(ready), "wizard could not advance beyond the model download")
    verify_selected_model(receipt)
    if "Allow access" in ready.get("enabled_buttons", []):
        click(receipt, "Allow access")
        for _ in range(22):
            observed = qa.control("audio-microphone-consent")
            if observed.get("allowed"):
                receipt["ordinary_os_microphone_confirmation"] = observed
                break
            time.sleep(0.4)
    def next_label(state):
        return next((button for button in state.get("enabled_buttons", [])
                     if button == "Continue" or button.startswith("Continue without ") or button == "Continue with the button"), None)
    ready = stage(next_label, 16)
    qa.snapshot(receipt, "permission_result", ready)
    install.require(next_label(ready), "ordinary microphone permission has not enabled Continue")
    click(receipt, next_label(ready))
    ready = qa.await_button("Start dictation", 14)
    qa.snapshot(receipt, "practice_screen", ready)
    install.require("Try your first dictation." in copy(ready), "fresh-user practice screen missing")
    first = receipt["practice"] = {}
    qa.capture_ui(first, "Start dictation", PRACTICE, PRACTICE_WORDS, expected_prior=0)
    ready = qa.await_button("Open dictation", 15)
    qa.snapshot(receipt, "practice_result", ready)
    install.require("Open dictation" in ready.get("enabled_buttons", []), "successful practice did not enable Open dictation")
    click(receipt, "Open dictation")
    receipt["final_onboarding_status"] = observe_settings().get("onboarding", {}).get("status")
    install.require(receipt["final_onboarding_status"] == "completed", "app did not persist completed onboarding")
    receipt["wizard_verified"] = True
    qa.save(receipt)
    # Use a separate phrase after leaving the wizard; the two actual app-generated rows must be distinct.
    ready = stage(lambda s: "Try it here" in s.get("enabled_buttons", []) or "Hands-free" in s.get("enabled_buttons", []), 12)
    if "Try it here" in ready.get("enabled_buttons", []):
        click(receipt, "Try it here")
    ready = qa.await_button("Start dictation", 8)
    start = next((b for b in ("Start dictation", "Hands-free") if b in ready.get("enabled_buttons", [])), None)
    install.require(start, "normal dictation did not become available after the wizard")
    second = receipt["normal_dictation"] = {}
    qa.capture_ui(second, start, qa.PHRASE, qa.KEYWORDS, expected_prior=1)
    qa.history_ui(second, qa.KEYWORDS)
    install.stop(receipt)
    install.require(receipt.get("normal_quit"), "normal quit missing")


if __name__ == "__main__":
    install.require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
                    and os.environ.get("GITHUB_REPOSITORY") == "pennel-app/pennel"
                    and os.environ.get("GITHUB_REF") == "refs/heads/codex/mac-release-loopback-dictation"
                    and platform.system() == "Darwin" and platform.machine() == "arm64", "disposable hosted Mac required")
    result = {"wizard_verified":False, "wizard_actions":[], "app_settings_written_by_trial":False,
              "model_bytes_staged_by_trial":False, "capture_input":"BlackHole 2ch", "physical_microphone":False,
              "macos":install.text("sw_vers", "-productVersion"), "build":install.text("sw_vers", "-buildVersion")}
    try:
        exercise(result)
    except Exception as error:
        result["stopped"] = str(error)
        raise
    finally:
        qa.save(result)
        print(json.dumps(result), flush=True)
