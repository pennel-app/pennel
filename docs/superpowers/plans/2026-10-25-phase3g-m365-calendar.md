# Fase 3g — Microsoft 365-agenda — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `pennel-core` can sign in to Microsoft 365 with the device-code flow, list calendars and pick the current event from Microsoft Graph; the shared app uses it as the calendar source on Windows and Linux (and optionally on macOS), with the same rules UI as the Mac calendar. EventKit and Graph share one "current event" rule in the core.

**Architecture:** A new core module `calendar` holds the platform-neutral model (`CalendarEvent`, `CalendarInfo`) and `pick_current`. `calendar::graph` is a blocking `ureq` client with injectable base URLs (login and Graph), so tests run against mockito. The refresh token lives in the host credential store through the `SecretStore` trait from Pennel Cloud v1. The Tauri host keeps the JSON shape of today's `calendar_snapshot` (`{has_access, calendars, current_event}`) and only changes where it comes from, so `call_detection::offer` and `decide_recording` stay as they are. The Swift bridge returns raw EventKit events; Rust picks.

**Tech Stack:** Rust (core edition as-is), ureq 3, serde_json, base64 (already added by Pennel Cloud v1 core Task 1), uniffi 0.32, mockito; Tauri 2; TypeScript (Vite, `h()` helper, node test runner); Swift (EventKit) in `apps/macos-platform` and `apps/macos`.

**Repo:** `<app-checkout>`, branch `feat/m365-calendar` from `main`. **Precondition:** `SecretStore` (`core/src/cloud/mod.rs`, Pennel Cloud v1 core Task 1, commit `d52e219` on `feat/pennel-cloud-v1`) and `PennelEngine::set_secret_store` are on `main`. If not, cherry-pick that commit first and say so in the PR.

**Spec:** `docs/superpowers/specs/2026-10-25-phase3g-m365-calendar-design.md`.

## Global Constraints

- Scopes are exactly `openid profile offline_access Calendars.Read`. No other Graph permission, no `body` field, no mail endpoints.
- Never log or put into errors: event titles, attendee names or addresses, calendar names, account names, tokens, device codes. Logs may hold HTTP status, AADSTS error numbers, counts and durations.
- The refresh token is stored only through `SecretStore`, account `m365-refresh`. Access tokens and the event cache live only in memory.
- A Graph failure or a timeout (5 s) during call detection yields "no calendar data", never an error that blocks the prompt.
- Dutch user-facing strings exactly as written in this plan; brand values from `assets/brand/brand.md`.
- Login base `https://login.microsoftonline.com/common/oauth2/v2.0`, Graph base `https://graph.microsoft.com/v1.0`; both injectable in tests. Client ID constant `M365_CLIENT_ID` in `core/src/calendar/graph.rs`, overridable with env `PENNEL_M365_CLIENT_ID`; empty means "not configured".
- `cd core && cargo fmt -- --check && cargo clippy --lib --tests -- -D warnings && cargo test`, `make core && make test && make app`, `cd apps/desktop && npm ci && npm run build && npm test`, `cd apps/desktop/src-tauri && cargo test`, `swift test --package-path apps/macos-platform`.

## Review Focus

1. Two overlapping events, an older one with a Teams link that started 50 minutes ago and a new one with a Teams link starting now → the new one is picked. Test in Task 1.
2. A declined or cancelled event, an all-day event and one marked `free` are never picked, even when they are the only events. Test in Task 1.
3. A refresh token rotated by Microsoft on refresh is stored; two concurrent `current_event` calls refresh once (a mutex, as in `cloud::account::Session`). Test in Task 2.
4. A tenant that requires admin approval (`AADSTS90094` in `error_description` during polling) → `M365LoginPoll::AdminConsentRequired { url }`, with the admin-consent URL for this client ID; no secret or code in it. Test in Task 2.
5. Graph slow (6 s) during `offer` → the record prompt still appears without a title, in under 6 s total. Test in Task 4.
6. Switching source on macOS from Microsoft 365 back to Mac-agenda → no Graph calls are made afterwards (the cache and timer are idle). Test in Task 4.

---

### Task 1: Calendar model, settings and `pick_current`

**Files:**
- Create: `core/src/calendar/mod.rs`
- Modify: `core/src/lib.rs` (module + re-exports), `core/src/settings.rs` (`calendar_source`), `core/src/rules.rs` (extend `old_settings_get_detection_defaults`)

**Interfaces:**
- Produces:
  - `#[derive(uniffi::Enum, Serialize, Deserialize)] #[serde(rename_all = "snake_case")] pub enum CalendarSource { System, Microsoft365 }`.
  - `Settings.calendar_source: CalendarSource` with `#[serde(default = "default_calendar_source")]`: `System` on macOS, `Microsoft365` elsewhere.
  - `#[derive(uniffi::Record)] pub struct CalendarInfo { pub id: String, pub name: String }`.
  - `#[derive(uniffi::Record)] pub struct CalendarEvent { pub title: String, pub calendar_id: String, pub calendar_name: String, pub start_ms: i64, pub end_ms: i64, pub all_day: bool, pub cancelled: bool, pub declined: bool, pub free: bool, pub has_meeting_link: bool, pub attendees: Vec<String> }`.
  - `pub fn pick_current(events: &[CalendarEvent], now_ms: i64) -> Option<CalendarEvent>` (exported as `#[uniffi::export] pub fn pick_current_event(events: Vec<CalendarEvent>, now_ms: i64) -> Option<CalendarEvent>` for the hosts). Rule:
    - Skip events that are all day, cancelled, declined or free.
    - Keep events where `start_ms - 5 min <= now_ms <= end_ms`.
    - Prefer `has_meeting_link`.
    - Then prefer the greatest `start_ms` (the most recent start).
  - `pub fn has_meeting_link(text: &str) -> bool` with the hints from `CalendarProvider.swift`: `teams.microsoft.com`, `zoom.us`, `meet.google.com`, `webex.com`, `whereby.com`; case-insensitive.

- [ ] **Step 1: Write failing tests** in `calendar/mod.rs`:
  - `picks_most_recent_start_among_linked_overlaps` (Review Focus 1).
  - `skips_all_day_cancelled_declined_and_free` (Review Focus 2).
  - `five_minute_lead_counts_as_current`.
  - `linked_event_beats_unlinked_event`.
  - `meeting_link_hints_are_case_insensitive`.
  - `calendar_source_defaults_per_platform`, plus an extension of `old_settings_get_detection_defaults` that removes `calendar_source` too and asserts the platform default.
- [ ] **Step 2:** Run `cd core && cargo test calendar`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the core tests, then `make core && make test && make app` (the Swift reference app must still compile with the new `Settings` field). Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): calendar model and one rule for the current event`.

### Task 2: Microsoft 365 login and token handling

**Files:**
- Create: `core/src/calendar/graph.rs`
- Modify: `core/src/calendar/mod.rs`, `core/src/error.rs`, `core/src/engine.rs` (field + exported methods), `core/src/lib.rs`

**Interfaces:**
- Consumes: `SecretStore` (`core/src/cloud/mod.rs`), `http::agent` (`core/src/http.rs`).
- Produces:
  - `PennelError::Calendar { message: String }`: a Dutch message, safe to show (it never contains account or event data).
  - `pub struct M365 { login_base: String, graph_base: String, client_id: String, secrets: Arc<dyn SecretStore>, http: ureq::Agent, access: Mutex<Option<AccessToken>>, cache: Mutex<Option<(Instant, Vec<CalendarEvent>)>> }` with `M365::new(secrets)` (production bases) and `M365::with_bases(login, graph, client_id, secrets)` (tests).
  - `#[derive(uniffi::Record)] pub struct M365LoginStart { pub user_code: String, pub verification_uri: String, pub device_code: String, pub interval_secs: u32, pub expires_in_secs: u32 }`.
  - `#[derive(uniffi::Enum)] pub enum M365LoginPoll { Pending, SlowDown, Expired, Declined, AdminConsentRequired { url: String }, Done { account: String } }`.
  - Methods:
    - `login_start() -> Result<M365LoginStart>`: `POST {login}/devicecode` with form `client_id`, `scope`.
    - `login_poll(device_code) -> Result<M365LoginPoll>`: `POST {login}/token` with `grant_type=urn:ietf:params:oauth:grant-type:device_code`.
      - Error mapping: `authorization_pending` → Pending, `slow_down` → SlowDown, `expired_token` and `bad_verification_code` → Expired, `authorization_declined` → Declined.
      - An `error_description` containing `AADSTS65001`, `AADSTS90094` or `AADSTS90095` → `AdminConsentRequired { url: "https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id={id}&scope=https://graph.microsoft.com/Calendars.Read" }`.
      - Success: store `refresh_token`, keep the access token with its `expires_in`, and read `preferred_username` from the id-token payload (base64url, no verification, used only for display) into `SecretStore` account `m365-account`.
    - `account() -> Option<String>`.
    - `logout()`: clears `m365-refresh`, `m365-account`, the access token and the cache.
    - Crate-internal `access_token() -> Result<String>`: refresh with `grant_type=refresh_token` when fewer than 60 s are left. Store the rotated refresh token. On `invalid_grant`, sign out and return `Calendar { message: "Je Microsoft 365-sessie is verlopen. Log opnieuw in." }`.
  - Exported on `PennelEngine` (blocking): `m365_login_start`, `m365_login_poll(device_code)`, `m365_account() -> Option<String>`, `m365_logout()`, `m365_configured() -> bool` (client ID not empty). The engine creates `M365` lazily from the secret store set by `set_secret_store`; without a store, every method returns `Calendar { message: "Microsoft 365 is in deze build niet ingesteld." }`.

- [ ] **Step 1: Write failing tests** (mockito for login and Graph, `cloud::tests::MemorySecrets`):
  - `device_code_start_sends_minimal_scope` (asserts the form body scope string exactly).
  - `poll_maps_pending_slow_down_expired_declined`.
  - `poll_done_stores_refresh_token_and_account_name` (an id token built in the test with `{"preferred_username":"jan@example.com"}`).
  - `admin_consent_error_returns_url_without_secrets` (Review Focus 4).
  - `refresh_rotates_and_is_single_flight` (Review Focus 3: two threads, mock expects exactly one refresh call).
  - `invalid_grant_signs_out`.
  - `unconfigured_client_id_reports_dutch_message`.
- [ ] **Step 2:** Run `cargo test calendar::graph`. Expected: FAIL.
- [ ] **Step 3:** Implement. Follow `cloud::account::Session` for the single-flight refresh (hold the access mutex across the refresh request).
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): Microsoft 365 device-code login`.

### Task 3: Calendars and current event from Graph

**Files:**
- Modify: `core/src/calendar/graph.rs`, `core/src/engine.rs`

**Interfaces:**
- Produces:
  - `M365::calendars() -> Result<Vec<CalendarInfo>>`: `GET {graph}/me/calendars?$select=id,name&$top=50`.
  - `M365::events(now_ms, timeout) -> Result<Vec<CalendarEvent>>`: for at most the first 10 calendars, `GET {graph}/me/calendars/{id}/calendarView?startDateTime=…&endDateTime=…&$select=subject,start,end,isAllDay,isCancelled,showAs,responseStatus,onlineMeeting,location,attendees&$top=100` (window −12 h…+12 h, ISO UTC), with header `Prefer: outlook.timezone="UTC"`.
    - Mapping:
      - `subject` → title, or `Meeting` if empty.
      - `isCancelled` → cancelled; `responseStatus.response == "declined"` → declined; `showAs == "free"` → free.
      - `has_meeting_link`: `onlineMeeting.joinUrl` is present, or `has_meeting_link(location.displayName)`.
      - Attendees: `emailAddress.name` for each attendee whose address is not the signed-in account's (case-insensitive), names that are not empty, deduplicated.
    - The result is cached for 5 minutes.
  - Exported on `PennelEngine`:
    - `m365_calendars() -> Result<Vec<CalendarInfo>>`.
    - `m365_current_event(now_ms: i64, timeout_ms: u32) -> Option<CalendarEvent>`: never `Err`. Errors are logged as status only and give `None`.
    - `m365_clear_cache()`.

- [ ] **Step 1: Write failing tests:**
  - `calendar_view_request_selects_only_needed_fields` (asserts the `$select` value and the `Prefer` header, and that `body` is absent).
  - `maps_graph_event_to_calendar_event` (fixture JSON with a declined attendee response, a Teams `joinUrl`, and the user among the attendees → removed).
  - `events_cached_for_five_minutes` (a second call makes no request).
  - `current_event_timeout_returns_none` (mock with a delay longer than `timeout_ms`).
  - `graph_401_after_refresh_returns_none_and_logs_status_only`.
  - `at_most_ten_calendars_queried`.
- [ ] **Step 2:** Run `cargo test calendar::graph`. Expected: FAIL.
- [ ] **Step 3:** Implement with `pick_current` from Task 1.
- [ ] **Step 4:** Run the full core checks. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): read calendars and the current event from Microsoft Graph`.

### Task 4: Desktop host — source-aware calendar, commands and capabilities

**Files:**
- Modify: `apps/desktop/src-tauri/src/call_detection.rs` (`calendar()` → `calendar(shared: &Shared)`, with the source switch), `apps/desktop/src-tauri/src/platform_commands.rs` (`calendar_snapshot` takes `State<Arc<Shared>>`), `apps/desktop/src-tauri/src/commands.rs` (new commands; `capabilities::snapshot` call at l.117), `apps/desktop/src-tauri/src/capabilities.rs` (`snapshot` gains `m365: M365State`), `apps/desktop/src-tauri/src/main.rs` (`generate_handler!` near l.661), `apps/desktop/src-tauri/src/secrets.rs` (only if the `SecretStore` impl from Pennel Cloud v1 desktop Task 1 is not on `main` yet)

**Interfaces:**
- Consumes: Task 2–3 engine methods; `Secrets` as `SecretStore`.
- Produces:
  - `call_detection::calendar(shared) -> serde_json::Value` with the unchanged shape `{has_access, calendars: [{id, name}], current_event: {title, calendar_id, calendar_name, attendees} | null}`. When `settings.calendar_source == Microsoft365`, it uses `m365_calendars` and `m365_current_event(now, 5000)`, and `has_access` = signed in. Otherwise it keeps today's macOS EventKit path, and on other platforms the empty value.
  - Tauri commands: `m365_login_start`, `m365_login_poll`, `m365_logout`, `m365_status -> { configured: bool, account: Option<String> }`. Each runs in `spawn_blocking`.
  - `capabilities::snapshot(hotkey_error, m365: M365State)`, where `pub struct M365State { pub configured: bool, pub signed_in: bool, pub source: CalendarSource }`:
    - Not configured → `Unsupported("Microsoft 365 is in deze build niet ingesteld.")`.
    - `source == Microsoft365` and signed in → `Available`.
    - `source == Microsoft365` and signed out → `PermissionRequired` with reason `"Log in bij Microsoft 365 om agendagegevens te gebruiken."`.
    - `source == System` on macOS → today's EventKit permission path.
  - Remove the Windows/Linux reason `"Agendagegevens zijn voorlopig alleen beschikbaar via de macOS-agenda."`.

- [ ] **Step 1: Write failing tests** (Rust unit tests in `capabilities.rs` and `call_detection.rs`, with an engine whose `M365` points at a mockito server through `PENNEL_M365_CLIENT_ID` plus injected bases, via a `#[cfg(test)]` constructor on the engine):
  - `calendar_capability_follows_m365_state` (all four states).
  - `snapshot_shape_unchanged_for_m365_source`.
  - `slow_graph_does_not_delay_prompt_past_timeout` (Review Focus 5).
  - `system_source_makes_no_graph_calls` (Review Focus 6: mockito expects 0 calls).
- [ ] **Step 2:** Run `cd apps/desktop/src-tauri && cargo test calendar`. Expected: FAIL.
- [ ] **Step 3:** Implement. Clear the M365 cache when `calendar_source` changes in `save_settings`.
- [ ] **Step 4:** Run `cargo test` in `src-tauri`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(desktop): Microsoft 365 as calendar source on every platform`.

### Task 5: Agenda section in the shared UI

**Files:**
- Create: `apps/desktop/src/calendar-source.ts` (pure helpers), `apps/desktop/tests/calendar-source.test.mjs`
- Modify: `apps/desktop/src/views/settings.ts` (`recordingSettingsView`, section "Agenda" around l.159), `apps/desktop/src/api.ts` (`Settings.calendar_source`, the four commands, `M365LoginStart`, `M365LoginPoll`), and reuse `pollLogin` from `apps/desktop/src/cloud-login.ts` (Pennel Cloud v1 desktop Task 2). If that file is not on `main` yet, create it with the exact signature from that plan and its tests.

**Interfaces:**
- Produces:
  - `sourceOptions(platform: "macos" | "windows" | "linux"): [CalendarSource, string][]`: `[["system","Mac-agenda"],["microsoft365","Microsoft 365"]]` on macOS, and `[]` elsewhere (no selector).
  - `agendaState(settings, status, capabilities) -> "system" | "m365_unconfigured" | "m365_signed_out" | "m365_signed_in"`.
  - The section, in the order and with the copy of spec §5:
    - Selector `Bron` (macOS only).
    - Signed out: the text `Geeft meetings de titel en deelnemers uit je Outlook-agenda. Pennel leest alleen je agenda, nooit je mail.` and the button `Inloggen bij Microsoft 365`.
    - Code screen: `Je browser is geopend. Vul daar deze code in:`, the buttons `Browser opnieuw openen` and `Annuleren`. On expiry `De code is verlopen.` with `Opnieuw beginnen`.
    - Signed in: `Verbonden als {account}` with `Afmelden`.
    - Admin consent: `Je organisatie moet Pennel eerst goedkeuren. Stuur deze link naar je IT-beheerder.` with `Link kopiëren`.
    - The toggle `Agenda gebruiken` and the calendar rule rows stay as today, fed by `api.calendar()`.
    - Declined: `Je hebt de toegang geweigerd.` with `Opnieuw beginnen`.

- [ ] **Step 1: Write failing tests** in `calendar-source.test.mjs`:
  - `sourceOptions` per platform.
  - `agendaState` for all four states.
  - `pollLogin` honours `AdminConsentRequired` as a terminal state (extend the helper with an `isTerminal` option if the Pennel Cloud version lacks it, and keep its existing tests green).
- [ ] **Step 2:** Run `npm test`. Expected: FAIL.
- [ ] **Step 3:** Implement. Opening the verification URL uses the existing opener command.
- [ ] **Step 4:** Run `npm run build && npm test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(desktop): Microsoft 365 sign-in in the Agenda settings`.

### Task 6: EventKit returns raw events; the core picks

**Files:**
- Modify: `apps/macos-platform/Sources/PennelMacPlatform/CalendarProvider.swift` (`events(now:)` returns raw items with `allDay`, `cancelled` (`event.status == .canceled`), `declined` (current user's `participantStatus == .declined`), `free` (`event.availability == .free`), `hasMeetingLink`, `startMs`, `endMs`), `apps/macos-platform/Sources/PennelMacPlatform/Bridge.swift` (`pennel_mac_calendar_snapshot` returns `events` instead of `current_event`), `apps/macos-platform/Tests/…` (bridge JSON test), `apps/desktop/src-tauri/src/macos.rs` (`calendar_snapshot` maps `events` → `Vec<CalendarEvent>` → `pennel_core::pick_current_event` → the unchanged `current_event` shape)

**Interfaces:**
- Consumes: `pick_current_event` (Task 1).
- Produces: the same `calendar_snapshot` JSON for the host as before; selection logic only in Rust.

- [ ] **Step 1: Write failing tests:** a Swift test that the bridge JSON contains the fields above for a synthetic event (pure mapping function, no EventKit store), and a Rust test in `macos.rs` (macOS-only `#[cfg]`) that a two-event `events` array gives the core's pick.
- [ ] **Step 2:** Run `swift test --package-path apps/macos-platform` and `cargo test` in `src-tauri`. Expected: FAIL.
- [ ] **Step 3:** Implement. Remove `currentEvent(now:)` from the platform library once nothing calls it.
- [ ] **Step 4:** Run both. Expected: PASS.
- [ ] **Step 5:** Commit `refactor(macos-platform): EventKit returns events, the core picks the current one`.

### Task 7: Native reference app, docs and parity check

**Files:**
- Modify: `apps/macos/Sources/Pennel/Windows/RecordingPane.swift` (section "Agenda" around l.45), `docs/desktop.md` (the platform table row "Calendar / Apple Speech" at `## Platform behavior`, split into "Calendar" and "Apple Speech"), `docs/open-items.md` (§3: 3g built, pending acceptance; §1: new T-item), `README.md` (only if it lists calendar as Mac-only)

- [ ] **Step 1:** Native: above the toggle, a disabled `Picker("Bron")` with `Mac-agenda` selected and `Microsoft 365` disabled, plus the caption `Microsoft 365: beschikbaar in de nieuwe Pennel-app.` The behaviour of the Mac calendar is unchanged.
- [ ] **Step 2:** Run `make app && make test`. Expected: PASS.
- [ ] **Step 3:** Docs:
  - In the `desktop.md` table, the Calendar row reads: macOS "EventKit or Microsoft 365 (Graph)"; Windows, Linux X11 and Linux Wayland "Microsoft 365 (Graph)".
  - Add a short paragraph: which scopes are used, what is read, that data goes straight to Microsoft and never through Pennel, and the admin-consent link.
  - Add to `open-items.md` §1: `T14 Microsoft 365-agenda op Windows en Linux: inloggen, Teams-call krijgt titel en deelnemers; tenant met admin-goedkeuring`.
- [ ] **Step 4:** Parity screenshots: the Agenda section of the shared app (preview build, macOS and a Windows or Linux build) next to the native pane, using the apps' own windows and never a capture of the user's screen. Check the order, labels and the disabled reason.
- [ ] **Step 5:** Run every check from Global Constraints. Expected: PASS.
- [ ] **Step 6:** Commit `feat(macos): show Microsoft 365 as a disabled calendar source; docs: calendar on every platform`.
