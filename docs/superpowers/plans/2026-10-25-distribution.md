# Distributie — publiceren van de gedeelde app en publieke repo — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One manual workflow turns an accepted, signed desktop candidate into a draft GitHub release plus the update feed and website metadata; the website shows each platform only once it passed acceptance; the repository is safe and accurate to make public.

**Architecture:** No rebuild at publication. `scripts/desktop-publish.py` takes the candidate artifacts (downloaded from an existing `desktop-candidate` run) and the acceptance receipt, reuses the checks in `scripts/desktop-update-manifest.py`, and writes `SHA256SUMS`, `latest.json`, `appcast.xml` (copied from the Mac candidate's legacy Sparkle output) and `release.json` (the website's `Release` shape). `.github/workflows/desktop-publish.yml` is glue: download, run the script, create a draft release, hand the metadata to `pennel-cloud`.

**Tech Stack:** Python 3 stdlib (unittest), GitHub Actions, `gh` CLI, gitleaks; Next 16 + vitest in `pennel-cloud/apps/web`.

**Repos:** `<app-checkout>` (Tasks 1–3, 5), branch `feat/desktop-publish` from `main`. `<cloud-checkout>` (Task 4), branch `feat/download-platforms` from `main`.

**Spec:** `docs/superpowers/specs/2026-10-25-distribution-design.md`.

## Global Constraints

- Never generate, replace, print or log signing keys, updater keys, certificates or tokens. Secrets reach tools only through environment variables or files with mode 600, never command lines (as `sign_detached` already does).
- Publication never rebuilds: every published byte has the SHA-256 recorded in the acceptance receipt.
- The GitHub release is created as a **draft**. Nothing in this plan makes a release public without a human action.
- A platform appears in `release.json` only if its acceptance gate is `passed` (`macos`; `windows`; `linux_x11` **and** `linux_wayland` for Linux). `latest.json` still requires all `REQUIRED_GATES` and all three `REQUIRED_TARGETS`, unchanged.
- Update URLs carry no identifiers beyond version and file name.
- `make packaging-test` (= `python3 -B -m unittest discover -s scripts/tests -p 'test_desktop_*.py'`) must pass; in `pennel-cloud/apps/web`: `npm run lint && npm run typecheck && npm test && npm run build`.

## Review Focus

1. A receipt where `windows` is `pending` but everything else passed → `release.json` lists Mac and Linux only, and `latest.json` is **refused** (the updater feed needs all targets). Test in Task 1.
2. An artifact downloaded from the candidate run whose SHA-256 differs from the receipt (wrong run id, re-run) → the script fails before writing any output. Test in Task 1.
3. The Linux AppImage `.sig` is the minisign signature the website requires as `signatureUrl` → `release.json` points to it with an HTTPS URL, and `getReleaseArtifact` returns the Linux download. Test in Task 1 (shape) and Task 4 (website).
4. The workflow runs on a fork or a non-`main` ref → it refuses (environment `desktop-release` protection plus an explicit ref check). Test in Task 2 by a `workflow-lint` assertion in `test_desktop_publish.py` that parses the YAML.
5. Full-history secret scan finds a real key → the plan stops; the key is rotated before the repo goes public (documented in Task 5, not automated).

---

### Task 1: `scripts/desktop-publish.py`

**Files:**
- Create: `scripts/desktop-publish.py`, `scripts/tests/test_desktop_publish.py`
- Modify: `Makefile` (no change needed: the `test_desktop_*.py` pattern already picks up the new file; verify)

**Interfaces:**
- Consumes: `accepted(receipt, revision)`, `verify_artifacts(receipt, version, artifacts)`, `manifest(version, base_url, artifacts, notes, timestamp)`, `REQUIRED_GATES`, `REQUIRED_TARGETS` from `scripts/desktop-update-manifest.py` (load with `importlib`, as `test_desktop_packaging.py` does); the candidate layout from `scripts/desktop-release.py` (`candidate.json` per platform, `legacy-sparkle/` and `appcast.xml` for Mac).
- Produces:
  - CLI: `desktop-publish.py <version> --revision <sha> --acceptance <receipt.json> --candidates <dir> --notes <notes.md> --download-base https://github.com/ikbenignace/pennel/releases/download/v<version>/ --output <dir>`.
  - `def collect(candidates: Path) -> dict[str, Path]`: maps the website keys `darwin-aarch64-dmg`, `windows-x86_64-nsis`, `windows-x86_64-msi`, `linux-x86_64-deb`, `linux-x86_64-appimage` and the updater targets `darwin-aarch64` (`.app.tar.gz`), `windows-x86_64` (NSIS `.exe`), `linux-x86_64` (`.AppImage`) to files. Exactly one match per key, else `ValueError`.
  - `def platform_gates(receipt) -> dict[str, bool]`: `{"mac": macos, "windows": windows, "linux": linux_x11 and linux_wayland}`, each `== "passed"`.
  - `def release_record(version, date, notes_url, download_base, files, gates) -> dict`: the `Release` shape of `pennel-cloud/apps/web/lib/site.ts` with `artifacts` per key: `{url, sha256, status: "published", validated: True, signing, notarized (mac only, True), signatureUrl (linux only, the `.sig` next to it)}`; Linux artifacts also get `desktopValidation: {x11: True, wayland: True}`. Platforms whose gate is not passed are omitted. Legacy top-level `dmgUrl`, `sha256`, `notarized` mirror the Mac DMG when Mac passed; when Mac did not pass, the function raises (the site's legacy fields require a Mac DMG).
  - `def sha256sums(files) -> str`: `"<hex>  <name>\n"` lines, sorted by name.
  - `def publish(args) -> None`: checks `accepted` (all gates; used only for `latest.json`), runs `verify_artifacts` for the three updater targets, verifies that every published file's hash matches the per-platform `candidate.json` receipt, then writes `SHA256SUMS`, `release.json`, `appcast.xml` (copied from the Mac candidate), `latest.json` (only if every gate passed; otherwise prints `Updater feed withheld: <gates>` and writes none) and `assets/` (hard links or copies of the files to upload).

- [ ] **Step 1: Write failing tests** in `scripts/tests/test_desktop_publish.py` (temporary directories with fake artifacts and `.sig` files; base64 signatures as in `test_manifest_requires_both_targets_and_signed_artifacts`):
  - `test_collect_requires_exactly_one_file_per_key`.
  - `test_release_record_omits_platform_without_passed_gate` (Review Focus 1, part one).
  - `test_latest_json_withheld_unless_every_gate_passed` (Review Focus 1, part two).
  - `test_hash_mismatch_writes_nothing` (Review Focus 2: the output directory stays empty).
  - `test_linux_artifacts_carry_https_signature_url` (Review Focus 3).
  - `test_sha256sums_sorted_and_two_spaces`.
  - `test_record_without_mac_is_rejected`.
- [ ] **Step 2:** Run `make packaging-test`. Expected: FAIL.
- [ ] **Step 3:** Implement. Reuse `desktop-update-manifest.py` functions; do not copy them.
- [ ] **Step 4:** Run `make packaging-test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(release): assemble an accepted desktop release without rebuilding`.

### Task 2: `desktop-publish` workflow

**Files:**
- Create: `.github/workflows/desktop-publish.yml`
- Modify: `scripts/tests/test_desktop_publish.py` (workflow assertions), `docs/releasing.md`

**Interfaces:**
- Consumes: Task 1 CLI; artifacts named `pennel-<target>-signed-candidate` from `.github/workflows/desktop-candidate.yml`.
- Produces: `workflow_dispatch` with inputs `version`, `candidate_run_id`, `revision`, `acceptance` (the receipt JSON as a string; written to `$RUNNER_TEMP/acceptance.json`), `notes` (Markdown). One job on `ubuntu-latest`, `environment: desktop-release`, `permissions: contents: write`, `if: github.ref == 'refs/heads/main'`. Steps:
  1. `actions/download-artifact@v5` with `run-id: ${{ inputs.candidate_run_id }}`, `github-token: ${{ github.token }}`, `path: candidates/`.
  2. `python3 scripts/desktop-publish.py …` → `build/publish/`.
  3. `gh release create v<version> --draft --target <revision> --title "Pennel <version>" --notes-file notes.md build/publish/assets/* build/publish/SHA256SUMS`.
  4. Upload `build/publish/{latest.json,appcast.xml,release.json}` as artifact `pennel-<version>-site-metadata`.
  5. If secret `PENNEL_CLOUD_PR_TOKEN` is set: check out `ikbenignace/pennel-cloud`, copy `latest.json` → `apps/web/public/updates/latest.json`, `appcast.xml` → `apps/web/public/appcast.xml`, `release.json` → `apps/web/content/release.json`, and open a PR `chore(release): Pennel <version>` with `gh pr create`. Otherwise skip with a notice.

- [ ] **Step 1: Write failing tests** in `test_desktop_publish.py` that parse the YAML with a tiny line-based check (no PyYAML dependency): `test_publish_workflow_is_manual_main_only_and_draft` (contains `workflow_dispatch`, `refs/heads/main`, `environment: desktop-release`, `--draft`, and no `--latest` or `gh release edit`; Review Focus 4), `test_publish_workflow_never_echoes_secrets` (no `echo` line references `secrets.`).
- [ ] **Step 2:** Run `make packaging-test`. Expected: FAIL.
- [ ] **Step 3:** Write the workflow. In `docs/releasing.md`, add a section "Gedeelde app publiceren" with the order: candidate run → device acceptance (fill `scripts/desktop-acceptance.example.json`) → `desktop-publish` → review and publish the draft on GitHub → merge the `pennel-cloud` PR and promote production. Mark the Sparkle-only `scripts/release.sh` flow as native-reference only.
- [ ] **Step 4:** Run `make packaging-test`. Expected: PASS.
- [ ] **Step 5:** Commit `ci(release): publish an accepted candidate as a draft release`.

### Task 3: Windows signing command

**Files:**
- Modify: `scripts/desktop-release.py` (`release_config`, Windows branch), `scripts/tests/test_desktop_packaging.py`, `.github/workflows/desktop-candidate.yml` (Windows step env), `docs/releasing.md`

**Interfaces:**
- Consumes: the existing `PENNEL_WINDOWS_SIGN_COMMAND` contract (`%1` = file to sign) and `verify_windows_signature`.
- Produces: when `PENNEL_WINDOWS_SIGN_COMMAND` is unset and `AZURE_ARTIFACT_SIGNING_ENDPOINT`, `AZURE_ARTIFACT_SIGNING_ACCOUNT` and `AZURE_ARTIFACT_SIGNING_PROFILE` are set, `release_config` builds the sign command for `signtool sign /v /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 /dlib <Azure.CodeSigning.Dlib.dll> /dmdf <metadata.json> %1`, writing `metadata.json` (endpoint, account, profile; no secrets) to the temporary runner directory. Authentication uses the `AZURE_CLIENT_ID`, `AZURE_TENANT_ID` and `AZURE_CLIENT_SECRET` environment variables, read by the dlib itself and never passed on the command line. If neither path is configured, the existing error stays (`test_windows_signing_cannot_be_silently_skipped`).

- [ ] **Step 1: Write failing tests:** `test_artifact_signing_builds_command_without_secrets` (the command contains `%1` and `/dmdf`, and no value of `AZURE_CLIENT_SECRET`), `test_explicit_sign_command_wins_over_artifact_signing`.
- [ ] **Step 2:** Run `make packaging-test`. Expected: FAIL.
- [ ] **Step 3:** Implement. In the workflow's Windows step, add the three `AZURE_ARTIFACT_SIGNING_*` variables (`vars.`) and the three `AZURE_*` credentials (`secrets.`), plus a step that installs the signing dlib from its NuGet package into `$RUNNER_TEMP`. Document the one-time Azure setup in `docs/releasing.md` (account, certificate profile, an app registration with the "Artifact Signing Certificate Profile Signer" role).
- [ ] **Step 4:** Run `make packaging-test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(release): sign Windows candidates through Azure Artifact Signing`.

### Task 4: Website reads `release.json` and shows every accepted platform

**Repo:** `pennel-cloud`.

**Files:**
- Create: `apps/web/content/release.json` (initially the JSON literal `null`)
- Modify: `apps/web/lib/site.ts` (`latestRelease` reads `content/release.json` at build), `apps/web/app/[lang]/download/page.tsx`, `apps/web/lib/messages/{en,nl,fr,de}.ts` (`download` section), `apps/web/lib/site-release.test.ts`, `apps/web/lib/structured-data.ts` only if `downloadUrl` reads `dmgUrl` directly

**Interfaces:**
- Consumes: `Release`, `ReleaseArtifactKey`, `getReleaseArtifact` (unchanged); the Task 1 `release.json`.
- Produces: `SITE.latestRelease = parseRelease(releaseJson)`, where `parseRelease(value: unknown): Release | null` validates the shape and returns `null` for anything invalid (a broken file must never show a broken button). The download page renders one block per platform (Mac, Windows, Linux) using `getReleaseArtifact`; a platform without a download keeps today's "Coming soon" badge and waitlist. Copy (nl): `Download voor Mac`, `Download voor Windows`, `Download voor Linux (AppImage)`, `Debian-pakket (.deb)`, `Controlesom (SHA-256)`; en/fr/de translated in the same style (catalogue parity test).

- [ ] **Step 1: Write failing tests** in `site-release.test.ts`: `parseRelease rejects a malformed file` (returns null), `windows artifact without passed gate is not offered` (absent key → `getReleaseArtifact` null), and a render test of the download page with a fixture where only Mac and Linux are present (Windows still shows the badge). Review Focus 3: the Linux fixture produced by Task 1 yields a download.
- [ ] **Step 2:** Run `npm test`. Expected: FAIL.
- [ ] **Step 3:** Implement. Keep the "Only claim what ships" banned-phrase test green: Windows/Linux become "available" only through `getReleaseArtifact`.
- [ ] **Step 4:** Run `npm run lint && npm run typecheck && npm test && npm run build`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(web): download page offers every accepted platform from release.json`.

### Task 5: Public repository readiness

**Files:**
- Create: `SECURITY.md`, `.github/workflows/secrets.yml`, `.gitleaks.toml` (only if allow-listing is needed for test fixtures such as fake base64 signatures)
- Modify: `CONTRIBUTING.md`, `README.md` (a "Download" section linking `https://pennel.app/download`), `docs/open-items.md` (§4)

**Interfaces:**
- Produces: CI job `secrets` running `gitleaks detect --redact --no-banner` on every push and PR (pinned gitleaks release, downloaded with checksum verification, not a third-party action with write permissions).

- [ ] **Step 1:** Run once, locally, over every branch: `gitleaks detect --redact --log-opts="--all"`. Record the outcome (count only, never the matches) in the PR description. Any real secret → stop, rotate it, then continue (Review Focus 5).
- [ ] **Step 2:** Rewrite `CONTRIBUTING.md` for the shared app:
  - Layout: `core/`, `apps/desktop/`, `apps/macos-platform/`, and `apps/macos/` as the reference app.
  - The checks from `AGENTS.md` "Build and test both".
  - UI parity and Dutch-first copy.
  - Tests first: Rust unit tests in `core/`, Rust/TS tests in `apps/desktop`, pure Swift logic in `apps/macos-platform` tests.
  - Remove "no web views" (Tauri uses the system WebView) and keep "no Electron".
- [ ] **Step 3:** `SECURITY.md`: report to `security@pennel.app`, what is in scope (the app, core and the update channel), expect an answer within 7 days, no bug bounty.
- [ ] **Step 4:** Add the `secrets` workflow; run `make packaging-test` and the existing checks. Expected: PASS.
- [ ] **Step 5:** `docs/open-items.md` §4: mark the publication tooling as built, pending Developer ID, the Windows certificate and acceptance.
- [ ] **Step 6:** Commit `docs: contributing and security policy for the public repository` and `ci: scan for leaked secrets`.
