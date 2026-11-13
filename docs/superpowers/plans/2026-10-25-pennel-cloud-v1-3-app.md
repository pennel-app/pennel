# Pennel Cloud v1 — deel 3: desktopapp (Account-tab, sync-runner, provider) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The shared Tauri app exposes Pennel Cloud: an Account tab (login, sync, recovery code, devices, usage, delete account), a background sync runner, sync state in the meeting list, and Pennel Cloud in Providers. The native Mac reference app shows the same tab disabled.

**Architecture:** The Tauri backend implements core's `SecretStore` with the existing `Secrets` (keyring) and runs `cloud_sync_run` on a dedicated thread. That thread wakes at start, every 3 minutes, on `meeting-finished`, and after its own write commands. Each run emits a `sync-changed` event. The frontend is vanilla TS with the `h()` helper, like the other views.

**Tech Stack:** Tauri 2, Rust, TypeScript (Vite, no framework), node test runner; SwiftUI for the disabled native tab.

**Repo:** `<app-checkout>`, branch `feat/pennel-cloud-v1`, after plan 2 is merged into the branch.

**Historical context:** The Cloud design is retained in private maintainer records. This public app plan keeps its client-side core interfaces below; current behavior is documented in [desktop architecture](../../desktop.md) and [device connection](../../device-connection.md).

## Global Constraints

- Dutch copy exactly as written here; brand values from `assets/brand/brand.md` only, no new colours or sizes.
- Platform parity: identical tab, order, labels and states on macOS, Windows and Linux.
- The recovery code is shown, never written to disk, logs or clipboard history. "Kopiëren" uses the same clipboard-history exclusion the app uses for dictated text on Windows.
- Never log meeting names, e-mail addresses or tokens.
- `cd apps/desktop && npm ci && npm run build && npm test`, `cd apps/desktop/src-tauri && cargo test`, `make core && make test && make app`.

## Review Focus

1. Login started, browser closed, app left on the code screen → the code expires after 15 min and the screen offers "Opnieuw beginnen", without polling forever. Test in Task 2 (the poll-loop helper).
2. Sync run triggered while one is still running (meeting finished + timer) → at most one run at a time, with one queued rerun and no pile-up. Test in Task 1.
3. Quitting the app during a sync → it exits promptly; the next start resumes (state is per file, so a partial pass is safe). Test in Task 1 (the runner stops on shutdown signal).
4. The user deletes their account while sync is on → the runner stops, the tab returns to signed out, and local meetings are untouched. Test in Task 2.
5. The meeting being recorded is never synced mid-recording → the runner passes the active meeting id. Test in Task 1.

---

### Task 1: Backend — secret store, cloud commands and the sync runner

**Files:**
- Create: `apps/desktop/src-tauri/src/cloud.rs`
- Modify: `apps/desktop/src-tauri/src/secrets.rs` (impl `SecretStore`), `apps/desktop/src-tauri/src/state.rs`, `apps/desktop/src-tauri/src/main.rs` (setup + `generate_handler!` at ~l.618), `apps/desktop/src-tauri/src/commands.rs` (`KINDS` at l.52 gets `(ProviderKind::PennelCloud, …)`, array length +1; model-list skip at l.239 also for PennelCloud), `apps/desktop/src-tauri/src/meeting_runtime.rs` (notify the runner after `meeting-finished` ~l.352 and after summarising)

**Interfaces:**
- Consumes: the plan 2 engine API (`set_secret_store`, `cloud_*`, `meeting_sync_states`, `cloud_folder_warning`).
- Produces:
  - Tauri commands (snake_case, JSON-serialisable mirrors of the core records):
    - `cloud_login_start`: device name = OS host name, platform = `macos`, `windows` or `linux`.
    - `cloud_login_poll`, `cloud_logout`, `cloud_account`, `cloud_devices`, `cloud_rename_device`, `cloud_revoke_device`, `cloud_delete_account`.
    - `cloud_sync_enable`, `cloud_sync_unlock`, `cloud_sync_new_recovery_code`, `cloud_sync_reset`, `cloud_sync_disable`, `cloud_sync_now`, `cloud_sync_last`.
    - `meeting_sync_states`, `cloud_folder_warning`, `open_url(url)` (reuse the existing opener if present).
  - `pub struct SyncRunner` with `SyncRunner::start(shared: Arc<Shared>, app: AppHandle) -> SyncRunner`, `poke(&self)` (coalescing: at most one queued rerun) and `stop(&self)`. Each run gets `recording_meeting_ids` from the meeting runtime's active session and emits `sync-changed` with the `SyncReport`. The interval is 180 s, and a pass only runs while `settings.cloud_sync` is on and the user is signed in.
  - The write commands that touch meetings (notes, rename, delete, summary, transcript retry, templates) call `poke()` after success.

- [ ] **Step 1: Write failing tests** (Rust unit tests in `cloud.rs` with a fake run function injected):
  - `poke_while_running_queues_one_rerun` (Review Focus 2: 5 pokes during a run → exactly 2 runs total).
  - `stop_ends_thread_promptly` (Review Focus 3: < 1 s while idle in the interval wait).
  - `active_recording_id_is_passed` (Review Focus 5).
  - `secrets_impl_roundtrip` (uses the existing `Shared::testing` helper's secrets backend).
- [ ] **Step 2:** Run `cd apps/desktop/src-tauri && cargo test cloud`. Expected: FAIL.
- [ ] **Step 3:** Implement. The runner uses a `Condvar` with a timeout for the interval and pokes, plus a stop flag.
- [ ] **Step 4:** Run `cargo test` in `src-tauri`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(desktop): Pennel Cloud commands and background sync`.

### Task 2: Account tab

**Files:**
- Create: `apps/desktop/src/views/account.ts`, `apps/desktop/src/cloud-login.ts` (pure poll-loop and formatting helpers), `apps/desktop/tests/cloud-login.test.mjs`
- Modify: `apps/desktop/src/main.ts` (tabs: insert `["account", "Account", accountView]` as the **last** tab), `apps/desktop/src/api.ts` (typed wrappers for the Task 1 commands and a `sync-changed` listener)

**Interfaces:**
- Consumes: the Task 1 commands.
- Produces:
  - `pollLogin(poll: () => Promise<Poll>, opts: { intervalMs, expiresAtMs, now, sleep }) -> Promise<"done" | "expired" | "cancelled">` (honours `slow_down` by adding 5 s to the interval, stops at expiry). Also `formatUsage(account) -> { audio: string, tokens: string, storage: string }` (e.g. `"3 u 20 min van 20 u"`, `"0,4 mln van 2 mln tokens"`, `"12 MB van 1 GB"`).
  - The view's states and copy (sections in this order):
    1. **Signed out:**
       - Title `Pennel Cloud`.
       - Text `Log in om je meetings op al je toestellen te hebben en Pennel Cloud als provider te gebruiken, zonder eigen API-sleutel.`
       - Button `Inloggen`.
    2. **Code screen:**
       - `Je browser is geopend. Controleer daar deze code:` followed by the code in large type.
       - Buttons `Browser opnieuw openen` and `Annuleren`.
       - On expiry: `De code is verlopen.` with button `Opnieuw beginnen` (Review Focus 1).
    3. **Signed in → Account:** `Ingelogd als {email}` and button `Uitloggen`.
    4. **Signed in → Gebruik deze maand:** three lines from `formatUsage`.
    5. **Signed in → Meetings synchroniseren:**
       - Toggle `Meetings synchroniseren`.
       - Explanation `End-to-end versleuteld: Pennel Cloud kan je meetings niet lezen. Audio blijft op dit toestel.`
       - Status line from `SyncReport`:
         - `Gesynchroniseerd · {relatieve tijd}`
         - `Bezig…`
         - `Offline — probeert later opnieuw`
         - `{server message}` for StorageFull/Error
       - The `cloud_folder_warning` text if present.
    6. **Enable flow:**
       - `NeedsSetup` → shows the recovery code once in 6 groups, with the text `Bewaar deze herstelcode. Je hebt hem nodig om sync op een ander toestel aan te zetten. Zonder deze code en zonder ingelogd toestel kan niemand je clouddata nog lezen, ook wij niet.`, the buttons `Kopiëren` and `Ik heb hem bewaard` (sync starts only after this click), and a `Afdrukken` link (OS print of a plain page).
       - `VaultLocked` / `NeedsRecoveryCode` → input `Herstelcode` with button `Ontgrendelen`, plus a secondary link `Herstelcode kwijt?`. That link explains the consequences and offers `Kluis opnieuw beginnen`, with the confirmation `Dit wist alle meetings in Pennel Cloud. Meetings op je toestellen blijven bestaan en worden opnieuw geüpload. Andere toestellen hebben daarna de nieuwe herstelcode nodig.`
       - `VaultReset` → `De kluis is opnieuw begonnen op een ander toestel. Voer de nieuwe herstelcode in.` with the same input.
    7. **Herstelcode:** button `Nieuwe herstelcode maken` with the confirmation `De oude herstelcode werkt daarna niet meer.`
    8. **Toestellen:** list `{name}` · `{platform}` · `laatst gezien {relatieve tijd}`, with `(dit toestel)` on the current one, rename inline and `Afmelden` on the others. The confirmation reads `Dit toestel kan dan niet meer synchroniseren. Wat het al had, blijft erop staan. Vrees je dat het gestolen is, begin dan de kluis opnieuw.`
    9. **Account verwijderen:** a danger button with the confirmation `Je account en alle versleutelde data in Pennel Cloud worden gewist. Meetings op dit toestel blijven staan.` (Review Focus 4: afterwards the view re-renders signed out and the runner is idle).
  - After the first successful login, if any of the three model slots has no provider, a prompt appears: `Pennel Cloud gebruiken voor {lijst van lege taken}?` with buttons `Ja` and `Nee`. "Ja" adds a Pennel Cloud provider account (if none exists) and fills only the empty slots.

- [ ] **Step 1: Write failing tests** in `tests/cloud-login.test.mjs`:
  - `pollLogin` returns `"done"` after pending ×2.
  - `slow_down` increases the interval.
  - It returns `"expired"` at `expiresAtMs` without further polls (Review Focus 1).
  - Cancel stops polling.
  - `formatUsage` produces the exact strings above for 12000 s / 400000 tokens / 12 MB.
- [ ] **Step 2:** Run `npm test`. Expected: FAIL.
- [ ] **Step 3:** Implement the helpers and the view. Re-render on `sync-changed`.
- [ ] **Step 4:** Run `npm run build && npm test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(desktop): Account tab for Pennel Cloud`.

### Task 3: Sync state in meetings, Pennel Cloud in Providers

**Files:**
- Modify: `apps/desktop/src/views/meetings.ts`, `apps/desktop/src/views/providers.ts`, `apps/desktop/src/api.ts`
- Create: `apps/desktop/src/sync-badge.ts`, `apps/desktop/tests/sync-badge.test.mjs`

**Interfaces:**
- Consumes: `meeting_sync_states`, `sync-changed`, `cloud_account` (signed-in check).
- Produces:
  - `badgeFor(state: "synced" | "pending" | "conflict" | undefined, syncOn: boolean) -> { icon, label } | null`. Sync off → `null`. Labels: `Gesynchroniseerd`, `Wacht op sync`, `Conflictversie`.
  - Meeting list: a small badge per row. Meeting detail with a conflict: a banner `Er is een conflictversie van dit bestand.` with buttons `Bekijken` (opens the conflict file in the existing file-view or reveal-in-folder) and `Verwijderen` (deletes the conflict copy after confirmation, then pokes sync).
  - Providers: a "Pennel Cloud" card without key or URL fields. Signed out, it shows `Log in bij Pennel Cloud` with a button that navigates to the Account tab. Signed in, it shows `Ingelogd als {email}`.
- [ ] **Step 1: Write failing tests:** `badgeFor` covers all four inputs plus sync off.
- [ ] **Step 2:** Run `npm test`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm run build && npm test` and `cargo test` in `src-tauri`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(desktop): sync state in meetings and Pennel Cloud provider card`.

### Task 4: Native reference app, docs and parity check

**Files:**
- Modify: `apps/macos/Sources/Pennel/Windows/SettingsView.swift` (`SettingsTab` l.4, `SegmentTabs` l.16, `switch` l.23), `apps/macos/Sources/Pennel/Windows/Components.swift` (only if `SegmentTabs` needs a disabled style), `docs/desktop.md` (platform table at `## Platform behavior` l.22 and the verification section l.130), `docs/open-items.md` (§3)

- [ ] **Step 1:** Native tab `Account` (last) showing the title `Pennel Cloud` and the disabled text `Beschikbaar in de nieuwe Pennel-app.`. No controls are active.
- [ ] **Step 2:** `make app && make test`. Expected: PASS.
- [ ] **Step 3:** Docs:
  - `desktop.md` gets a "Pennel Cloud" row in the capability table (shared app on every platform; native reference app: disabled) and a short paragraph on what is synced and what the server can see.
  - `open-items.md` §3 marks fase B and sync as built, pending device acceptance.
  - Add acceptance items to §1: T11 `Twee computers, zelfde account: meeting opnemen op de ene, verschijnt op de andere`, T12 `Kluis opnieuw beginnen en herstelcode op een tweede toestel`, T13 `Dicteren en samenvatten via Pennel Cloud zonder eigen sleutel`.
- [ ] **Step 4:** Parity screenshots: render the Account tab of the shared app (preview build) and the native tab side by side, using the app's own windows and never a capture of the user's screen. Check that the order, labels and the disabled state read correctly.
- [ ] **Step 5:** Run every check from Global Constraints. Expected: PASS.
- [ ] **Step 6:** Commit `feat(macos): disabled Account tab; docs: Pennel Cloud on desktop`.
