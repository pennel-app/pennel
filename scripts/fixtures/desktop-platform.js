/* Loaded only by capture-desktop-reference.mjs before the development entry point. */
(() => {
  const options = globalThis.__PENNEL_FIXTURE_OPTIONS__;
  if (!options || !location.hostname.match(/^(127\.0\.0\.1|localhost)$/)) throw new Error("Visual fixtures require an owned local server");
  const clone = value => structuredClone(value);
  const systemLanguages = options.system_languages || [options.locale || "nl"];
  const systemInterfaceLanguage = systemLanguages.map(language => String(language).trim().split(/[-_.@]/)[0].toLowerCase()).find(language => language === "en" || language === "nl") || "en";
  const profileKey = options.profile ? `pennel-synthetic-fixture:${options.profile}` : null;
  let persistedSettings;
  if (profileKey) {
    try { persistedSettings = JSON.parse(globalThis.localStorage?.getItem(profileKey) || "null"); } catch {}
  }
  const now = Date.parse("2026-10-25T09:00:00Z");
  const ActualDate = Date;
  function FixtureDate(...args) {
    if (!new.target) return new ActualDate(now).toString();
    return Reflect.construct(ActualDate, args.length ? args : [now], new.target);
  }
  Object.setPrototypeOf(FixtureDate, ActualDate);
  FixtureDate.prototype = ActualDate.prototype;
  FixtureDate.now = () => now;
  globalThis.Date = FixtureDate;

  const meetingId = "2026-10-24 0900 Projectoverleg";
  const sentence = "Zullen we morgen om tien uur afspreken?";
  const pillSnapshot = options.window === "pill" ? {
    state: options.pill_state,
    preview: { session_id: "qa-" + options.name, revision: options.preview_text ? 7 : 0, text: options.preview_text || "" },
  } : { state: "hidden" };
  let notes = "- Planning afronden\n- Volgende afspraak bevestigen";
  const summary = "## Besluiten\n- De planning wordt maandag afgerond.\n\n## Actiepunten\n- [ ] Volgende afspraak bevestigen.";
  const meeting = { id: meetingId, title: "Projectoverleg", start_iso: "2026-10-24T09:00:00+02:00", duration_min: 25, has_summary: true };
  const library = [
    { id: "2026-10-25 0830 Weekstart", title: "Weekstart", start_iso: "2026-10-25T08:30:00+01:00", duration_min: 18, has_summary: true },
    meeting,
    { id: "2026-10-14 1400 Klantgesprek Studio Noord", title: "Klantgesprek Studio Noord", start_iso: "2026-10-14T14:00:00+02:00", duration_min: 42, has_summary: true },
  ];
  const meetingInfo = id => library.find(item => item.id === id) || meeting;
  const transcript = [
    { start_ms: 0, end_ms: 8000, speaker: "others", text: "We ronden de planning maandag af.", failed: false, voice: 1 },
    { start_ms: 8000, end_ms: 14000, speaker: "me", text: "Ik bevestig de volgende afspraak.", failed: false, voice: null },
    { start_ms: 14000, end_ms: 21000, speaker: "others", text: "Dan zet ik het voorstel nog vandaag in de map.", failed: false, voice: 2 },
  ];
  const available = () => ({ status: "available", reason: null });
  const providers = [
    { id: "local", kind: "local", name: "Lokaal", base_url: "" },
    { id: "groq", kind: "groq", name: "Groq", base_url: "https://api.groq.com/openai/v1" },
    { id: "apple-speech", kind: "apple_speech", name: "Apple-dicteren", base_url: "" },
    { id: "openai", kind: "open_ai", name: "OpenAI", base_url: "https://api.openai.com/v1" },
  ];
  const providerDefinitions = [
    ["local", "Lokaal op deze computer", false, "", true, false, "whisper-large-v3-turbo", ""],
    ["groq", "Groq", true, "https://api.groq.com/openai/v1", true, true, "whisper-large-v3-turbo", "llama-3.3-70b-versatile"],
    ["open_ai", "OpenAI", true, "https://api.openai.com/v1", true, true, "gpt-4o-mini-transcribe", "gpt-4.1-mini"],
    ["open_router", "OpenRouter", true, "https://openrouter.ai/api/v1", true, true, "openai/whisper-large-v3-turbo", "google/gemini-3.5-flash-lite"],
    ["mistral", "Mistral", true, "https://api.mistral.ai/v1", true, true, "voxtral-mini-latest", "mistral-small-latest"],
    ["apple_speech", "Apple-dicteren", false, "", true, false, "dictation", ""],
    ["apple_intelligence", "Apple Intelligence", false, "", false, true, "", "system"],
    ["ollama", "Ollama (lokaal)", false, "http://localhost:11434", false, true, "", "gemma3:4b"],
    ["lm_studio", "LM Studio (lokaal)", false, "http://localhost:1234/v1", false, true, "", ""],
    ["custom", "Eigen server", true, "", true, true, "", ""],
    ["pennel_cloud", "Pennel Cloud", false, "https://api.pennel.app/v1", true, true, "pennel-auto", "pennel-auto"],
  ];
  const kinds = providerDefinitions.map(([kind, label, requires_key, default_base_url, stt, llm, default_stt_model, default_llm_model]) => ({
    kind, label, requires_key, default_base_url, stt, llm, default_stt_model, default_llm_model,
    is_cli_agent: false, availability: available(),
  }));
  const selectedProvider = options.engine === "apple" ? "apple-speech" : options.engine === "custom" ? "openai" : "local";
  const model = selectedProvider === "apple-speech" ? "dictation" : selectedProvider === "openai" ? "gpt-4o-mini-transcribe" : "whisper-large-v3-turbo";
  let settings = {
    providers, dictation_stt: { provider_id: selectedProvider, model }, meeting_stt: { provider_id: "local", model: "whisper-large-v3-turbo" },
    llm: { provider_id: "groq", model: "llama-3.3-70b-versatile" }, decision: null, reranking: null, cleanup_verification_enabled: Boolean(options.cleanup_verification_enabled), mode: options.mode || "fast", language: "nl", ui_language: options.ui_language || "system", dictionary: ["Pennel", "Studio Noord"],
    style_rules: [{ bundle_id: "com.tinyspeck.slackmacgap", style: "casual" }, { bundle_id: "com.apple.MobileSMS", style: "casual" }, { bundle_id: "com.apple.mail", style: "formal" }, { bundle_id: "com.microsoft.Word", style: "formal" }, { bundle_id: "com.apple.Terminal", style: "literal" }, { bundle_id: "com.microsoft.VSCode", style: "literal" }], history_retention_days: 30, hotkey: "fn", local_unload_minutes: 5,
    meetings_dir: "/QA/Pennel Preview/Meetings", default_template: "algemeen", meeting_audio_retention_days: 30,
    app_rules: [], calendar_rules: [], event_rules: [], calendar_enabled: false, echo_cancellation: true,
    record_shortcut_enabled: true, consent_enabled: false, auto_detect: true, diarization: Boolean(options.speakers), speaker_recognition: options.speaker_mode || "local",
    appearance: options.appearance, dictation_pad_auto_paste: false, cloud_sync: Boolean(options.cloud), claude_access: Boolean(options.claude),
    onboarding: { version: 1, status: options.onboarding_status || (options.window === "onboarding" ? "pending" : options.shortcut_denied ? "skipped" : "completed"), resume_step: options.step || "welcome", goal: options.goal || "dictation" },
  };
  if (options.cleanup_connection) {
    settings.providers.push({ id: "qa-verifier", kind: "custom", name: "Synthetic verifier", base_url: "https://verifier.example.invalid/v1" });
    settings.decision = { provider_id: "qa-verifier", protocol: "perplexity", model: "synthetic-unqualified", expected_model: null, upstream: null, max_input_bytes: 16000, timeout_secs: 3 };
  }
  if (persistedSettings && typeof persistedSettings === "object") settings = { ...settings, ...persistedSettings };
  const interfaceLanguage = () => settings.ui_language === "nl" || settings.ui_language === "en" ? settings.ui_language : systemInterfaceLanguage;
  const capabilities = Object.fromEntries(["pad_external_paste", "global_hotkey", "record_shortcut", "microphone", "system_audio", "echo_cancellation", "call_detection", "calendar", "apple_speech", "apple_intelligence", "selection", "rich_clipboard", "audio_playback", "diarization", "local_models"].map(name => [name, available()]));
  capabilities.local_acceleration = "metal";
  const permissionStatus = options.permission || "granted";
  let permissions = ["microphone", "accessibility", "calendar", "speech", "system_audio"].map(kind => ({ kind, status: kind === "calendar" ? "not_determined" : permissionStatus }));
  if (options.shortcut_denied) {
    permissions = permissions.map(permission => permission.kind === "accessibility" ? { ...permission, status: "denied" } : permission);
  }
  if (permissions.find(permission => permission.kind === "accessibility")?.status !== "granted") {
    const reason = "Geef Pennel toegang tot Toegankelijkheid om fn in andere apps te gebruiken.";
    capabilities.global_hotkey = { status: "permission_required", reason };
    capabilities.selection = { status: "permission_required", reason };
    capabilities.pad_external_paste = { status: "permission_required", reason };
  }
  const keys = { groq: Boolean(options.configured_text_provider), openai: false };
  const providerRevisions = Object.fromEntries(providers.map(provider => [provider.id, 0]));
  const models = (options.local_models || [{
    id: "whisper-large-v3-turbo", name: "Whisper Large v3 Turbo", description: "Spraakherkenning in meerdere talen.",
    size_bytes: 886381760, parameters: "809M", speed: 72, accuracy: 86, languages: ["nl", "en", "fr", "de"], recommended: true, legacy: false,
  }]).map(model => ({ ...model, state: model.id === "whisper-large-v3-turbo" && options.download_active
    ? { kind: "partial", bytes: Math.floor(model.size_bytes / 3) }
    : model.id === "whisper-large-v3-turbo" && !(options.window === "onboarding" && ["language", "welcome", "engine"].includes(options.step)) ? { kind: "downloaded" } : { kind: "not_downloaded" } }));
  const diarizer = { id: "diar_streaming_sortformer_4spk-v2.1", name: "Sortformer (sprekers)", size_bytes: 139310336, state: { kind: "not_downloaded" } };
  const speakerModels = [["pyannote-segmentation-3.0", 5992913], ["3dspeaker-campplus-zh-en-advanced", 28281164]].map(([id, size_bytes]) => ({ id, size_bytes, state: options.speakers === "ready" ? { kind: "downloaded" } : { kind: "not_downloaded" } }));
  if (options.speakers === "legacy") diarizer.state = { kind: "downloaded" };
  let voiceProfile = options.voice_profile ? { created_iso: "2026-10-20T10:00:00+02:00", speech_seconds: 22 } : null;
  let knownVoices = options.known_voices ? [{ id: "qa-noor", name: "Noor", meetings: 4, updated_iso: "2026-10-24T09:30:00+02:00" }, { id: "qa-sam", name: "Sam Peeters", meetings: 1, updated_iso: "2026-10-14T15:00:00+02:00" }] : [];
  const meetingVoices = [{ voice: 1, name: "Noor", remembered: Boolean(options.known_voices), can_remember: true }, { voice: 2, name: null, remembered: false, can_remember: true }];
  const modelFor = id => models.find(model => model.id === id) || speakerModels.find(model => model.id === id) || (id === diarizer.id ? diarizer : undefined);
  const activeDownloads = new Set(options.download_active ? ["whisper-large-v3-turbo"] : []);
  const downloadWaiters = new Map();
  let pad = sentence;
  let sessionCounter = 0;
  const sessions = new Map();
  const listeners = new Map();
  const calls = [];
  const unknown = [];
  const emit = (name, payload) => {
    for (const handler of listeners.get(name) || []) handler({ payload: clone(payload) });
  };
  const frozenMeetingRecognition = () => {
    const account = settings.providers.find(provider => provider.id === settings.meeting_stt.provider_id);
    return { provider_kind: account?.kind ?? null, provider_name: account?.name ?? "Niet ingesteld", model: settings.meeting_stt.model, language: settings.language };
  };
  let summaryOutdated = !!options.summary_outdated;
  let retranscriptionStatus = options.retranscription_phase ? {
    running: !["completed", "failed"].includes(options.retranscription_phase), recognition: frozenMeetingRecognition(),
    progress: { phase: options.retranscription_phase, speaker: "others", completed: 15_000, total: 60_000 }, error: null, warning: null,
  } : null;
  const completeDownload = (id, error = null) => {
    if (!activeDownloads.delete(id)) return false;
    const model = modelFor(id);
    if (!error && model) {
      model.state = { kind: "downloaded" };
      emit("download-progress", { id, downloaded: model.size_bytes, total: model.size_bytes });
    }
    const waiting = downloadWaiters.get(id) || [];
    downloadWaiters.delete(id);
    emit("download-done", { id, error });
    for (const waiter of waiting) error ? waiter.reject(new Error(error)) : waiter.resolve(true);
    return true;
  };
  const shortcutDenied = () => permissions.find(permission => permission.kind === "accessibility")?.status !== "granted";
  const cleanupVerification = () => {
    const config = settings.decision;
    const account = settings.providers.find(provider => provider.id === config?.provider_id);
    return { enabled: settings.cleanup_verification_enabled === true, available: false,
      reason_code: settings.cleanup_verification_enabled && settings.mode === "clean" ? config ? "unqualified" : "unconfigured" : "disabled",
      provider_id: config?.provider_id ?? null, model: config?.model ?? null, endpoint: account ? `${account.base_url}/decisions` : null };
  };
  const info = () => ({ interface_language: interfaceLanguage(), system_interface_language: systemInterfaceLanguage, settings: clone(settings), keys: clone(keys), provider_revisions: clone(providerRevisions), active_model_downloads: [...activeDownloads], cleanup_verification: cleanupVerification(),
    unsaved_history_count: history.filter(entry => entry.status === "unsaved").length,
    kinds: clone(kinds), platform: "macos", version: "0.1.0", hotkey_error: shortcutDenied() ? capabilities.global_hotkey.reason : null,
    log_path: "/QA/Pennel Preview/pennel.log", data_dir: "/QA/Pennel Preview", capabilities: clone(capabilities), recording_app_ids: options.recording_app_ids || [], diarizer_model_id: diarizer.id, preview: true, migration_warning: null });
  const history = [
    ...(options.history_unsaved ? [{ id: "qa-dictation-unsaved", created_at_ms: now - 60_000, app_bundle_id: "com.apple.mail", raw: sentence, text: sentence, status: "unsaved", error: "De dicteergeschiedenis kon niet worden bewaard.", has_audio: false }] : []),
    { id: "qa-dictation-2", created_at_ms: now - 15 * 60_000, app_bundle_id: "dev.pennel.Pennel.preview", raw: sentence, text: sentence, status: "done", error: null, has_audio: false },
    { id: "qa-dictation-1", created_at_ms: now - 60 * 60_000, app_bundle_id: "com.apple.mail", raw: "ik werk het voorstel bij en stuur het morgen door", text: "Ik werk het voorstel bij en stuur het morgen door.", status: "done", error: null, has_audio: false },
    { id: "qa-dictation-0", created_at_ms: now - 26 * 60 * 60_000, app_bundle_id: "com.tinyspeck.slackmacgap", raw: "kan iemand de notities van gisteren delen", text: "Kan iemand de notities van gisteren delen?", status: "done", error: null, has_audio: false },
    { id: "qa-dictation-failed", created_at_ms: now - 27 * 60 * 60_000, app_bundle_id: "com.apple.Notes", raw: "", text: "", status: "failed", error: "Geen verbinding met de provider.", has_audio: true },
  ];
  // `cloud` is unset (signed out), "synced" or "conflict"; all accounts, devices and codes are synthetic.
  let signedIn = Boolean(options.cloud);
  let syncSetupPhase = options.sync_setup || (signedIn ? "active" : "signed_out");
  if (options.sync_setup) settings.cloud_sync = options.sync_setup === "active";
  const notSignedIn = () => ({ code: "not_signed_in", message: "Je bent niet ingelogd bij Pennel Cloud." });
  const cloudAccount = { email: "noor@example.com", audio_seconds: 5400, tokens: 120000, storage_bytes: 52_428_800, limit_audio_seconds: 36000, limit_tokens: 2_000_000, limit_storage_bytes: 5_368_709_120 };
  const cloudDevices = [
    { id: "qa-device-mac", name: "MacBook van Noor", platform: "macos", last_seen_iso: "2026-10-25T08:55:00Z", current: true },
    { id: "qa-device-windows", name: "Werklaptop", platform: "windows", last_seen_iso: "2026-10-24T16:10:00Z", current: false },
  ];
  const syncReport = () => ({ status: options.sync_error ? { kind: "error", message: "Synthetic sync connection failed." } : { kind: !signedIn ? "signed_out" : !settings.cloud_sync ? "off" : options.sync_status || "synced" }, brake: false, uploaded: settings.cloud_sync ? 3 : 0, downloaded: settings.cloud_sync ? 2 : 0, conflicts: options.cloud === "conflict" ? 1 : 0, last_success_ms: signedIn && ["active", "paused"].includes(syncSetupPhase) ? now - 4 * 60_000 : null });
  const syncSetup = () => ({ phase: syncSetupPhase, sync_enabled: settings.cloud_sync, generation: signedIn && syncSetupPhase !== "new_library" ? 1 : null, can_approve_devices: ["active", "paused", "ready"].includes(syncSetupPhase), uploaded: syncReport().uploaded, downloaded: syncReport().downloaded, conflicts: syncReport().conflicts, last_success_ms: syncReport().last_success_ms });
  const emptyLink = () => ({ phase: "idle", role: null, request_id: null, connection_code: null, qr_payload: null, qr_svg: null, verification_code: null, peer_name: null, peer_platform: null, account_email: null, expires_at_ms: null, error_code: null, error_message: null });
  const fixtureLink = (phase, role = options.link_role || (syncSetup().can_approve_devices ? "trusted_device" : "new_device")) => ({ ...emptyLink(), phase, role, request_id: "qa-connection-request", connection_code: phase === "waiting_peer" ? "QAFIXTURE-12345678" : null, verification_code: ["awaiting_approval", "waiting_approval", "provisioning"].includes(phase) ? "481 926" : null, peer_name: "Noor’s iPhone", peer_platform: "ios", account_email: "noor@example.com", expires_at_ms: now + 600000, error_code: phase === "failed" ? "connection_failed" : null, error_message: phase === "failed" ? "Synthetic connection rejected." : null });
  let deviceLink = options.link_phase ? fixtureLink(options.link_phase) : emptyLink();
  let linkPolls = 0;
  let linkStartFailures = 0;
  let recoveryCode = "QAFI-XTUR-E000-0000-0000-0000";
  const completeLink = () => { signedIn = true; settings.cloud_sync = true; syncSetupPhase = "active"; deviceLink = fixtureLink("complete", deviceLink.role); emit("settings-changed", null); emit("sync-changed", syncReport()); return clone(deviceLink); };
  const fixture = {
    ...options, now: new ActualDate(now).toISOString(), meeting_id: meetingId,
    onboarding: { step: options.step, engine: options.engine || "local", download_error: options.download_error || null, practice_error: options.practice_error || null },
  };
  const sessionSnapshot = kind => ({ session_id: `qa-${kind}-${++sessionCounter}`, kind,
    phase: kind === "practice" && options.practice_error ? "error" : kind === "practice" && options.practice_text ? "done" : "idle", text: kind === "pad" ? pad : options.practice_text || "",
    error: kind === "practice" ? options.practice_error || null : null, target_name: null,
    external_paste_available: !shortcutDenied(), external_paste_reason: shortcutDenied() ? capabilities.pad_external_paste.reason : null });
  let recordingStorageError = Boolean(options.recording_storage_error);
  let recordingRecovered = false;
  const meetingStatus = (includeContent = true) => {
    const retained = options.section === "recording" && !recordingRecovered;
    const storageError = retained && recordingStorageError;
    const paused = retained && Boolean(options.transcription_paused);
    const persistenceError = storageError ? "Je opname kon niet volledig worden bewaard. Probeer opnieuw te bewaren." : null;
    return {
      recording: retained && !storageError, phase: storageError ? "error" : retained ? "recording" : "idle", busy: retained && !storageError,
      error: persistenceError, session_id: retained ? "qa-recording" : null, id: retained ? meetingId : null,
      title: meeting.title, elapsed_secs: 754, system_audio: true, heard_system_audio: true,
      echo_cancellation_active: retained && !storageError, audio_warning: null, persistence_error: persistenceError,
      transcription_warning: paused ? "Transcriptie is tijdelijk gepauzeerd. Je audio en herstelbestanden blijven lokaal bewaard." : null,
      terminal_error: storageError ? "De opname is gestopt omdat bewaren niet lukt." : null, can_retry_persistence: storageError,
      pending: storageError ? 1 : paused ? 3 : 0, transcript_revision: retained ? 1 : 0,
      ...(includeContent ? { notes, transcript: retained ? clone(transcript) : [] } : {}),
      ...(retained ? { recognition: { provider_kind: "local", provider_name: "Lokaal", model: settings.meeting_stt.model, language: "nl" } } : {}),
    };
  };

  async function invoke(command, args = {}) {
    calls.push(command);
    switch (command) {
      case "desktop_environment": return {
        material: options.native_material || "unavailable",
        appearance: settings.appearance === "system" ? options.appearance : settings.appearance,
        reduce_transparency: Boolean(options.reduce_transparency),
        reduce_motion: Boolean(options.reduce_motion),
        increase_contrast: Boolean(options.increase_contrast),
        titlebar_height: options.platform && options.platform !== "macos" ? 0 : 28,
        titlebar_controls_width: options.platform && options.platform !== "macos" ? 0 : 96,
        focused: options.window_focused !== false,
      };
      case "set_desktop_command_state": globalThis.__PENNEL_DESKTOP_COMMAND_STATE__ = clone(args.state); return null;
      case "app_info": if (options.app_info_fail) throw new Error("Synthetic startup failure"); if (options.app_info_delay_ms) await new Promise(resolve => setTimeout(resolve, options.app_info_delay_ms)); return info();
      case "save_settings": {
        settings = clone(args.settings);
        settings.ui_language = ["en", "nl", "system"].includes(settings.ui_language) ? settings.ui_language : "system";
        if (profileKey) { try { globalThis.localStorage?.setItem(profileKey, JSON.stringify(settings)); } catch {} }
        emit("settings-changed", null);
        return info();
      }
      case "get_permissions": return clone(permissions);
      case "request_permission": {
        permissions = permissions.map(permission => permission.kind === args.kind ? { ...permission, status: "granted" } : permission);
        if (args.kind === "accessibility") for (const capability of ["global_hotkey", "selection", "pad_external_paste"]) capabilities[capability] = available();
        return clone(permissions);
      }
      case "onboarding_readiness": {
        const slot = args.target === "meeting" ? settings.meeting_stt : settings.dictation_stt;
        const account = settings.providers.find(account => account.id === slot.provider_id);
        const definition = kinds.find(kind => kind.kind === account?.kind);
        const denied = permissions.some(permission => permission.kind === "microphone" && ["denied", "restricted", "not_determined"].includes(permission.status));
        const configured = Boolean(account && slot.model && (account.kind === "local" ? models.some(model => model.id === slot.model && model.state.kind === "downloaded") : !definition?.requires_key || keys[account.id]));
        const accessibility = permissions.find(permission => permission.kind === "accessibility")?.status === "granted";
        return { configured, can_record: configured && !denied, can_use_shortcut: accessibility, can_paste_externally: accessibility, reason: denied ? "Geef Pennel toegang tot je microfoon." : configured ? null : "Kies een werkende spraakherkenning.", resume_step: configured ? "permissions" : "engine" };
      }
      case "pad_text": return pad;
      case "save_pad_text": {
        pad = String(args.text || "");
        for (const session of sessions.values()) if (session.kind === "pad") {
          session.text = pad;
          if (session.phase === "error") { session.phase = "idle"; session.error = null; }
          emit("dictation-session", session);
        }
        return null;
      }
      case "open_dictation_session": { const session = sessionSnapshot(args.kind); sessions.set(session.session_id, session); return clone(session); }
      case "get_dictation_session": return clone(sessions.get(args.sessionId || args.session_id));
      case "toggle_dictation_session": {
        const session = sessions.get(args.sessionId || args.session_id);
        if (!session) throw new Error("Onbekende QA-sessie");
        if (session.phase === "listening") {
          session.phase = "done"; session.text = session.kind === "pad" ? pad + "\n\n" + sentence : sentence;
          if (session.kind === "pad") pad = session.text;
        } else session.phase = "listening";
        emit("dictation-session", session); return null;
      }
      case "cancel_dictation_session": { const session = sessions.get(args.sessionId || args.session_id); if (session) { session.phase = "idle"; emit("dictation-session", session); } return null; }
      case "close_dictation_session": {
        const id = args.sessionId || args.session_id;
        const session = sessions.get(id);
        if ((args.onlyIfIdle || args.only_if_idle) && session && ["listening", "working"].includes(session.phase)) throw new Error("Finish or cancel dictation before leaving.");
        sessions.delete(id); return null;
      }
      case "dictation_state": return clone(pillSnapshot);
      case "frontend_ready": globalThis.__PENNEL_FIXTURE_READY__ = true; return null;
      case "frontend_flushed": return null;
      case "app_identity": return null;
      case "history": return clone(history);
      case "delete_history": {
        const index = history.findIndex(entry => entry.id === args.id);
        if (index !== -1) history.splice(index, 1);
        emit("history-changed", null); emit("settings-changed", null); return null;
      }
      case "save_history": {
        const entry = history.find(entry => entry.id === args.id);
        if (!entry || entry.status === "failed") throw new Error("Onbekend tijdelijk QA-dictaat");
        entry.status = "done"; entry.error = null;
        emit("history-changed", null); emit("settings-changed", null); return null;
      }
      case "copy_history": {
        if (!history.some(entry => entry.id === args.id)) throw new Error("Onbekend QA-dictaat");
        return null;
      }
      case "retry_history": return sentence;
      case "local_models": return clone(args.supportedOnly || args.supported_only ? models.filter(model => model.languages.some(language => language.split(/[-_]/)[0] === (args.language || "nl"))) : models);
      case "model_state":
        if (options.model_state_delay_ms) await new Promise(resolve => setTimeout(resolve, options.model_state_delay_ms));
        return clone(modelFor(args.id)?.state || { kind: "not_downloaded" });
      case "download_model": {
        if (!modelFor(args.id)) throw new Error("Onbekend QA-model");
        if (activeDownloads.has(args.id)) return null;
        activeDownloads.add(args.id);
        queueMicrotask(() => completeDownload(args.id, options.download_error || null));
        return null;
      }
      case "join_model_download": {
        if (!activeDownloads.has(args.id)) return false;
        const model = modelFor(args.id);
        return new Promise((resolve, reject) => {
          if (!downloadWaiters.has(args.id)) downloadWaiters.set(args.id, []);
          downloadWaiters.get(args.id).push({ resolve, reject });
          emit("download-progress", { id: args.id, downloaded: model?.state.bytes || 0, total: model?.size_bytes || 0 });
        });
      }
      case "cancel_download": {
        completeDownload(args.id, "cancelled");
        return null;
      }
      case "delete_model": {
        if (activeDownloads.has(args.id)) throw new Error("Dit model wordt nog gedownload.");
        const model = modelFor(args.id); if (model) model.state = { kind: "not_downloaded" }; return info();
      }
      case "add_provider": { const definition = kinds.find(kind => kind.kind === args.kind); settings.providers.push({ id: `qa-${args.kind}`, kind: args.kind, name: definition?.label || args.kind, base_url: definition?.default_base_url || "" }); return info(); }
      case "remove_provider": settings.providers = settings.providers.filter(provider => provider.id !== args.id); return info();
      case "set_key": keys[args.account] = Boolean(args.key); providerRevisions[args.account] = (providerRevisions[args.account] || 0) + 1; return info();
      case "inference_protocols": return [
        ["open_router_decisions", "OpenRouter Decisions", "https://openrouter.ai/api/v1"],
        ["open_router_system_one", "OpenRouter System One", "https://openrouter.ai/api/v1"],
        ["type_safe", "TypeSafe Jev", "https://api.typesafe.ai/v1"],
        ["perplexity", "Perplexity Decisions", "https://api.perplexity.ai/v1"],
        ["mercury", "Mercury Decide", "https://api.inceptionlabs.ai/v1"],
        ["local_decider", "Local Decider", "http://127.0.0.1:8080/v1"],
        ["tei", "TEI reranking", "http://127.0.0.1:8080"],
      ].map(([protocol, name, default_base_url]) => ({ protocol, name, default_base_url, decision: protocol !== "tei", reranking: protocol === "tei", extraction: protocol === "mercury", requires_key: !["tei", "local_decider"].includes(protocol) }));
      case "inference_availability": return { capability: args.capability, configured: false, available: false, reason: "Configure this optional connection first.", provider_id: null, model: null, protocol: null, endpoint: null };
      case "cleanup_verification_availability": return cleanupVerification();
      case "provider_models": return [{ id: "gpt-4o-mini-transcribe", name: "GPT-4o mini Transcribe", audio_input: true, is_router: false }, { id: "llama-3.3-70b-versatile", name: "Llama 3.3", audio_input: false, is_router: false }];
      case "test_provider_task": {
        if (!["dictation_stt", "meeting_stt", "llm"].includes(args.slot)) throw new Error("Onbekende QA-taak");
        const choice = settings[args.slot];
        const provider = settings.providers.find(provider => provider.id === choice.provider_id);
        const kind = kinds.find(kind => kind.kind === provider?.kind);
        const configured = Boolean(provider && kind && (args.slot === "llm" ? kind.llm : kind.stt) && choice.model && (!kind.requires_key || keys[provider.id])
          && (provider.kind !== "local" || modelFor(choice.model)?.state.kind === "downloaded"));
        return { verified: configured, message: configured ? "Test geslaagd met uitsluitend synthetische voorbeelddata." : "Stel eerst het gekozen model en de provider in." };
      }
      case "detect_cli_agents": return [];
      case "check_cli_agent": return "Niet uitgevoerd in deze fixture.";
      case "meeting_status": return meetingStatus();
      case "meeting_runtime_status": return meetingStatus(false);
      case "set_meeting_title": meeting.title = String(args.title ?? ""); return null;
      case "set_meeting_notes": case "save_meeting_notes": notes = String(args.text ?? ""); return null;
      case "retry_meeting_persistence": {
        const status = meetingStatus();
        if (!status.can_retry_persistence || args.sessionId !== status.session_id) throw new Error("Deze QA-opnamesessie is niet meer actief.");
        recordingStorageError = false; recordingRecovered = true;
        emit("meeting-finished", { id: meetingId, session_id: status.session_id, error: null, flush_warning: null });
        emit("meeting-updated", null); return null;
      }
      case "meetings": return options.empty ? [] : clone(library);
      case "meeting_detail": return { info: clone(meetingInfo(args.id)), notes, summary, transcript: clone(transcript), speakers: [{ voice: 1, name: "Noor" }], dir: `/QA/Pennel Preview/Meetings/${meetingId}`, attendees: ["Noor", "Sam"], audio_tracks: [{ speaker: "me", available: !!options.saved_audio }, { speaker: "others", available: !!options.saved_audio }],
        summary_outdated: summaryOutdated, retranscription_status: clone(retranscriptionStatus),
        retranscription_plan: { recognition: frozenMeetingRecognition(), available: !!options.saved_audio && !retranscriptionStatus?.running && frozenMeetingRecognition().provider_kind === "local", reason: options.saved_audio ? null : "Er is geen bewaarde audio om opnieuw te transcriberen." } };
      case "search_library": return { meetings: options.empty ? [] : clone(library), passages: [] };
      case "ask_meetings": return { text: "De planning wordt maandag afgerond. Jij bevestigt de volgende afspraak.", citations: [{ number: 1, passage: { meeting_id: meetingId, title: "Projectoverleg", start_iso: meeting.start_iso, kind: "transcript", start_ms: 0, text: transcript[0].text, snippet: transcript[0].text } }] };
      case "meeting_templates": return [["algemeen", "General"], ["een-op-een", "One-to-one"], ["klantgesprek", "Client meeting"], ["standup", "Standup"]].map(([id, name]) => ({ id, name, builtin: true, customized: false }));
      case "template_source": return "Focus on what was discussed and agreed.\n\n## Summary\n## Decisions\n## Action items\n";
      case "mark_lines": return summary.split("\n").filter(Boolean).map(text => ({ text, mine: false }));
      case "calendar_snapshot": return { has_access: false, calendars: [], current_event: null };
      case "record_prompt": return null;
      case "check_for_updates": return { available: false, version: null, body: null };
      case "export_meeting": return { title: "Projectoverleg", markdown: summary, html: "<h2>Besluiten</h2><p>De planning wordt maandag afgerond.</p>" };
      case "choose_meetings_dir": return null;
      case "pick_style_application": return options.style_picker_cancelled ? null : { id: "com.apple.mail", name: "Mail" };
      case "start_meeting": return meetingId;
      case "save_template": return "qa-template";
      case "rename_meeting": return meetingId;
      case "summarize_meeting": return summary;
      case "retranscribe_meeting": return 0;
      case "retranscribe_meeting_audio": {
        if (!options.saved_audio || retranscriptionStatus?.running || frozenMeetingRecognition().provider_kind !== "local") throw new Error("Er is geen bewaarde audio om opnieuw te transcriberen.");
        retranscriptionStatus = { running: true, recognition: frozenMeetingRecognition(), progress: { phase: "transcribing", speaker: "others", completed: 15_000, total: 60_000 }, error: null, warning: null };
        emit("meeting-retranscription-changed", { id: args.id, status: clone(retranscriptionStatus) });
        if (options.retranscription_delay_ms) await new Promise(resolve => setTimeout(resolve, options.retranscription_delay_ms));
        retranscriptionStatus.running = false;
        retranscriptionStatus.progress = { phase: options.retranscription_error ? "failed" : "completed", speaker: null, completed: 1, total: 1 };
        retranscriptionStatus.error = options.retranscription_error || null;
        if (!options.retranscription_error) summaryOutdated = true;
        emit("meeting-retranscription-changed", { id: args.id, status: clone(retranscriptionStatus) });
        if (options.retranscription_error) throw new Error(options.retranscription_error);
        return transcript.length;
      }
      case "diarize_meeting": return { voices: 2, named: 1, me_found: false };
      case "speaker_setup": return { local_available: options.speaker_local !== false, ready: options.speaker_mode === "cloud" ? signedIn : speakerModels.every(model => model.state.kind === "downloaded") || diarizer.state.kind === "downloaded", signed_in: signedIn, model_ids: speakerModels.map(model => model.id), legacy_model: diarizer.state.kind === "downloaded", profile: clone(voiceProfile), known: clone(knownVoices), enrollment_text: "Ik lees deze tekst voor zodat Pennel mijn stem leert kennen. Zo weet Pennel in elke meeting wanneer ik aan het woord ben, ook als we samen rond één microfoon zitten. Vandaag is een rustige dag: de koffie is nog warm, de zon schijnt door het raam en op mijn bureau ligt een schrift vol ideeën. Straks overlopen we de planning, verdelen we de taken en spreken we af wie welke vragen beantwoordt." };
      case "meeting_voices": return clone(meetingVoices);
      case "remember_speaker": { const voice = meetingVoices.find(item => item.voice === args.voice); if (voice) { voice.name = args.name; voice.remembered = true; } return null; }
      case "start_voice_enrollment": { let seconds = 0; globalThis.__PENNEL_ENROLLMENT__ = setInterval(() => emit("voice-enrollment", { seconds: ++seconds, level: 0.1, stopped: false }), 1000); return null; }
      case "finish_voice_enrollment": clearInterval(globalThis.__PENNEL_ENROLLMENT__); if (options.enrollment_delay_ms) await new Promise(resolve => setTimeout(resolve, options.enrollment_delay_ms)); if (options.enrollment_error) throw new Error(options.enrollment_error); voiceProfile = { created_iso: "2026-10-25T09:00:00+01:00", speech_seconds: 21 }; return clone(voiceProfile);
      case "cancel_voice_enrollment": clearInterval(globalThis.__PENNEL_ENROLLMENT__); return null;
      case "forget_voice_profile": voiceProfile = null; return null;
      case "rename_known_voice": knownVoices = knownVoices.map(voice => voice.id === args.id ? { ...voice, name: args.name } : voice); return null;
      case "forget_known_voice": knownVoices = knownVoices.filter(voice => voice.id !== args.id); return null;
      case "forget_all_voices": knownVoices = []; voiceProfile = null; return null;
      case "cloud_account": if (!signedIn) throw notSignedIn(); return clone(cloudAccount);
      case "cloud_devices": if (!signedIn) throw notSignedIn(); return clone(cloudDevices);
      case "claude_access": case "set_claude_access": return { enabled: Boolean(options.claude), status: options.claude ? { kind: "connected", last_request_ms: now - 2 * 60_000 } : { kind: "off" }, mcp_url: "https://api.pennel.app/mcp" };
      case "cloud_sync_setup":
        if (options.cloud_sync_setup_delay_ms) await new Promise(resolve => setTimeout(resolve, options.cloud_sync_setup_delay_ms));
        return syncSetup();
      case "cloud_sync_prepare":
        if (!signedIn) throw notSignedIn();
        if (syncSetupPhase === "locked") throw { code: "vault_locked", message: "Enter your recovery code." };
        settings.cloud_sync = false; syncSetupPhase = "ready"; recoveryCode = "QAFI-XTUR-E000-0000-0000-0000";
        if (options.recovery_delay_ms) await new Promise(resolve => setTimeout(resolve, options.recovery_delay_ms));
        return recoveryCode;
      case "cloud_sync_activate": case "cloud_sync_resume":
        if (!signedIn || syncSetupPhase === "locked") throw { code: "vault_locked", message: "Enter your recovery code." };
        settings.cloud_sync = true; syncSetupPhase = "active"; emit("sync-changed", syncReport()); return null;
      case "cloud_sync_unlock": signedIn = true; settings.cloud_sync = true; syncSetupPhase = "active"; emit("sync-changed", syncReport()); return null;
      case "cloud_recovery_code_context":
        if (!signedIn || args.code !== recoveryCode) throw { code: "recovery_code", message: "This recovery code changed." };
        return `qa-recovery-context:${cloudAccount.email}:${recoveryCode}`;
      case "cloud_recovery_confirm":
        if (!signedIn || args.code !== recoveryCode || args.context !== `qa-recovery-context:${cloudAccount.email}:${recoveryCode}`) throw { code: "recovery_changed", message: "This recovery code changed. Save the current code." };
        settings.cloud_sync = true; syncSetupPhase = "active"; emit("sync-changed", syncReport()); return null;
      case "cloud_sync_new_recovery_code": recoveryCode = "QAFI-XTUR-E111-1111-1111-1111"; return recoveryCode;
      case "cloud_sync_reset": signedIn = true; settings.cloud_sync = true; syncSetupPhase = "active"; recoveryCode = "QAFI-XTUR-E222-2222-2222-2222"; return recoveryCode;
      case "cloud_link_availability":
        if (options.link_availability_delay_ms) await new Promise(resolve => setTimeout(resolve, options.link_availability_delay_ms));
        if (options.link_availability_error) throw { code: options.link_availability_error, message: "Synthetic service access failure." };
        return { available: options.link_available !== false, reason_code: options.link_available === false ? options.link_reason || "pairing_unavailable" : null };
      case "cloud_link_view": return clone(deviceLink);
      case "cloud_link_start":
        if (options.link_start_error && (!options.link_start_fail_once || linkStartFailures++ === 0)) {
          deviceLink = { ...emptyLink(), phase: "failed", role: args.role, error_code: options.link_start_error, error_message: "Synthetic connection admission failure." };
          throw { code: deviceLink.error_code, message: deviceLink.error_message };
        }
        linkPolls = 0; deviceLink = fixtureLink("waiting_peer", args.role); return clone(deviceLink);
      case "cloud_link_join": linkPolls = 0; deviceLink = fixtureLink(args.role === "trusted_device" ? "awaiting_approval" : "waiting_approval", args.role); return clone(deviceLink);
      case "cloud_link_poll":
        if (options.link_auto_complete && ++linkPolls > 0) return completeLink();
        return clone(deviceLink);
      case "cloud_link_approve": return completeLink();
      case "cloud_link_cancel": deviceLink = emptyLink(); return clone(deviceLink);
      case "cloud_sync_last": return syncReport();
      case "cloud_folder_warning": return null;
      case "meeting_sync_states": return signedIn && !options.empty ? [{ id: meetingId, state: options.cloud === "conflict" ? "conflict" : "synced" }] : [];
      case "meeting_conflict_files": return options.cloud === "conflict" ? ["Notities (conflict MacBook van Noor).md"] : [];
      case "cloud_login_start": return { user_code: "QAFX-2026", verification_uri: "https://example.com/qa-login", device_code: "qa-device-code", interval_secs: 5 };
      case "cloud_login_poll": return { kind: "pending" };
      case "cloud_sync_now": return syncReport();
      case "cloud_logout": signedIn = false; settings.cloud_sync = false; syncSetupPhase = "signed_out"; deviceLink = emptyLink(); emit("sync-changed", syncReport()); emit("settings-changed", null); return null;
      case "cloud_sync_disable": settings.cloud_sync = false; syncSetupPhase = "paused"; emit("sync-changed", syncReport()); return null;
      case "cloud_rename_device": case "cloud_revoke_device": case "cloud_sync_confirm_deletes":
      case "delete_conflict_copy": case "open_url": case "copy_secret_text": return null;
      case "show_window": case "show_workspace": case "copy_text": case "play_meeting_audio": case "stop_audio":
      case "answer_record_prompt": case "toggle_dictation": case "install_update": case "share_meeting": case "open_log": case "open_path":
      case "set_meeting_attendees": case "rename_speaker":
      case "delete_meeting": case "delete_template": case "test_apple_intelligence": case "stop_meeting": return null;
      default: unknown.push(command); throw new Error(`Onbekend fixturecommando: ${command}`);
    }
  }
  globalThis.__PENNEL_PLATFORM__ = {
    fixture,
    invoke,
    async listen(name, handler) {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name).add(handler);
      return () => listeners.get(name)?.delete(handler);
    },
  };
  if (profileKey && globalThis.addEventListener) globalThis.addEventListener("storage", event => {
    if (event.key !== profileKey || !event.newValue) return;
    try { settings = { ...settings, ...JSON.parse(event.newValue) }; emit("settings-changed", null); } catch {}
  });
  globalThis.__PENNEL_FIXTURE__ = { options: fixture, calls, unknown, emit, info, completeDownload };
})();
