"""Exercise public Pennel installers, exclusively on a fresh GitHub-hosted Mac."""

import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import sqlite3
import subprocess
import sys
import time
from urllib.request import urlopen


TAP = "pennel-app/pennel"
CASK = TAP + "/pennel"
RELEASES = {
    "0.1.47": ("0a684715272a131f8fed5f9c5f2e96d371883cf5", "e5dbea86517965ccaa49b539ca4c2a9e16810e77fdcfd6c2a098ee0a6222d8b1"),
    "0.1.49": ("27b076a327d0c6f3641dd0f53d10d7f0a03be14d", "fe53d1f1f3ff66bbf1f6d7b6d206b952e936c6a6c6d7227e37ac6b571c4bed6b"),
    "0.1.52": ("906c23a982506f307c53c9a68c935efe1e193f83", "682ac00daa5044d35f1187bc33cd828212da4195fc793b3e2af9fedc4e6b6aa8"),
}
APP = Path("/Applications/Pennel.app")
DATA = Path.home() / "Library/Application Support/Pennel"
LIBRARY = Path.home() / "Pennel fixture library"
MEETING = LIBRARY / "2026-11-18 0900 Homebrew QA meeting" / "meeting.md"
HISTORY = DATA / "history.sqlite"
FIXTURE_ID = "019a946d-30c0-7000-8000-000000000001"
TEXT = "Homebrew QA history sample"
MEETING_BODY = "---\ntitle: Homebrew QA meeting\nstart: 2026-11-18T09:00:00Z\nduration_min: 0\ntemplate: \npennel_id: homebrew-qa-sample\n---\n\nSynthetic summary.\n"


def run(*args, capture=False, check=True, env=None, timeout=150):
    return subprocess.run(
        list(map(str, args)), check=check, env=env, text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.STDOUT if capture else None, timeout=timeout,
    )


def text(*args):
    return run(*args, capture=True).stdout.strip()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def verify_tap(tap_dir, version):
    pin, digest = RELEASES[version]
    require(text("git", "-C", tap_dir, "remote", "get-url", "origin").rstrip("/").removesuffix(".git").lower()
            == "https://github.com/pennel-app/homebrew-pennel", "unexpected tap origin")
    require(text("git", "-C", tap_dir, "rev-parse", "HEAD") == pin, "tap moved from the expected public revision")
    cask = (tap_dir / "Casks/pennel.rb").read_text()
    require(f'sha256 "{digest}"' in cask and f'version "{version}"' in cask, "cask version or digest changed")
    require(not text("git", "-C", tap_dir, "status", "--porcelain", "--untracked-files=no"), "modified tap checkout")


def package(version, tap_dir, result):
    verify_tap(tap_dir, version)
    pin, digest = RELEASES[version]
    name = f"Pennel_{version}_aarch64.dmg"
    url = f"https://github.com/pennel-app/pennel/releases/download/v{version}/SHA256SUMS.txt"
    with urlopen(url, timeout=30) as response:
        checksum_rows = [row.split() for row in response.read().decode().splitlines()]
    require([digest, name] in checksum_rows, "release checksum mismatch")
    cache = Path(text("brew", "--cache")) / "downloads"
    files = list(cache.glob("*--" + name))
    require(len(files) == 1 and hashlib.sha256(files[0].read_bytes()).hexdigest() == digest,
            "installed cask download did not match published hash")
    info = plistlib.loads((APP / "Contents/Info.plist").read_bytes())
    require(info["CFBundleShortVersionString"] == version and info["CFBundleIdentifier"] == "dev.pennel.Pennel",
            "installed product identifier or version mismatch")
    require(text("lipo", "-archs", APP / "Contents/MacOS" / info["CFBundleExecutable"]) == "arm64", "unexpected architecture")
    run("codesign", "--verify", "--deep", "--strict", APP)
    identity = text("codesign", "-dv", "--verbose=4", APP)
    result.update(tap_commit=pin, dmg_sha256=digest, download_bytes=files[0].stat().st_size,
                  installed_bundle_version=info["CFBundleVersion"], architecture="arm64", strict_signature=True)
    if version == "0.1.47":
        require("Authority=Pennel Release Signing" in identity or "TeamIdentifier=not set" in identity, "unexpected legacy signing identity")
        result["historical_signing"] = "self-signed cask"
        return
    require("TeamIdentifier=B538Q2K76C" in identity and "Authority=Developer ID Application:" in identity,
            "unexpected published Developer ID identity")
    run("xcrun", "stapler", "validate", APP)
    run("xcrun", "stapler", "validate", files[0])
    assessment = run("spctl", "-a", "-vvv", "--type", "execute", APP, capture=True, check=False)
    result.update(personal_developer_id=True, app_ticket_valid=True, dmg_ticket_valid=True,
                  gatekeeper_assessment=assessment.stdout.strip())
    require(assessment.returncode == 0 and "Notarized Developer ID" in assessment.stdout, "Gatekeeper did not accept the installed app")


def observer():
    return Path(os.environ["RUNNER_TEMP"]) / "pennel-observe"


def launch(result, fixture=False):
    run("open", "-a", APP, timeout=20)
    observer_result = {}
    for _ in range(35):
        probe = run(observer(), "inspect", capture=True, check=False, timeout=8)
        if probe.returncode == 0:
            observer_result = json.loads(probe.stdout)
            if observer_result.get("finished_launching") and HISTORY.exists():
                break
        time.sleep(0.5)
    result["native"] = observer_result
    require(observer_result.get("finished_launching") and HISTORY.exists(), "installed application did not finish initialization")
    if fixture:
        verify_fixtures(result)
        result["native"] = json.loads(text(observer(), "fixtures"))


def insert_entry(connection, entry_id, timestamp, value):
    connection.execute(
        "INSERT INTO dictations(id, created_at_ms, raw, text, status, has_audio) VALUES(?, ?, ?, ?, 'done', 0)",
        (entry_id, timestamp, value, value),
    )


def seed():
    require(HISTORY.is_file(), "released app did not initialize the history database")
    stt = {"provider_id": "local", "model": "whisper-large-v3-turbo"}
    settings = {
        "providers": [{"id": "local", "kind": "local", "name": "Local", "base_url": ""},
                      {"id": "groq", "kind": "groq", "name": "Groq", "base_url": "https://api.groq.com/openai/v1"}],
        "dictation_stt": stt, "meeting_stt": stt, "llm": {"provider_id": "groq", "model": "llama-3.3-70b-versatile"},
        "mode": "fast", "language": "nl", "ui_language": "en", "dictionary": ["QA-Proefwoord"], "style_rules": [],
        "history_retention_days": 1, "hotkey": "fn", "local_unload_minutes": 5,
        "meetings_dir": str(LIBRARY), "calendar_enabled": False, "auto_detect": False,
        "onboarding": {"version": 1, "status": "completed", "resume_step": "practice", "goal": "dictation"}
    }
    (DATA / "settings.json").write_text(json.dumps(settings))
    MEETING.parent.mkdir(parents=True)
    MEETING.write_text(MEETING_BODY)
    MEETING.with_name("notes.md").write_text("QA typed note.\n")
    with sqlite3.connect(HISTORY) as database:
        insert_entry(database, FIXTURE_ID, int(time.time() * 1000), TEXT)


def add_runtime_witness():
    # Only the non-default one-day setting removes a two-day-old record on launch.
    with sqlite3.connect(HISTORY) as database:
        insert_entry(database, "qa-expired", int((time.time() - 2 * 86400) * 1000), "expired fixture")


def verify_fixtures(result):
    settings = json.loads((DATA / "settings.json").read_text())
    require(settings["meetings_dir"] == str(LIBRARY) and settings["dictionary"] == ["QA-Proefwoord"]
            and settings["language"] == "nl" and settings["dictation_stt"]["model"] == "whisper-large-v3-turbo"
            and settings["llm"]["provider_id"] == "groq", "saved preferences or processing selections changed")
    require(MEETING.read_text() == MEETING_BODY and MEETING.with_name("notes.md").read_text() == "QA typed note.\n",
            "meeting files changed")
    with sqlite3.connect(HISTORY) as database:
        require(database.execute("SELECT text FROM dictations WHERE id=?", (FIXTURE_ID,)).fetchone() == (TEXT,),
                "saved history entry changed")
        require(database.execute("SELECT id FROM dictations WHERE id='qa-expired'").fetchone() is None,
                "runtime did not consume the saved retention setting")
    lock = LIBRARY / ".pennel-library.lock"
    holder = run("/usr/sbin/lsof", "-Fpc", lock, capture=True, check=False)
    require(holder.returncode == 0 and "pennel" in holder.stdout.lower(), "custom library was not opened by Pennel")
    result.update(saved_preferences=True, custom_library_open=True, meeting_and_notes_preserved=True,
                  current_history_preserved=True, runtime_applied_one_day_history_retention=True)


def main(receipt):
    require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
            and os.environ.get("GITHUB_REPOSITORY") == "pennel-app/pennel"
            and os.environ.get("GITHUB_REF") == "refs/heads/codex/mac-release-install-qa", "fresh hosted runner required")
    require(platform.system() == "Darwin" and platform.machine() == "arm64", "Apple Silicon Mac required")
    require(not any(path.exists() for path in (APP, DATA, LIBRARY)), "existing installation or fixture detected")
    receipt.update(macos=text("sw_vers", "-productVersion"), os_build=text("sw_vers", "-buildVersion"),
                   homebrew=text("brew", "--version"), gatekeeper_host=text("spctl", "--status"))
    audio = json.loads(text("system_profiler", "SPAudioDataType", "-json"))
    devices = [device for group in audio.get("SPAudioDataType", []) for device in group.get("_items", [])]
    receipt["audio_devices_with_input_channels"] = sum(bool(device.get("coreaudio_device_input")) for device in devices)
    scenario = receipt["scenario"]
    history_env = {**os.environ, "HOMEBREW_NO_AUTO_UPDATE": "1"}
    tap_dir = Path(text("brew", "--repository")) / "Library/Taps/pennel-app/homebrew-pennel"
    if scenario == "upgrade":
        run("brew", "tap", TAP)
    for index, version in enumerate(["0.1.52"] if scenario == "clean" else RELEASES):
        result = receipt["versions"][version] = {}
        if scenario == "clean":
            require(text("git", "ls-remote", "https://github.com/pennel-app/homebrew-pennel", "refs/heads/main").split()[0]
                    == RELEASES[version][0], "tap has moved; reselect the current published version")
        else:
            pin = RELEASES[version][0]
            run("git", "-C", tap_dir, "fetch", "--depth=1", "origin", pin, env=history_env)
            run("git", "-C", tap_dir, "checkout", "--detach", pin)
            verify_tap(tap_dir, version)
        args = ["brew", "upgrade", "--cask", "pennel"] if index else ["brew", "install", "--cask", CASK]
        run(*args, env=history_env if scenario == "upgrade" else None, timeout=320)
        result["homebrew_command"] = " ".join(args)
        package(version, tap_dir, result)
        if scenario == "upgrade" and index == 0:
            launch(result)
            run(observer(), "quit")
            seed()
            result["seeded_synthetic_profile_in_app_initialized_storage"] = True
        if scenario == "upgrade":
            add_runtime_witness()
        launch(result, fixture=scenario == "upgrade")
        run(observer(), "quit")
        result["normal_quit"] = True
        print(json.dumps({version: result}), flush=True)


if __name__ == "__main__":
    outcome = {"scenario": sys.argv[1], "versions": {}, "dictation_verified": False, "credential_access_verified": False}
    try:
        main(outcome)
    except Exception as error:
        outcome["stopped"] = str(error)
        raise
    finally:
        filename = Path(os.environ["RUNNER_TEMP"]) / "pennel-install-receipt.json"
        filename.write_text(json.dumps(outcome, indent=2) + "\n")
        print(json.dumps(outcome), flush=True)
