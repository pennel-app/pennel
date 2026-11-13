# Pennel Cloud v1 — deel 2: kern (account, versleuteling, sync, provider) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `pennel-core` can log in to Pennel Cloud, keep the meetings folder in end-to-end-encrypted sync with the server from plan 1, and use Pennel Cloud as a provider without an API key.

**Architecture:** Core stays synchronous (ureq). Secrets are stored by the host through a new foreign trait `SecretStore`. Sync is a single blocking pass, `PennelEngine::cloud_sync_run`, which the host calls on a background thread at start, after its own writes, and every 3 minutes. The pass pulls, then pushes, then reports. The sync algorithm talks to a `SyncApi` trait, so it is unit-tested against an in-memory fake; only the HTTP adapter is tested with mockito.

**Tech Stack:** Rust (core edition as-is), ureq 3, rusqlite, uniffi 0.32; new crates `chacha20poly1305` (XChaCha20-Poly1305), `argon2`, `hmac` (the release built on the same `digest` as `sha2 = 0.11`), `rand` or `getrandom`, `base64`.

**Repo:** `<app-checkout>`, branch `feat/pennel-cloud-v1`.

**Historical context:** The server design and implementation plan are retained in private maintainer records. For current public app integration and compatibility requirements, read [desktop architecture](../../desktop.md) and [device connection](../../device-connection.md).

## Global Constraints

- Never log or put into errors: meeting content, file names, folder names, e-mail addresses, tokens, keys, recovery codes. Logs and errors may hold counts, byte lengths, durations, HTTP status codes.
- The vault key and refresh token live only in the host credential store (via `SecretStore`) and in memory. The recovery code is never stored anywhere.
- Synced: `*.md` and other non-audio files inside meeting folders, and `.templates/*.md`. Not synced: `*.m4a` and any audio, `*.tmp`, `.DS_Store`, `desktop.ini`, `Thumbs.db`, dot-files other than `.templates`, the folder of a meeting that is still recording.
- Local files are never deleted or overwritten without the previous content surviving: either it matches the last-synced hash, or it is moved to the sync trash (`<data_dir>/sync-trash/`, purged after 30 days), or a conflict copy is written.
- Conflict copy name: `<stem> (conflict <device name> <YYYY-MM-DD HHmm>).<ext>`, in the same folder.
- Cloud base URL: `https://api.pennel.app` (release), `https://api-staging.pennel.app` (preview engine), env `PENNEL_CLOUD_URL` overrides both (tests and dev).
- Dutch user-facing strings exactly as written in this plan.
- `cd core && cargo fmt -- --check && cargo clippy --lib --tests -- -D warnings && cargo test`, `make core && make test && make app` (the Swift reference app must still compile), `cd apps/desktop/src-tauri && cargo test`.

## Review Focus

1. The same meeting renamed on device A while device B edits its notes → B ends with the new folder name and its edits intact (as the live file or a conflict copy), and no half-empty old folder remains. Test in Task 5.
2. A first sync on a device that already has meetings, against a server that already has the same meetings from another device → identical files are matched by hash, with no conflict copies and no duplicates. Test in Task 5.
3. A file modified by Obsidian while a pull is writing the same file → the user's edit survives (re-hash right before replacing; on mismatch write a conflict copy instead). Test in Task 5.
4. The server feed returns `cursor_expired` (device offline > 90 days) → a full compare runs, and nothing is deleted locally just because it is absent from the feed. Test in Task 5.
5. A file with a non-UTF-8 name or Unicode in NFD form (macOS) vs NFC (Windows/Linux) → the same key on every platform (normalise to NFC). Test in Task 4.

---

### Task 1: `SecretStore`, errors and the cloud HTTP helpers

**Files:**
- Create: `core/src/cloud/mod.rs`, `core/src/cloud/http.rs`
- Modify: `core/src/lib.rs`, `core/src/error.rs`, `core/Cargo.toml`, `core/src/engine.rs` (constructor + setter)

**Interfaces:**
- Produces:
  - `#[uniffi::export(with_foreign)] pub trait SecretStore: Send + Sync { fn get(&self, account: String) -> Option<String>; fn set(&self, account: String, value: Option<String>); }`. Account names are `pennel-cloud-refresh` and `pennel-cloud-vault-key`.
  - `PennelEngine::set_secret_store(&self, store: Arc<dyn SecretStore>)` (exported).
  - New `PennelError` variants: `NotSignedIn`, `Cloud { code: String, message: String }` (server JSON error; `message` is the server's Dutch text, safe to show because it is ours), `VaultLocked`.
  - `cloud::base_url(preview: bool) -> String`.
  - `cloud::http::send_json(agent, method, url, token: Option<&str>, body: Option<&serde_json::Value>) -> Result<(u16, serde_json::Value), PennelError>`, `send_bytes(...) -> Result<(u16, HeaderMap-lite, Vec<u8>), PennelError>`.
  - `cloud::http::error_from(status, &Value) -> PennelError`: maps `{error, message}` to `Cloud`. A missing `message` falls back to `"Pennel Cloud gaf een fout ({status})."`.

- [ ] **Step 1: Write failing tests** (mockito):
  - `error_from_uses_server_message`.
  - `base_url_respects_env_and_preview`.
  - `secret_store_roundtrip_through_engine`: a test `MemorySecrets` implementing `SecretStore`.
- [ ] **Step 2:** Run `cd core && cargo test cloud::`. Expected: FAIL.
- [ ] **Step 3:** Implement. Add the crates now (used from Task 3 on).
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): secret store hook and Pennel Cloud HTTP helpers`.

### Task 2: Account login, tokens and account calls

**Files:**
- Create: `core/src/cloud/account.rs`
- Modify: `core/src/engine.rs` (exported methods), `core/src/lib.rs` (re-exports)

**Interfaces:**
- Consumes: Task 1; server routes from plan 1 Tasks 2–4.
- Produces (all exported on `PennelEngine`, blocking):
  - `cloud_login_start(device_name: String, platform: String) -> Result<CloudLoginStart, PennelError>`, where `CloudLoginStart { user_code, verification_uri, device_code, interval_secs: u32 }`.
  - `cloud_login_poll(device_code: String) -> Result<CloudLoginPoll, PennelError>`, where `enum CloudLoginPoll { Pending, SlowDown, Expired, Done { email: String } }`. `Done` stores the refresh token via `SecretStore`, keeps the access token in memory, then calls `GET /v1/account` for the e-mail.
  - `cloud_signed_in() -> bool` (a refresh token is present).
  - `cloud_logout()`: best-effort `DELETE /v1/devices/{own id}`, then clear both secrets and the sync state (Task 5's `SyncState::clear`).
  - `cloud_account() -> Result<CloudAccount, PennelError>`, where `CloudAccount { email, audio_seconds: u64, tokens: u64, storage_bytes: u64, limit_audio_seconds: u64, limit_tokens: u64, limit_storage_bytes: u64 }`.
  - `cloud_devices() -> Result<Vec<CloudDevice>, PennelError>`, where `CloudDevice { id, name, platform, last_seen_iso, current: bool }`; also `cloud_rename_device(id, name)`, `cloud_revoke_device(id)`, `cloud_delete_account()` (also clears local cloud state, never local meeting files).
  - Crate-internal `cloud::account::Session::access_token(&self) -> Result<String, PennelError>`: refreshes when there are fewer than 60 s left, and stores the rotated refresh token. A 401 `session_revoked` or `unauthorized` on refresh clears the secrets and returns `NotSignedIn`.

- [ ] **Step 1: Write failing tests** (mockito):
  - `login_poll_pending_then_done_stores_refresh_token`.
  - `poll_slow_down_and_expired_map`.
  - `access_token_refreshes_when_near_expiry_and_rotates` (asserts the store holds the new refresh token).
  - `revoked_session_signs_out`.
  - `logout_clears_secrets_even_if_server_unreachable`.
  - `delete_account_keeps_meeting_files` (tempdir with a meeting → still there).
- [ ] **Step 2:** Run `cargo test cloud::account`. Expected: FAIL.
- [ ] **Step 3:** Implement. Concurrent `access_token` callers must not refresh twice (a mutex around refresh), otherwise the server's reuse detection would revoke the device.
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): Pennel Cloud login and account calls`.

### Task 3: Pennel Cloud as a provider

**Files:**
- Modify: `core/src/provider.rs`, `core/src/engine.rs` (the `keys.get` sites: around l.256, 555, 603, 1051, 1094; `requires_key` checks l.199, 350), `core/src/cli_agent.rs` (exhaustive match if any), `apps/macos/Sources/Pennel/Windows/ProvidersPane.swift` (switches at l.12, l.34; `ProviderKind.all` l.9; not in `addableKinds`)

**Interfaces:**
- Produces:
  - `ProviderKind::PennelCloud` with slug `pennel-cloud`, label `Pennel Cloud`, `default_base_url` = `cloud::base_url(preview) + "/v1"` (the engine fills it at `endpoint()` time; the stored `base_url` is ignored for this kind), `default_model` = `pennel-auto`, `requires_key` false, `supports` all three tasks.
  - `fn credential(&self, account: &ProviderAccount, keys: &ApiKeys) -> Result<Option<String>, PennelError>` in `engine.rs`. This replaces the direct `keys.get(&account)` calls: for `PennelCloud` it returns `Session::access_token()` (or `NotSignedIn`), and for every other kind it is unchanged.
  - Errors from a `PennelCloud` request are mapped with `cloud::http::error_from` instead of `http::map_status`, so the user sees the server's Dutch quota message.
  - The `NotSignedIn` text shown by hosts: `"Log in bij Pennel Cloud in Instellingen → Account."`

- [ ] **Step 1: Write failing tests:**
  - `pennel_cloud_dictation_uses_access_token`: mockito proxy, signed-in session from a `MemorySecrets` with a refresh token, and a mocked refresh endpoint. Assert `Authorization: Bearer <access>`, `model=pennel-auto`, and the URL is `{PENNEL_CLOUD_URL}/v1/audio/transcriptions`.
  - `pennel_cloud_without_login_is_not_signed_in`.
  - `pennel_cloud_quota_error_shows_server_message` (429 body → `Cloud { code: "quota", message }`).
  - `stored_base_url_ignored_for_pennel_cloud`.
- [ ] **Step 2:** Run `cargo test pennel_cloud`. Expected: FAIL.
- [ ] **Step 3:** Implement. The Swift switches get `.pennelCloud` cases: label "Pennel Cloud", no key field, a disabled card with `"Beschikbaar in de nieuwe Pennel-app"`. It is not added to `addableKinds`.
- [ ] **Step 4:** Run the core tests, then `make core && make test && make app`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): Pennel Cloud provider`.

### Task 4: Vault crypto and object keys

**Files:**
- Create: `core/src/cloud/crypto.rs`

**Interfaces:**
- Produces:
  - `pub struct VaultKey([u8; 32])` (zeroise on drop; no `Debug` output of the bytes); `VaultKey::generate()`, `to_b64()` / `from_b64()` (for `SecretStore`).
  - `pub fn new_recovery_code() -> String`: 24 symbols from the Crockford alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, shown as 6 groups of 4 joined by `-`.
  - `pub fn normalise_recovery_code(input: &str) -> Option<String>`: uppercase; drop spaces and dashes; map `O→0`, `I→1`, `L→1`; must be 24 valid symbols.
  - `pub struct Kdf { alg: "argon2id", m_kib: 65536, t: 3, p: 1, salt: b64(16 bytes) }` (serde, the server's `kdf` JSON).
  - `pub fn wrap(key: &VaultKey, code: &str) -> (Vec<u8>, Kdf)` and `pub fn unwrap(wrapped: &[u8], kdf: &Kdf, code: &str) -> Result<VaultKey, PennelError>` (wrong code → `Cloud { code: "recovery_code", message: "Deze herstelcode klopt niet." }`). Wrapped layout: `0x01 || nonce(24) || ciphertext`, associated data `b"pennel-vault-v1"`.
  - `pub fn object_id(key: &VaultKey, object_key: &str) -> String`: HMAC-SHA256 over the NFC-normalised `object_key`, as lowercase hex (64 chars).
  - `pub struct Payload { key: String, folder: Option<String>, content: Vec<u8>, modified_ms: i64, device: String }` (serde JSON; `content` base64).
  - `pub fn seal(key, object_id, generation: u32, &Payload) -> Vec<u8>` and `pub fn open(key, object_id, generation, &[u8]) -> Result<Payload, PennelError>`. Layout `0x01 || nonce(24) || ct`, associated data `object_id bytes || generation as u32 big-endian`.
  - NFC: add `unicode-normalization` (small, pure Rust) and use it in `object_id` and in Task 5's scan.

- [ ] **Step 1: Write failing tests:**
  - `wrap_unwrap_roundtrip` (use cheap Argon2 params behind `#[cfg(test)]` only for speed, plus one test with the real params).
  - `wrong_recovery_code_rejected`.
  - `recovery_code_format_and_normalise` (`"abcd efgh-ijkl…"` style input, `O/0` mix-up).
  - `object_id_is_stable_hex_and_key_dependent`.
  - `object_id_nfc_equals_nfd` (Review Focus 5: `"Café"` in NFC and NFD).
  - `seal_open_roundtrip`.
  - `open_rejects_other_object_id_or_generation`.
  - `open_rejects_tampered_ciphertext`.
- [ ] **Step 2:** Run `cargo test cloud::crypto`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): vault key, recovery code and object encryption`.

### Task 5: Sync engine

**Files:**
- Create: `core/src/cloud/sync/mod.rs` (the pass), `core/src/cloud/sync/api.rs` (`SyncApi` trait + HTTP impl + `FakeApi` under `#[cfg(test)]`), `core/src/cloud/sync/state.rs` (SQLite `<data_dir>/sync.sqlite`), `core/src/cloud/sync/scan.rs`, `core/src/cloud/sync/trash.rs`
- Modify: `core/src/meetings.rs` (`pennel_id` frontmatter, `uid_of`, `ensure_uid`), `core/src/settings.rs` (`cloud_sync: bool`, `#[serde(default)]`), `core/src/engine.rs` (exported methods)

**Interfaces:**
- Consumes: Tasks 1, 2, 4; server sync routes (plan 1 Task 5).
- Produces:
  - `MeetingStore::uid_of(&self, id) -> Option<String>` and `ensure_uid(&self, id) -> Result<String, PennelError>`. `create` writes `pennel_id: <uuid v7>` into `meeting.md`'s frontmatter; existing meetings get one on the first sync pass. The `Frontmatter` struct gains `uid: String`, rendered only when non-empty.
  - `trait SyncApi { fn vault(&self) -> Result<RemoteVault>; fn put_vault(&self, generation, wrapped, kdf, create: bool) -> Result<()>; /* create → header `If-None-Match: *`; 409 `vault_exists` → VaultLocked */ fn reset_vault(&self, generation: u32) -> Result<u32>; /* header X-Pennel-Generation; 409 generation → VaultReset */ fn changes(&self, since: i64, limit: u32) -> Result<ChangePage>; fn get(&self, id) -> Result<Option<(i64, Vec<u8>)>>; fn put(&self, id, base: i64, generation: u32, blob: &[u8]) -> Result<PutResult>; fn delete(&self, id, base: i64, generation: u32) -> Result<PutResult>; }`, where `enum PutResult { Ok(i64), Conflict { current: i64, deleted: bool }, GenerationChanged }` and `ChangePage` is `Ok { changes, cursor, more, generation }` or `CursorExpired`.
  - `SyncState`: tables `entries(key text pk, object_id text, server_version int, synced_hash text, folder text)` and `meta(k, v)` (`cursor`, `generation`); `SyncState::clear()`.
  - Scan output `LocalFile { key: String, rel_path: PathBuf, folder: Option<String>, hash: String /* sha256 hex */ }`. Meeting files are keyed `meeting/<uid>/<file name>`, templates `templates/<file name>`. Names are NFC-normalised. A meeting folder without `meeting.md` is skipped.
  - `trash::move_to_trash(data_dir, meetings_root, rel_path)`; `trash::purge(data_dir, older_than_days: 30)`.
  - Exported on `PennelEngine`:
    - `cloud_sync_enable() -> Result<String /* recovery code */>`: the server has no key → generate, wrap, `put_vault(create = true)` (409 `vault_exists` = another device won the race → `VaultLocked`), store the key, set `cloud_sync = true`. The server already has a key → return `VaultLocked`; the UI then asks for the code.
    - `cloud_sync_unlock(code: String) -> Result<()>`: unwrap, store the key, `cloud_sync = true`, record the generation.
    - `cloud_sync_new_recovery_code() -> Result<String>`: rewrap at the current generation.
    - `cloud_sync_reset() -> Result<String>`: `reset_vault`, new key, new code, clear `SyncState`; the next pass uploads everything.
    - `cloud_sync_disable()`: `cloud_sync = false`; the key stays, state is kept.
    - `cloud_sync_run(recording_meeting_ids: Vec<String>) -> SyncReport`: never `Err`; failures are a status.
    - `cloud_sync_last() -> SyncReport`.
    - `meeting_sync_states() -> Vec<MeetingSyncState>`, with `MeetingSyncState { id, state: SyncItemState }` and `enum SyncItemState { Synced, Pending, Conflict }`. `Conflict` = the folder holds a file whose stem contains ` (conflict `.
    - `cloud_folder_warning() -> Option<String>`: the meetings root is under `~/Library/Mobile Documents`, `~/Library/CloudStorage/`, or a path segment equals `Dropbox`, `OneDrive` (prefix), `iCloudDrive` or `Google Drive` → `"Deze map synct al via {naam}. Dubbel synchroniseren kan conflicten geven."`, where naam is `iCloud`, `Dropbox`, `OneDrive` or `Google Drive`.
  - `SyncReport { status: SyncStatus, uploaded: u32, downloaded: u32, conflicts: u32, last_success_ms: Option<i64> }`, with `enum SyncStatus { Off, SignedOut, NeedsSetup, NeedsRecoveryCode, VaultReset, Synced, Offline, StorageFull { message }, Error { message } }`. `NeedsRecoveryCode` = the server has a key and this device has none. `VaultReset` = the server generation is above the local one: stop, forget the local key, the UI asks for the new code.

The pass (an algorithm the tests do not fully determine, so spelled out here):

```
1. Off/SignedOut/locked checks; purge trash; vault = api.vault(); generation check.
2. Pull: page through changes(cursor). On CursorExpired: treat every remote object as unknown
   (fetch all via changes(0)), never delete locally because something is absent.
   For each change:
     entry = state[object_id]; local = current file (re-hash now)
     deleted:  if local is None -> drop entry
               elif local.hash == entry.synced_hash -> move_to_trash, drop entry
               else -> keep local (it will be pushed against the tombstone version)
     updated:  payload = open(get(id)); if local is None or local.hash == entry.synced_hash
                  or local.hash == hash(payload.content) -> write atomically (tmp+rename,
                  re-hash just before rename; mismatch -> write conflict copy instead)
               else -> write payload as conflict copy, keep local
               meeting folder: locate by uid; missing -> create using payload.folder
               (suffix " (2)" if a different meeting owns that name); present with other
               name and payload is meeting.md -> rename folder to payload.folder
     update entry (server_version, synced_hash = hash written or kept-local's base)
   Save cursor after each page.
3. Push: scan (skip recording ids); for each file whose hash != entry.synced_hash:
     put(object_id, base = entry.server_version or 0); Conflict -> fetch and handle as
     "updated, both changed" above; GenerationChanged -> status VaultReset, stop.
   For each entry whose file vanished (and folder not recording): delete(base).
4. Report counts; last_success_ms on success; transport errors -> Offline.
```

- [ ] **Step 1: Write failing tests** (against `FakeApi` and tempdirs; two "devices" = two tempdirs + two `SyncState`s sharing one `FakeApi`):
  - `new_meeting_reaches_second_device` (all text files arrive; audio does not).
  - `edit_roundtrips_back`.
  - `concurrent_edit_creates_conflict_copy_and_keeps_local` (the copy name matches the Global Constraints pattern; both contents survive).
  - `delete_moves_to_trash_on_other_device`.
  - `locally_modified_file_survives_remote_delete`.
  - `rename_on_a_edit_on_b` (Review Focus 1).
  - `first_sync_matches_identical_files_without_conflicts` (Review Focus 2).
  - `external_edit_during_pull_is_kept` (Review Focus 3: the fake writes to the file between get and rename via a hook).
  - `cursor_expired_does_full_compare_without_deleting` (Review Focus 4).
  - `recording_meeting_is_skipped`.
  - `templates_sync`.
  - `vault_reset_on_other_device_reports_vault_reset`.
  - `storage_full_reported_with_server_message`.
  - `network_error_reports_offline`.
  - `existing_meeting_gets_uid_once`.
  - `folder_warning_detects_cloud_paths`.
  - The HTTP `SyncApi` impl against mockito: `put_sends_if_match_and_generation`, `conflict_409_maps_to_conflict`, `generation_409_maps`, `cursor_expired_410_maps`.
- [ ] **Step 2:** Run `cargo test cloud::sync`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the full core checks (fmt, clippy, test). Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): end-to-end encrypted meeting sync`.
