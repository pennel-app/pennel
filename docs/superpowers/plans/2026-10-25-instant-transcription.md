# Directe transcriptie (live tekst tijdens dicteren en meetings) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** While the user dictates, the pill shows what Pennel hears, using true streaming where the model can, fast windowed or phrase-wise transcription where it cannot, and a disabled control with a Dutch reason otherwise. The final paste stays one clean paste and is never worse than today. Meetings get a display-only preview line for local models and Apple Speech.

**Architecture:**
- New core module `core/src/live/`:
  - A capability resolver picks one `LiveMethod` for the configured dictation model: `Stream`, `Window`, `Phrases` or `Unavailable { reason }`.
  - A `LiveDictation` uniffi object receives audio pieces from the host, drives one strategy on its own worker thread, emits `LivePartial`s through a foreign `LiveListener`, and on `finish()` produces the usual `DictationResult` through the same hallucination filter, cleanup and history as `dictate`.
- The full take is always kept. Any live failure falls back to today's whole-take `stt()`.
- Local streaming uses transcribe-cpp's `Session::stream`. Cloud realtime uses one synchronous `tungstenite` WebSocket per take. Apple uses a new foreign trait `SystemLiveTranscriber`.
- Meetings gain a display-only preview computed by the existing worker when it is idle.

**Tech Stack:** Rust (core edition 2024), transcribe-cpp 0.4.1 (already a dependency), ureq 3, uniffi 0.32. New crates:
- `tungstenite` (sync, `rustls-tls-webpki-roots`, no default features);
- `base64`, if plan 2 of Pennel Cloud has not added it yet.

On top of that: Tauri 2 + TypeScript (desktop), Swift (macOS platform library), and Axum + tokio-tungstenite in `pennel-cloud` (Task 11 only).

**Repos:**
- Tasks 1–10 and 12: `<app-checkout>`, branch `feat/instant-transcription`.
- Task 11: private Cloud implementation record; the public client compatibility dependency remains below.
- Task 12 also depends on plan `2026-10-25-pennel-cloud-v1-2-core.md` Task 3 (`ProviderKind::PennelCloud`) being merged.

**Spec:** `docs/superpowers/specs/2026-10-25-instant-transcription-design.md`. The capability matrix is §4, UX §5, the server §6.

## Global Constraints

- Partial text exists only in core memory, in the `LiveListener` callback, in `emit_to("pill", …)` / `emit_to(<live meeting window>, …)` and in webview memory. It never goes:
  - into `diagnostics::Log`, `eprintln!`, `tracing` or `PennelError` messages;
  - into the history store (`history.rs`);
  - into `transcript.md` (only final segments do);
  - into an app-wide broadcast.
- Logs may hold: the method, ms to the first partial, the partial count, and the fallback category (`network`, `http_<status>`, `model`, `busy`, `rate_limited`).
- Exactly one paste per take, with the final (cleaned) text. A partial with a stale `seq` (take finished, cancelled or superseded) is dropped by the host.
- The full audio of a take is kept until `finish()` returns. If the live path failed at any point, `finish()` transcribes the whole take with `PennelEngine::stt` exactly as `dictate` does today.
- Never more than one decode in flight per take.
  - Window strategy: next decode no earlier than `max(1000 ms, 2 × last decode ms)`.
  - A decode slower than the audio it covers switches the take to pause-only updates.
- One paid provider per take: the live path and the final path never both send the same audio to a paid API, except for the fallback after a live failure.
- API keys and access tokens only in the `Authorization` header of the WebSocket handshake, never in the URL.
- Dutch user-facing strings exactly as written in this plan.
- Checks:
  - `cd core && cargo fmt -- --check && cargo clippy --lib --tests -- -D warnings && cargo test`;
  - `make core && make test && make app` (the Swift reference app must still compile);
  - `cd apps/desktop && npm ci && npm run build`;
  - `cd apps/desktop/src-tauri && cargo test`.
  - Task 11: `cargo test -p pennel-server` and `cargo clippy -p pennel-server --tests -- -D warnings` in `pennel-cloud-v1`.

## Review Focus

1. A sentinel phrase dictated through every strategy (Stream fake, Window fake, Phrases via mockito, Realtime via a local test socket) never appears in captured log output, in error strings, or in the history DB except as the final `raw`/`text`. Test in Tasks 3, 5 and 6.
2. A live failure mid-take (socket closed, 429, model error, `Busy`) → `finish()` still returns the full transcript via the whole-take path, and the history row is identical in shape to a `dictate` row. Test in Tasks 3 and 6.
3. A meeting is recording with the same local model while the user dictates with a streaming local model → no deadlock. Meeting chunks wait and are all transcribed after the take, and `pending()` returns to 0. Test in Task 4.
4. Partials arriving after `finish()`/`cancel()` never render, and the paste count stays 1. Test in Task 8 (desktop) and Task 3 (`seq` monotonic, no callback after `finish` returns).
5. `cancel()` releases the WebSocket, the model lease and the worker thread within 200 ms. Test in Tasks 3, 4 and 6.
6. The final text for Window/Phrases equals the joined per-phrase final decodes, with no duplicated or dropped words at phrase boundaries. Test in Tasks 2 and 3 with a fake transcriber that echoes sample ranges.

---

### Task 1: Capability resolver, catalog streaming flag, setting

**Repo:** musli

**Files:**
- Create: `core/src/live/mod.rs`, `core/src/live/capability.rs`
- Modify: `core/src/catalog.rs` (parse `capabilities.streaming` into `CatalogModel`), `core/src/models.rs` (`LocalModel.streaming: bool`), `core/src/settings.rs` (`live_preview: bool`, `#[serde(default = "enabled")]`, included in `default_settings`), `core/src/engine.rs` (exported method), `core/src/lib.rs`

**Interfaces:**
- Produces:
  - `#[derive(uniffi::Enum)] pub enum LiveMethod { Stream, Window, Phrases, Unavailable }`.
  - `#[derive(uniffi::Record)] pub struct LiveCapability { method: LiveMethod, label: String }`.
  - `label` holds the exact Dutch line shown under the switch:
    - `"Live via {model naam}."` (Stream, local or Apple);
    - `"Live via {provider label}."` (Stream, cloud);
    - `"Terwijl je spreekt, telkens opnieuw herkend."` (Window);
    - `"Per zin via {provider label}."` (Phrases);
    - `"Niet beschikbaar: Apple-dicteren vraagt macOS 26 of nieuwer."` (Apple without a live transcriber);
    - `"Uitgeschakeld."` (setting off).
  - `PennelEngine::live_capability(&self) -> LiveCapability` (exported) for the dictation slot.
  - `PennelEngine::meeting_preview_available(&self) -> bool` (exported): true for `Local`, and for `AppleSpeech` when a system transcriber is set.
  - Crate-internal `fn resolve(settings, models: &ModelStore, has_system_live: bool) -> LiveMethod`. Rules, in order:
    1. `live_preview == false` → `Unavailable`.
    2. `Local` → `Stream` if the catalog entry has `streaming` (role asr), else `Window`.
    3. `AppleSpeech` → `Stream` if `has_system_live`, else `Unavailable`.
    4. `OpenAi`/`Mistral` → `Stream`.
    5. everything else that `supports(kind, DictationStt)` → `Phrases`.
    6. otherwise → `Unavailable`.
  - `LocalModel.streaming` is shown by the UIs as a "Live" label in the model list.
  - Legacy entries (`legacy: true`) inherit `streaming` from `scores`.

- [ ] **Step 1: Write failing tests:**
  - `catalog_reads_streaming_flag` (`nemotron-3.5-asr-streaming-0.6b` true, `parakeet-tdt-0.6b-v3` false).
  - `resolve_local_streaming_vs_window`.
  - `resolve_cloud_kinds` (OpenAI/Mistral Stream; Groq/OpenRouter/Custom/Ollama/LmStudio Phrases).
  - `resolve_apple_needs_live_transcriber`.
  - `resolve_respects_setting_off`.
  - `old_settings_json_defaults_live_preview_on`.
  - `labels_are_exact_dutch`.
- [ ] **Step 2:** Run `cd core && cargo test live::capability catalog`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run. Expected: PASS. Then run `make core && make test && make app`: the Swift `Settings` initialiser gains `livePreview`, so update the call sites the compiler flags.
- [ ] **Step 5:** Commit `feat(core): live transcription capability per model`.

### Task 2: Phrase segmenter

**Repo:** musli

**Files:**
- Create: `core/src/live/phrases.rs`
- Modify: `core/src/chunker.rs` (make `SILENCE_RMS` and the frame RMS helper `pub(crate)` so both use one definition)

**Interfaces:**
- Produces:
  - `pub(crate) struct PhraseSegmenter { … }`, built with `new(sample_rate, PhraseRules)` where `PhraseRules { min_ms: u32, pause_ms: u32, max_ms: u32 }`.
  - Defaults: `PhraseRules::DEFAULT = { 3000, 500, 15000 }` and `PhraseRules::GROQ = { 4000, 500, 15000 }`.
  - `push(&mut self, samples: &[f32]) -> Vec<Phrase>`, where `Phrase { index: u32, start_sample: u64, end_sample: u64 }`.
  - `open(&self) -> Option<(u64, u64)>`: the range of the phrase still open.
  - `finish(&mut self) -> Option<Phrase>`.
  - Ranges are contiguous and cover every pushed sample exactly once. Silent phrases are still emitted, with `silent: bool`, so the caller can skip them without losing time.

- [ ] **Step 1: Write failing tests:**
  - `closes_on_pause_after_min`.
  - `does_not_close_before_min`.
  - `hard_cut_at_max`.
  - `ranges_are_contiguous_and_complete` (Review Focus 6).
  - `small_pushes_equal_one_big_push`.
  - `silent_phrase_flagged`.
- [ ] **Step 2:** Run `cargo test live::phrases`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, and run `cargo test chunker` too. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): phrase segmenter for live transcription`.

### Task 3: `LiveDictation` session, Window strategy, fallback

**Repo:** musli

**Files:**
- Create: `core/src/live/session.rs`, `core/src/live/window.rs`
- Modify: `core/src/engine.rs`:
  - split `process()` so that "raw → hallucination check → cleanup → `Transcript`" is reusable;
  - add `start_live_dictation`;
  - `LocalEngine` gains `pub fn decode(&self, path, samples, language, vocabulary) -> Result<(String, Duration), PennelError>`, a timed wrapper around the existing `transcribe`.

  Also `core/src/lib.rs`.

**Interfaces:**
- Consumes: Tasks 1 and 2.
- Produces:
  - `#[uniffi::export(with_foreign)] pub trait LiveListener: Send + Sync { fn on_partial(&self, partial: LivePartial); }`.
  - `#[derive(uniffi::Record)] pub struct LivePartial { seq: u32, committed: String, tentative: String }`. `seq` grows by one per callback and starts at 1.
  - `PennelEngine::start_live_dictation(self: Arc<Self>, app_bundle_id: Option<String>, keys: ApiKeys, command: bool, listener: Arc<dyn LiveListener>) -> Result<Arc<LiveDictation>, PennelError>`.
    - It validates exactly like `dictate`: `NotFound`, `MissingApiKey`.
    - It never fails because of the live path. `LiveMethod::Unavailable` gives a session that only buffers.
  - `#[derive(uniffi::Object)] pub struct LiveDictation` with:
    - `push_audio(samples: Vec<f32>)` (16 kHz mono, non-blocking: hands off to the worker through a channel);
    - `finish() -> Result<DictationResult, PennelError>` (blocking; performs history and cleanup like `dictate`; with `command == true`, `finish_command(selection: Option<String>)` is used instead and mirrors `PennelEngine::command`);
    - `cancel()`.
  - No `on_partial` call happens after `finish`/`cancel` returns.
  - Crate-internal `trait LiveStrategy: Send { fn feed(&mut self, samples: &[f32], out: &mut dyn FnMut(String, String)) -> Result<(), PennelError>; fn finish(&mut self) -> Result<String, PennelError>; }`, implemented by the Window strategy here and by Tasks 4–7.
  - A strategy error marks the take `fallback` (logged as a category only). The worker stops calling the strategy, and `finish()` then runs `engine.stt(settings, &settings.dictation_stt, &encode_wav(all_samples), keys)`.
  - Window strategy (local non-streaming):
    - Runs `PhraseSegmenter` with `DEFAULT`.
    - On every closed phrase it decodes that phrase once → committed text.
    - Between closes, and when the budget of the Global Constraints allows, it decodes the open phrase → tentative text.
    - `finish()` = joined committed phrases + a decode of the last open phrase.
    - Joining: single space, trimmed; silent phrases are skipped.

- [ ] **Step 1: Write failing tests**, with a test-only `FakeDecoder` (echoes `"p{index}"` per phrase and sleeps a configurable time) injected through `#[cfg(test)]` construction:
  - `window_emits_committed_and_tentative`.
  - `window_final_is_joined_phrases` (Review Focus 6).
  - `window_backs_off_when_decode_slow` (decode count bounded).
  - `strategy_error_falls_back_to_whole_take` (mockito STT behind a `Custom` provider, with an asserted single request carrying the full WAV; Review Focus 2).
  - `no_partial_after_finish_or_cancel` (Review Focus 4).
  - `cancel_returns_within_200ms` (Review Focus 5).
  - `finish_writes_history_like_dictate`.
  - `hallucination_final_is_no_speech`.
  - `partial_text_never_logged` (Review Focus 1: capture the crate's log sink with the sentinel `"zebrapaard-7741"`).
  - `unavailable_method_only_buffers_and_finishes_normally`.
- [ ] **Step 2:** Run `cargo test live::`. Expected: FAIL.
- [ ] **Step 3:** Implement. The worker owns the strategy and all samples. `finish()` sends a `Finish` message and joins.
- [ ] **Step 4:** Run the full core checks. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): live dictation session with windowed preview and safe fallback`.

### Task 4: Local streaming models via transcribe-cpp `Stream`

**Repo:** musli

**Files:**
- Create: `core/src/live/local_stream.rs`
- Modify: `core/src/local_engine.rs`:
  - add `pub fn with_stream<R>(&self, path, language, vocabulary, f: impl FnOnce(&mut transcribe_cpp::Stream<'_>) -> R) -> Result<R, PennelError>`. It holds the `loaded` mutex for the closure, loads the model if needed, and checks `capabilities().supports_streaming`. When that is false it returns `PennelError::InvalidResponse { message: "lokaal model: geen streaming" }` and the session falls back to Window.
  - record `supports_streaming` in `Loaded`.

**Interfaces:**
- Consumes: Tasks 1 and 3.
- Produces:
  - A `LiveStrategy` impl. The worker thread calls `with_stream` once per take and drives `feed`/`text()` from the channel inside the closure.
  - `CommitPolicy::Auto`. A partial is sent only when `committed_changed || tentative_changed`. `finish()` = `stream.finalize()`, then `text().committed` trimmed.
  - Language goes through the existing `run_options` (`UNSUPPORTED_LANGUAGE` is avoided the same way).
  - Voxtral Mini 4B Realtime: if `LocalEngine` recorded that the previous take on this model ran slower than real time (`input_received_ms` vs wall clock), `resolve` returns `Window` for it. Store the flag in memory only.

- [ ] **Step 1: Write failing tests:**
  - `non_streaming_model_falls_back_to_window` (a fake `Loaded` flag; no model file needed).
  - `meeting_jobs_wait_and_complete_after_stream` (Review Focus 3): a `MeetingSession` on `Local` with a test hook that replaces the decode, plus a live take holding the lock. Assert that `pending()` drains to 0 after `finish`.
  - `#[ignore] real_nemotron_streams_partials` (downloads `nemotron-3.5-asr-streaming-0.6b` like `real_parakeet_transcribes`; feeds `tests/fixtures/hello.wav` in 100 ms pieces; asserts ≥ 2 partials and that the final contains "test").
- [ ] **Step 2:** Run `cargo test live::local_stream`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the core checks, then `cargo test -- --ignored real_nemotron` locally on a Mac and record the timings in the PR description.
- [ ] **Step 5:** Commit `feat(core): stream local models that support it`.

### Task 5: Phrases strategy for batch cloud providers

**Repo:** musli

**Files:**
- Create: `core/src/live/phrases_cloud.rs`
- Modify: `core/src/stt.rs` (expose a `transcribe_with_prompt` path if needed; behaviour unchanged for existing callers)

**Interfaces:**
- Consumes: Tasks 2 and 3.
- Produces:
  - A `LiveStrategy` that sends each closed non-silent phrase once via `stt::transcribe`, on the worker. One request in flight at most. Closed phrases queue.
  - `prompt` = the dictionary prompt (as today) + `" "` + the last 200 characters of committed text, within `MAX_PROMPT_CHARS`.
  - `PhraseRules::GROQ` for `ProviderKind::Groq`, `DEFAULT` otherwise.
  - The open phrase is never sent (no tentative text for Phrases).
  - HTTP 429 → stop sending for this take. `finish()` then sends the remaining audio from the first unsent phrase as one request, and the result is the committed text + that text.
  - Any other error → whole-take fallback (Task 3).
  - `finish()` sends the last open phrase and joins the results.

- [ ] **Step 1: Write failing tests** (mockito):
  - `each_phrase_sent_once_and_joined`: count requests; the total WAV bytes equal the take's bytes plus headers.
  - `prompt_carries_previous_text`.
  - `groq_uses_four_second_minimum`.
  - `rate_limit_sends_rest_as_one_request`.
  - `server_error_falls_back_to_whole_take`.
  - `partial_text_never_logged` (sentinel in the mock response).
- [ ] **Step 2:** Run `cargo test live::phrases_cloud`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the core checks. Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): phrase-wise live transcription for batch providers`.

### Task 6: Realtime WebSocket client (OpenAI, Mistral)

**Repo:** musli

**Files:**
- Create: `core/src/realtime/mod.rs`, `core/src/realtime/openai.rs`, `core/src/realtime/mistral.rs`, `core/src/live/realtime.rs`
- Modify: `core/Cargo.toml` (`tungstenite = { version = "…", default-features = false, features = ["handshake", "rustls-tls-webpki-roots"] }`, plus `base64`), `core/src/provider.rs` (`pub fn realtime_model(kind) -> Option<&'static str>`: OpenAI `gpt-live-transcribe`, Mistral `voxtral-mini-transcribe-realtime-2602`), `core/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 1 and 3.
- Produces:
  - `trait RealtimeProtocol { fn url(&self, base_url: &str, model: &str) -> String; fn open_messages(&self, language: Option<&str>, terms: &[String]) -> Vec<String>; fn audio_message(&self, pcm16: &[u8]) -> String; fn commit_message(&self) -> Option<String>; fn end_message(&self) -> String; fn parse(&self, text: &str) -> RealtimeEvent; }`.
  - `enum RealtimeEvent { Delta { item: Option<String>, text: String }, Completed { item: Option<String>, text: String }, Done { text: Option<String> }, Error { code: String }, Ignore }`. The `Error` variant keeps only the code. Server messages are never stored.
  - OpenAI:
    - URL `wss://api.openai.com/v1/realtime?intent=transcription` (from base_url: `https://` → `wss://`, path `/realtime`).
    - `session.update` with `type: "transcription"`, `audio.input.format { type: "audio/pcm", rate: 24000 }`, `audio.input.transcription { model, language?, prompt? }`, `turn_detection: null`.
    - Audio is upsampled 16→24 kHz (linear) and sent as base64 `input_audio_buffer.append`.
    - `input_audio_buffer.commit` at each phrase close from `PhraseSegmenter` (`DEFAULT`) and at finish.
    - Deltas are keyed by `item_id`. Committed text = completed items in commit order. Tentative = the deltas of the newest item.
  - Mistral:
    - URL `wss://api.mistral.ai/v1/audio/transcriptions/realtime?model=…`.
    - `session.update { audio_format: { encoding: "pcm_s16le", sample_rate: 16000 }, target_streaming_delay_ms: 480, language?, context_bias? }`.
    - `input_audio.append { audio }` in 100 ms pieces, then `input_audio.end`.
    - `transcription.text.delta` appends to tentative. `transcription.done.text` is final.
  - The `Live` strategy runs one socket on its own thread. Reads use a 100 ms timeout, so `cancel` is prompt.
  - `finish()` sends end/commit and waits ≤ 10 s for `Done`/all `Completed`. On timeout or error, it does the whole-take fallback with the user's **batch** model (that is the only allowed double send; Global Constraints).
  - `resolve` only returns `Stream` for OpenAI/Mistral when `realtime_model` is `Some`.
  - The Modellen line (Task 1 label) becomes `"Live via OpenAI (gpt-live-transcribe)."` / `"Live via Mistral (voxtral-mini-transcribe-realtime-2602)."`.

- [ ] **Step 1: Write failing tests** against a local `tungstenite` server on `127.0.0.1:0`, with the base_url overridden:
  - `openai_session_update_then_append_commit`.
  - `openai_completed_items_ordered_by_commit`.
  - `openai_resamples_to_24k` (sample count ×1.5).
  - `mistral_append_end_done`.
  - `auth_in_header_not_url`.
  - `socket_drop_midway_falls_back_to_batch_once` (mockito for batch; Review Focus 2).
  - `cancel_closes_socket_within_200ms` (Review Focus 5).
  - `error_event_never_reaches_logs_or_errors` (sentinel; Review Focus 1).
- [ ] **Step 2:** Run `cargo test realtime:: live::realtime`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the core checks. Then do a manual check with real keys (`PENNEL_LIVE_SMOKE=1 cargo test -- --ignored realtime_smoke`) and record the first-partial ms.
- [ ] **Step 5:** Commit `feat(core): realtime transcription over WebSocket for OpenAI and Mistral`.

### Task 7: Apple Speech live (`SystemLiveTranscriber`)

**Repo:** musli

**Files:**
- Create: `core/src/live/system.rs`, `apps/macos-platform/Sources/PennelMacPlatform/AppleLiveSpeech.swift`
- Modify:
  - `core/src/system_stt.rs`: add `#[uniffi::export(with_foreign)] pub trait SystemLiveTranscriber: Send + Sync { fn begin(&self, language: Option<String>, listener: Arc<dyn LiveListener>) -> Result<u64, PennelError>; fn feed(&self, handle: u64, samples: Vec<f32>); fn finish(&self, handle: u64) -> Result<String, PennelError>; fn cancel(&self, handle: u64); }`.
  - `core/src/engine.rs`: `set_system_live_transcriber`.
  - `apps/macos-platform/Sources/PennelMacPlatform/Bridge.swift`: `@_cdecl("pennel_mac_live_begin" | "_feed" | "_finish" | "_cancel")`, with a C callback for partials.
  - `apps/desktop/src-tauri/src/macos.rs`: externs and safe wrappers.
  - `apps/desktop/src-tauri/src/main.rs`: `impl SystemLiveTranscriber for AppleTranscriber` on macOS, registered at startup only when `permissions()["apple_speech"]` is true.

**Interfaces:**
- Consumes: Tasks 1 and 3.
- Produces:
  - Swift: `DictationTranscriber(locale:, preset: .progressiveLongDictation)`, or `reportingOptions: [.volatileResults]` (verify on macOS 26; spec §8).
  - `SpeechAnalyzer` started with an `AsyncStream<AnalyzerInput>`. Buffers are converted with `SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith:)`.
  - Results with `isFinal` append to committed text. Volatile results replace tentative text.
  - `finish` finishes the input, calls `finalizeAndFinishThroughEndOfInput()`, and returns committed text.
  - The live session never writes a temp file.
  - The native Mac reference app does **not** implement this trait.

- [ ] **Step 1: Write failing tests:**
  - Core: `system_live_strategy_relays_partials_and_final` (a fake Rust `SystemLiveTranscriber`).
  - Core: `system_live_error_falls_back_to_system_transcribe`.
  - Desktop (`macos.rs`): `live_bridge_symbols_link` (calls `pennel_mac_live_cancel(0)` safely).
- [ ] **Step 2:** Run `cargo test live::system` and `cd apps/desktop/src-tauri && cargo test macos`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run all checks. Then do a manual check on macOS 26: dictate Dutch, see partials, and record the timings.
- [ ] **Step 5:** Commit `feat: live Apple dictation results in the shared app`.

### Task 8: Desktop dictation — feed live session, pill shows text

**Repo:** musli

**Files:**
- Modify:
  - `apps/desktop/src-tauri/src/audio.rs`: `Recorder::start` takes an extra `on_chunk: impl Fn(&[f32]) + Send + 'static`; the samples are still kept.
  - `apps/desktop/src-tauri/src/dictation.rs`:
    - `begin` calls `engine.start_live_dictation` on a helper thread; audio pieces go to `LiveDictation::push_audio`.
    - `submit` calls `finish()`/`finish_command(selection)` instead of `dictate`/`command`.
    - `cancel` calls `cancel()`.
  - `apps/desktop/src-tauri/src/pill.rs`:
    - `Pill::partial(&self, take: u64, partial: &LivePartial)` → `emit_to(LABEL, "pill-text", {take, seq, committed, tentative})`; the payload is never logged.
    - Window height grows to fit two lines (constants from the brand guide).
    - Windows: `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`; macOS: `sharingType = .none` via `crate::macos::configure_panel`.
  - `apps/desktop/src/pill.ts`, `apps/desktop/src/pill.css`: the text balloon from spec §5.1, the stale `take`/`seq` guard, and the labels `"Opkuisen…"` (Clean after release) and `"Opdracht…"` as prefix in command mode.
  - `apps/desktop/src-tauri/src/status.rs` / `core/src/status.rs`: add `PillState::Cleaning` with the label `"Opkuisen…"`.

  The Mac `PillView.swift` gets the same label for parity of states. Its text balloon stays out of scope (Task 10).

**Interfaces:**
- Consumes: Tasks 3–7.
- Produces: a host `LiveListener` impl that forwards to `Pill::partial` with the current take id. Any partial whose take id is not the active recording or the one being finished is dropped.

- [ ] **Step 1: Write failing tests:**
  - Rust: `stale_partials_are_dropped` (take id guard).
  - Rust: `one_paste_per_take_with_live_session` (extend `ordered_paste_results_keep_their_own_target_and_release_time`).
  - Rust: `pill_text_event_only_to_pill_window`.
  - TS, if a test runner exists in `apps/desktop`; otherwise a pure function `tailText(committed, tentative, max)` with a Rust-side mirror test: `tail_cuts_at_word_boundary_with_ellipsis`.
- [ ] **Step 2:** Run `cd apps/desktop/src-tauri && cargo test dictation pill`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm run build`, `cargo test`, then the app. Check by screenshot of the pill window (headless/own window, never the user's screen) on macOS and Linux/Windows CI artefacts.
- [ ] **Step 5:** Commit `feat(desktop): show live text in the pill while dictating`.

### Task 9: Meeting preview line

**Repo:** musli

**Files:**
- Modify:
  - `core/src/session.rs`: the worker computes previews when its channel is empty.
  - `core/src/chunker.rs`: `pub(crate) fn open_samples(&self) -> &[f32]` and `open_start_ms()`.
  - `apps/desktop/src-tauri/src/commands.rs`: `MeetingStatus.preview: Vec<LivePreview>`.
  - `apps/desktop/src/views/meetings.ts`: grey italic line per speaker under the live transcript, or the fixed line `"Tekst verschijnt per fragment van 15 tot 30 seconden."` when `meeting_preview_available()` is false.
  - `apps/desktop/src/styles.css`: existing muted text token only.

**Interfaces:**
- Consumes: Task 1 (`meeting_preview_available`), Task 3 (`LocalEngine::decode`).
- Produces:
  - `#[derive(uniffi::Record)] pub struct LivePreview { speaker: Speaker, text: String }`.
  - `MeetingSession::preview(&self) -> Vec<LivePreview>` (exported).
  - Previews are computed only for `Local`/`AppleSpeech`, at most every 3 s per speaker, only on ≥ 1 s of open speech, never while `dictating` is true, never while jobs are pending, and only when `live_preview` is on.
  - A preview for a speaker is cleared when that speaker's chunk is enqueued.
  - Previews are never written to disk.

- [ ] **Step 1: Write failing tests:**
  - `preview_appears_for_open_chunk_and_clears_on_cut` (test decode hook).
  - `no_preview_for_cloud_provider`.
  - `no_preview_while_dictating_or_pending`.
  - `preview_never_in_transcript_file`.
- [ ] **Step 2:** Run `cargo test session::`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the core and desktop checks.
- [ ] **Step 5:** Commit `feat: live preview line in the meeting window`.

### Task 10: Settings UI, model labels, Mac reference app parity, docs

**Repo:** musli

**Files:**
- Modify:
  - `apps/desktop/src/views/dictation.ts`: switch `"Toon tekst terwijl je spreekt"`, `live_capability().label` under it, and the hint `"De tekst is zichtbaar voor wie je scherm ziet."`.
  - `apps/desktop/src/views/` model list (where `LocalModel` is rendered): a `"Live"` label when `streaming`.
  - `apps/desktop/src-tauri/src/commands.rs`: `live_capability` command.
  - `apps/macos/Sources/Pennel/Windows/DictationPane.swift`: the same switch, **disabled**, with `"Beschikbaar in de nieuwe Pennel-app"`.
  - `apps/macos/Sources/Pennel/Windows/LocalModelsView.swift`: the `"Live"` label.
  - `docs/desktop.md`: parity table row "Live tekst tijdens dicteren", with the gap for the native app and Apple only on macOS 26+.
  - `README.md`: only once Tasks 1–9 ship, one line under dictation.

**Interfaces:**
- Consumes: Task 1.

- [ ] **Step 1: Write failing tests:**
  - Desktop Rust: `live_capability_command_returns_core_label`.
  - Swift `PennelKit` tests, if the pane logic is extracted: `live_toggle_disabled_in_reference_app`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement. Review parity with side-by-side screenshots of both apps' Dicteren panes.
- [ ] **Step 4:** Run all checks from the Global Constraints.
- [ ] **Step 5:** Commit `feat: live transcription setting in both apps`.

### Task 11: Cloud realtime compatibility

The historical server implementation task is retained in private maintainer records. In this plan, the public client tasks below depend on a compatible Cloud realtime interface; this archive does not establish deployment or acceptance.

### Task 12: Pennel Cloud as a live provider in the core

**Repo:** musli (after Task 11 is deployed to staging, and after plan `2026-10-25-pennel-cloud-v1-2-core.md` Task 3)

**Files:**
- Modify: `core/src/realtime/mistral.rs` (reused, with `PennelCloud` URL and auth), `core/src/live/capability.rs` (`PennelCloud` → `Stream` when the account reports `realtime`, else `Phrases`), `core/src/engine.rs` (use `credential()` from cloud plan Task 3 for the bearer token), `core/src/cloud/http.rs` (map the `error` event codes `quota`/`overloaded` to `PennelError::Cloud` with the server message, shown by hosts as today)

**Interfaces:**
- Consumes: Tasks 6 and 11; cloud plan Tasks 2–3.
- Produces:
  - `PennelCloud` dictation uses `{cloud::base_url(preview)}/v1/audio/transcriptions/realtime` (scheme `wss`) with `Authorization: Bearer <access token>`, and the label `"Live via Pennel Cloud."`.
  - An access token that expires during a take is never refreshed mid-socket. The socket stays authorised from the handshake.

- [ ] **Step 1: Write failing tests:**
  - `pennel_cloud_uses_realtime_when_available`.
  - `pennel_cloud_without_realtime_uses_phrases`.
  - `pennel_cloud_quota_error_shows_server_message`.
  - `not_signed_in_is_not_signed_in` (no socket opened).
- [ ] **Step 2:** Run `cargo test pennel_cloud live`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the full core checks, plus a manual staging check (`PENNEL_CLOUD_URL=https://api-staging.pennel.app`).
- [ ] **Step 5:** Commit `feat(core): live dictation through Pennel Cloud`.
