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
  if (options.insights_seeded) transcript.splice(0, transcript.length,
    { start_ms: 0, end_ms: 8000, speaker: "others", text: "Noor will send the draft on Friday.", failed: false, voice: 1 },
    { start_ms: 8000, end_ms: 16000, speaker: "me", text: "We might move the deadline, but no decision was made.", failed: false, voice: null },
    { start_ms: 16000, end_ms: 24000, speaker: "others", text: "Who will review the final proposal?", failed: false, voice: 2 },
    { start_ms: 24000, end_ms: 32000, speaker: "others", text: "The earlier plan was withdrawn; Sam owns the follow-up now.", failed: false, voice: 2 });
  const richUtterance = (segment, index) => ({ ...segment, utterance_id: segment.utterance_id || `qa-utterance-${index}`,
    source_id: segment.source_id || (segment.speaker === "me" ? "microphone" : "system"),
    attribution: segment.attribution || (segment.voice == null ? { kind: "unknown", voices: [] } : { kind: "single", voices: [segment.voice] }), attribution_revision: segment.attribution_revision || 1 });
  if (options.rich_utterances) transcript.forEach((segment, index) => Object.assign(segment, richUtterance(segment, index),
    options.rich_attribution_states ? { attribution: { kind: ["overlap", "unknown", "mixed"][index], voices: index === 1 ? [] : [1, 2] } } : {}));
  if (options.rich_provisional) transcript.forEach(segment => { segment.attribution = { ...segment.attribution, provisional: true }; });
  if (options.insights_seeded) transcript.forEach((segment, index) => { segment.utterance_id = `00000000-0000-7000-8000-${String(index + 1).padStart(12, "0")}`; });
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
    is_cli_agent: false, availability: clone(options.provider_availability?.[kind] || available()),
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
    appearance: options.appearance, dictation_pad_auto_paste: false, live_text_enabled: options.live_text_enabled !== false, cloud_sync: Boolean(options.cloud), claude_access: Boolean(options.claude),
    onboarding: { version: 1, status: options.onboarding_status || (options.window === "onboarding" ? "pending" : options.shortcut_denied ? "skipped" : "completed"), resume_step: options.step || "welcome", goal: options.goal || "dictation" },
  };
  if (options.cleanup_connection) {
    settings.providers.push({ id: "qa-verifier", kind: "custom", name: "Synthetic verifier", base_url: "https://verifier.example.invalid/v1" });
    settings.decision = { provider_id: "qa-verifier", protocol: "perplexity", model: "synthetic-unqualified", expected_model: null, upstream: null, max_input_bytes: 16000, timeout_secs: 3 };
  }
  if (options.inference_cloud) {
    settings.providers.push({ id: "qa-cloud", kind: "pennel_cloud", name: "Pennel Cloud", base_url: "https://api.pennel.app/v1" });
    if (options.inference_cloud !== "signed_out") {
      settings.decision = { provider_id: "qa-cloud", protocol: "pennel_cloud", model: "pennel-auto", expected_model: null, upstream: null, max_input_bytes: 16000, timeout_secs: 30 };
      settings.reranking = clone(settings.decision);
    }
  }
  if (persistedSettings && typeof persistedSettings === "object") settings = { ...settings, ...persistedSettings };
  for (const slot of ["dictation_stt", "meeting_stt", "llm"]) if (options.model_choices?.[slot]) settings[slot] = clone(options.model_choices[slot]);
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
  const frontendFlushes = [];
  const copiedTexts = [];
  const unknown = [];
  const setupTrace = { samples: [], savedChoices: [], appliedTasks: [], catalogRequests: [] };
  const catalogAttempts = new Map();
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
  let credentialStore = clone(options.credential_store || { status: "ready", pending: 0 });
  const info = () => ({ interface_language: interfaceLanguage(), system_interface_language: systemInterfaceLanguage, settings: clone(settings), keys: clone(keys), credential_store: clone(credentialStore), provider_revisions: clone(providerRevisions), cloud_account_epoch: String(cloudAccountEpoch), cloud_signed_in: signedIn, active_model_downloads: [...activeDownloads], cleanup_verification: cleanupVerification(),
    unsaved_history_count: history.filter(entry => entry.status === "unsaved").length,
    kinds: clone(kinds), platform: options.platform || "macos", version: "0.1.0", hotkey_error: shortcutDenied() ? capabilities.global_hotkey.reason : null,
    log_path: "/QA/Pennel Preview/pennel.log", data_dir: "/QA/Pennel Preview", capabilities: clone(capabilities), recording_app_ids: options.recording_app_ids || [], diarizer_model_id: diarizer.id, preview: true, migration_warning: null });
  const history = [
    ...(options.history_unsaved ? [{ id: "qa-dictation-unsaved", created_at_ms: now - 60_000, app_bundle_id: "com.apple.mail", raw: sentence, text: sentence, status: "unsaved", error: "De dicteergeschiedenis kon niet worden bewaard.", has_audio: false }] : []),
    { id: "qa-dictation-2", created_at_ms: now - 15 * 60_000, app_bundle_id: "dev.pennel.Pennel.preview", raw: sentence, text: sentence, status: "done", error: null, has_audio: false },
    { id: "qa-dictation-1", created_at_ms: now - 60 * 60_000, app_bundle_id: "com.apple.mail", raw: "ik werk het voorstel bij en stuur het morgen door", text: "Ik werk het voorstel bij en stuur het morgen door.", status: "done", error: null, has_audio: false },
    { id: "qa-dictation-0", created_at_ms: now - 26 * 60 * 60_000, app_bundle_id: "com.tinyspeck.slackmacgap", raw: "kan iemand de notities van gisteren delen", text: "Kan iemand de notities van gisteren delen?", status: "done", error: null, has_audio: false },
    { id: "qa-dictation-failed", created_at_ms: now - 27 * 60 * 60_000, app_bundle_id: "com.apple.Notes", raw: "", text: "", status: "failed", error: "Geen verbinding met de provider.", has_audio: true },
  ];
  // `cloud` is unset (signed out), "synced" or "conflict"; all accounts, devices and codes are synthetic.
  let signedIn = Boolean(options.cloud) || Boolean(options.inference_cloud && options.inference_cloud !== "signed_out");
  let cloudAccountEpoch = 0;
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
  let recordingStopped = false;
  let transcriptRevision = 1;
  let saveFailure = Boolean(options.live_save_error);
  let noteSaveDelay = options.note_save_delay_ms || 0;
  const pendingNoteSaves = new Set();
  const liveRequests = new Map();
  let meetingUid = "qa-meeting-stable-uid";
  let liveSession = "qa-recording";
  let previewRevision = 1;
  const previewTracks = ["me", "others"].map(source => ({ source, revision: 1, epoch: 0,
    chunk_start_ms: 21000, start_ms: 21000, end_ms: 24000, text: "", mode: "local", state: settings.live_text_enabled ? "ready" : "disabled",
    ...clone(options.caption_tracks?.find(track => track.source === source) || {}) }));
  const previewSnapshot = () => ({ session_id: `preview-${liveSession}`, revision: previewRevision,
    transcript_revision: transcriptRevision, tracks: clone(previewTracks) });
  let activeMeetingId = meetingId;
  let metadataStatus = options.metadata_status || "valid";
  let playbackRequest = null;
  const initialLiveSpeakers = () => ({ enabled: Boolean(options.live_speaker_enabled), available: !options.live_speaker_reason,
    state: options.live_speaker_reason ? "unavailable" : options.live_speaker_enabled ? "running" : "off",
    reason_code: options.live_speaker_reason || null, revision: 1,
    model: options.live_speaker_reason === "local_required" ? null : "Owned segmentation + embedding models",
    destination: options.live_speaker_reason === "local_required" || options.live_speaker_reason === "unsupported" ? null : "local" });
  let liveSpeakerStatus = initialLiveSpeakers();
  let liveSpeakerDelay = options.live_speaker_delay_ms || 0;
  const retainedMeetings = new Map();
  const storedTranscript = segments => segments.map(segment => segment.utterance_id ? clone(segment) : ({ ...segment, end_ms: segment.start_ms,
    text: segment.text.trim().replace(/\p{White_Space}+/gu, " "), voice: segment.speaker === "others" ? segment.voice ?? null : null }));
  const fingerprint = async text => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text))), byte => byte.toString(16).padStart(2, "0")).join("");
  const evidenceValue = segment => segment.utterance_id
    ? [segment.utterance_id, segment.source_id ?? null, segment.start_ms, segment.end_ms, segment.failed ? "" : segment.text.split(/\p{White_Space}+/u).filter(Boolean).join(" "), segment.failed]
    : [segment.start_ms, segment.speaker, segment.text.trim().replace(/\p{White_Space}+/gu, " "), segment.failed, segment.speaker === "others" ? segment.voice ?? null : null];
  const refreshFingerprints = () => Promise.all(transcript.filter(segment => segment.utterance_id).map(async segment => { segment.source_fingerprint = await fingerprint(JSON.stringify(evidenceValue(segment))); }));
  const completeLiveQuestion = (requestId, force = false) => {
    const request = liveRequests.get(requestId);
    if (!request || !force && (request.cancelled || request.completed)) return false;
    request.completed = true;
    const anchor = request.snapshot.anchors.find(anchor => anchor.passage.kind === "transcript");
    emit("live-question-result", { snapshot: request.snapshot,
      answer: { text: "Synthetic frozen answer [1]", citations: anchor ? [{ number: 1, passage: anchor.passage }] : [] }, error: null });
    return true;
  };
  const meetingStatus = (includeContent = true) => {
    const retained = options.section === "recording" && !recordingRecovered && !recordingStopped;
    const storageError = retained && recordingStorageError;
    const paused = retained && Boolean(options.transcription_paused);
    const persistenceError = storageError ? "Je opname kon niet volledig worden bewaard. Probeer opnieuw te bewaren." : null;
    return {
      recording: retained && !storageError, phase: storageError ? "error" : retained ? "recording" : "idle", busy: retained && !storageError,
      error: persistenceError, session_id: retained ? liveSession : null, meeting_uid: retained ? meetingUid : null, id: retained ? activeMeetingId : null,
      title: meeting.title, elapsed_secs: 754, system_audio: true, heard_system_audio: true,
      echo_cancellation_active: retained && !storageError, audio_warning: null, persistence_error: persistenceError,
      transcription_warning: paused ? "Transcriptie is tijdelijk gepauzeerd. Je audio en herstelbestanden blijven lokaal bewaard." : null,
      terminal_error: storageError ? "De opname is gestopt omdat bewaren niet lukt." : null, can_retry_persistence: storageError,
      pending: storageError ? 1 : paused ? 3 : 0, transcript_revision: retained ? transcriptRevision : 0,
      ...(retained && options.rich_utterances ? { metadata_status: metadataStatus } : {}),
      ...(retained && options.live_captions ? { preview: previewSnapshot() } : {}),
      ...(retained && options.live_speakers ? { live_speaker_attribution: clone(liveSpeakerStatus) } : {}),
      ...(includeContent ? { notes, transcript: retained ? clone(transcript) : [] } : {}),
      ...(retained ? { recognition: { provider_kind: "local", provider_name: "Lokaal", model: settings.meeting_stt.model, language: "nl" } } : {}),
    };
  };

  const insightStores = new Map();
  const insightJobs = new Map();
  let insightSequence = 100;
  const insightId = () => `00000000-0000-7000-8000-${String(++insightSequence).padStart(12, "0")}`;
  const libraryUids = new Map(library.map((info, index) => [info.id, info.id === meetingId ? "qa-meeting-stable-uid" : `qa-library-meeting-${index}`]));
  const deletedMeetingUids = new Set();
  const uidFor = id => retainedMeetings.get(id)?.meeting_uid || (id === activeMeetingId ? meetingUid : libraryUids.get(id));
  const meetingIdForUid = uid => deletedMeetingUids.has(uid) ? null : uid === meetingUid ? activeMeetingId
    : [...retainedMeetings.entries()].find(([, value]) => value.meeting_uid === uid)?.[0] || [...libraryUids.entries()].find(([, value]) => value === uid)?.[0] || null;
  const recordForId = id => {
    if (deletedMeetingUids.has(uidFor(id))) return null;
    const retained = retainedMeetings.get(id), info = retained?.info || meetingInfo(id);
    return { id, uid: uidFor(id), info, notes: retained?.notes ?? (id === activeMeetingId ? notes : `Synthetic notes for ${info.title}.`),
      transcript: retained?.transcript || transcript, summary };
  };
  let conversationFixture, conversationFixtureLoading, conversationStorageFailure = Boolean(options.conversation_storage_error);
  const conversationCommands = { conversation_snapshot: "snapshot", update_conversation: "update", begin_conversation_question: "begin", cancel_conversation_question: "cancel", conversation_source: "source" };
  async function conversations() {
    if (conversationFixture) return conversationFixture;
    conversationFixtureLoading ??= (async () => {
      const factory = globalThis.__PENNEL_CREATE_CONVERSATION_FIXTURE__ || (await import(options.conversation_fixture_url)).createConversationFixture;
      conversationFixture = factory({ now, storage: globalThis.localStorage, storageKey: profileKey ? `${profileKey}:conversations:v1` : null,
        storageUnavailable: () => conversationStorageFailure, libraryContext: () => settings.meetings_dir,
        storageReason: () => options.conversation_storage_reason || "Conversation storage is unavailable. Unsaved text is kept in this app; retry saving or copy it before quitting.",
        autoComplete: !options.live_question_manual && !options.conversation_manual, answerDelay: options.live_question_delay_ms ?? 80,
        capture: captureConversationEvidence, resolveSource: resolveConversationSource,
        requestIsCurrent: evidence => (!evidence.session_id || evidence.session_id === (evidence.meeting_uid === meetingUid ? liveSession
          : [...retainedMeetings.values()].find(value => value.meeting_uid === evidence.meeting_uid)?.session_id))
          && (!evidence.meeting_uid || !deletedMeetingUids.has(evidence.meeting_uid))
          && evidence.sources.every(source => !deletedMeetingUids.has(source.passage.meeting_uid)),
        completed: (snapshot, answer, error) => emit("live-question-result", { snapshot, answer, error }),
        resolveScope: scope => {
          if (scope.kind === "library") return { scope: { kind: "library" }, current_meeting_id: null };
          const currentId = meetingIdForUid(scope.meeting_uid) || (scope.meeting_id && uidFor(scope.meeting_id) && !deletedMeetingUids.has(uidFor(scope.meeting_id)) ? scope.meeting_id : null);
          if (!currentId) throw Error("This meeting is no longer available.");
          return { scope: { kind: "meeting", meeting_uid: uidFor(currentId) }, current_meeting_id: currentId };
        } });
      return conversationFixture;
    })();
    return conversationFixtureLoading;
  }
  async function conversationSourceRevision(anchor, record) {
    if (anchor.passage.kind === "transcript") {
      const ids = anchor.passage.utterance_ids;
      const rows = ids?.length ? ids.map(id => record.transcript.find(row => row.utterance_id === id))
        : record.transcript.filter(row => row.start_ms === anchor.passage.start_ms && row.text === anchor.source_text);
      if (!rows.length || rows.some(row => !row)) return null;
      return fingerprint(JSON.stringify(await Promise.all(rows.map(async row => [await fingerprint(JSON.stringify(evidenceValue(row))), row.attribution_revision || 0, row.confirmed_speaker_name || null]))));
    }
    return fingerprint(anchor.passage.kind === "notes" ? record.notes : record.summary);
  }
  async function resolveConversationSource(source) {
    const id = meetingIdForUid(source.passage.meeting_uid), record = id ? recordForId(id) : null;
    const revision = record && source.anchor ? await conversationSourceRevision(source.anchor, record) : null;
    return { status: revision === null ? "unavailable" : revision === source.revision ? "current" : "stale",
      passage: clone(source.passage), anchor: clone(source.anchor), current_meeting_id: id };
  }
  async function captureConversationEvidence(request) {
    const id = request.scope.kind === "meeting" ? meetingIdForUid(request.scope.meeting_uid) : activeMeetingId;
    const record = id ? recordForId(id) : null;
    if (!record) throw Error("The meeting source is no longer available.");
    if (request.session_id && request.session_id !== liveSession) throw Error("This question belongs to another recording session.");
    const notesRevision = await fingerprint(record.notes);
    if (request.expected_notes_revision && request.expected_notes_revision !== notesRevision) throw Error("The saved notes changed. Your question was not sent.");
    const anchors = [];
    for (const row of record.transcript) {
      if (row.failed || !row.text.trim()) continue;
      const rowFingerprint = await fingerprint(JSON.stringify(evidenceValue(row)));
      const passage = { meeting_id: request.session_id ? record.uid : record.id, meeting_uid: record.uid, title: record.info.title, start_iso: record.info.start_iso,
        kind: "transcript", start_ms: row.start_ms, text: `${row.speaker === "me" ? "Ik" : "Anderen"}: ${row.text}`, snippet: row.text.slice(0, 180),
        ...(row.utterance_id ? { utterance_ids: [row.utterance_id], utterance_fingerprints: [rowFingerprint] } : {}) };
      anchors.push({ passage, segment_keys: [row.utterance_id ? `utterance:${row.utterance_id}` : `${row.speaker}:${row.start_ms}:0`],
        segment_fingerprints: [rowFingerprint], notes_start: null, notes_end: null, source_text: row.text });
    }
    let offset = 0;
    for (const line of record.notes.split("\n")) {
      if (line.trim()) anchors.push({ passage: { meeting_id: record.id, meeting_uid: record.uid, title: record.info.title, start_iso: record.info.start_iso,
        kind: "notes", start_ms: null, text: line, snippet: line.slice(0, 180) }, segment_keys: [], segment_fingerprints: [],
        notes_start: offset, notes_end: offset + line.length, source_text: line });
      offset += line.length + 1;
    }
    const selected = options.live_question_selected_scope ? anchors.slice(0, Math.max(1, Math.floor(anchors.length / 2))) : anchors;
    const cutoff = Math.max(0, ...record.transcript.filter(row => !row.failed && row.text.trim()).map(row => row.end_ms));
    const sources = await Promise.all(selected.map(async (anchor, index) => ({ number: index + 1, passage: clone(anchor.passage), anchor: clone(anchor),
      status: "current", included_through_ms: cutoff, revision: await conversationSourceRevision(anchor, record) })));
    return { request_id: request.request_id, meeting_uid: request.scope.kind === "meeting" ? record.uid : null, session_id: request.session_id || null,
      transcript_revision: transcriptRevision, notes_revision: notesRevision, included_through_ms: cutoff, pending_transcription: request.session_id ? meetingStatus().pending : 0,
      anchors: clone(selected), sources, selection: { scope: options.live_question_selected_scope ? "selected_excerpts" : "whole_meeting",
        available_sources: anchors.length, included_sources: selected.length, available_history_turns: options.live_question_selected_scope ? 8 : request.history_turns,
        included_history_turns: options.live_question_selected_scope ? 2 : Math.min(6, request.history_turns), history_reduced: Boolean(options.live_question_selected_scope) || request.history_turns > 6 } };
  }
  const peopleStores = new Map();
  const peoplePendingConfirmation = new Set();
  const peopleTrace = [];
  let peopleSequence = 500;
  let peopleWriteFailure = Boolean(options.people_write_error);
  let peopleInsertDelay = options.people_insert_delay_ms || 0;
  const pendingInsertResponses = new Set();
  let contactDelay = options.people_contact_delay_ms || 0;
  let contactResult = options.people_contact_cancelled ? { status: "cancelled" } : {
    status: "selected", contact: { source: "macos_contacts", source_id: "qa-selected-contact", source_revision: "qa-contact-snapshot-1",
      display_name: "Synthetic Contact", email: "contact@example.invalid", selected_at_ms: now },
  };
  const personId = () => `00000000-0000-7000-8000-${String(++peopleSequence).padStart(12, "0")}`;
  const peopleNotes = id => retainedMeetings.get(id)?.notes ?? notes;
  const peopleTranscript = id => retainedMeetings.get(id)?.transcript || transcript;
  const peopleStore = id => {
    const uid = uidFor(id);
    if (!peopleStores.has(uid)) {
      const names = options.people_seeded ? ["Alex", "Alex", "Synthetic guest"] : ["Noor", "Sam"];
      const people = names.map((display_name, index) => ({ id: personId(), display_name, origin: "attendee", private_alias: null, email: null,
        source: { source_id: "qa-calendar-source", account_id: "qa-calendar-account", record_id: `qa-attendee-${index}`, revision: "qa-event-1" }, source_status: "available" }));
      if (options.people_seeded) people.push({ id: personId(), display_name: "Synthetic Contact", origin: "contact", private_alias: "Private synthetic alias",
        email: "contact@example.invalid", source: { source_id: "macos_contacts", account_id: null, record_id: "qa-selected-contact", revision: "qa-contact-snapshot-1" }, source_status: "snapshot" });
      const calendar = options.people_seeded ? { source: "eventkit", source_id: "qa-calendar-source", account_id: "qa-calendar-account", calendar_id: "qa-calendar",
        event_id: "qa-event", external_event_id: null, instance_id: "2026-10-24T07:00:00Z", source_revision: "qa-event-1",
        participants: names.map((name, index) => ({ source_id: `qa-attendee-${index}`, name, email: null, response: index === 0 ? "declined" : null, is_self: index === 1 })) } : null;
      peopleStores.set(uid, { schema_version: 1, meeting_uid: uid, revision: 0, people, speaker_links: [], mentions: [], calendar, recovery_status: "ready" });
    }
    return peopleStores.get(uid);
  };
  const peopleRead = async id => {
    const value = clone(peopleStore(id)), currentNotes = peopleNotes(id), rows = peopleTranscript(id);
    value.notes_revision = await fingerprint(currentNotes);
    value.source_revision = await fingerprint(JSON.stringify(rows.map(row => [row.utterance_id, row.source_fingerprint, row.attribution_revision])));
    for (const link of value.speaker_links) {
      link.state = link.utterance_ids.every((uid, index) => rows.some(row => row.utterance_id === uid && row.source_fingerprint === link._fingerprints[index])) ? "linked" : "needs_review";
      delete link._fingerprints;
    }
    for (const mention of value.mentions) {
      if (mention._notes_hash !== value.notes_revision) mention.state = "unresolved";
      delete mention._notes_hash;
    }
    return value;
  };
  async function peopleUpdate(args) {
    if (peopleWriteFailure) throw Error("Synthetic participant storage unavailable.");
    if (args.sessionId && args.sessionId !== liveSession) throw Error("The recording session changed.");
    const request = args.request || {}, before = await peopleRead(args.id), store = peopleStore(args.id);
    if (before.revision !== request.expected_revision) throw Error("Participants changed. Refresh before saving.");
    const findPerson = id => { const person = store.people.find(person => person.id === id); if (!person) throw Error("This person is no longer available."); return person; };
    const findMention = id => { const mention = store.mentions.find(mention => mention.id === id); if (!mention) throw Error("This mention is no longer available."); return mention; };
    let edited;
    switch (request.action) {
      case "create_person": {
        if (!String(request.display_name || "").trim()) throw Error("Enter a name.");
        store.people.push({ id: personId(), display_name: request.display_name, origin: request.origin || "manual", private_alias: request.private_alias || null,
          email: request.email || null, source: request.source || null, source_status: request.origin === "contact" ? "snapshot" : "available" });
        break;
      }
      case "replace_source": Object.assign(findPerson(request.person_id), { display_name: request.display_name, private_alias: request.private_alias || null,
        email: request.email || null, source: request.source || null, source_status: "snapshot" }); break;
      case "rename_person":
        if (!String(request.display_name || "").trim()) throw Error("Enter a name.");
        findPerson(request.person_id).display_name = request.display_name;
        break;
      case "set_private_alias": findPerson(request.person_id).private_alias = request.private_alias || null; break;
      case "remove_person":
        findPerson(request.person_id);
        store.people = store.people.filter(person => person.id !== request.person_id);
        store.speaker_links = store.speaker_links.filter(link => link.person_id !== request.person_id);
        store.mentions = store.mentions.filter(mention => mention.person_id !== request.person_id);
        break;
      case "invalidate_source":
        for (const person of store.people) if (person.source?.source_id === request.source_id && (!request.account_id || person.source.account_id === request.account_id)) person.source_status = "unavailable";
        break;
      case "link_speaker": {
        findPerson(request.person_id);
        if (before.source_revision !== request.expected_source_revision || !request.utterance_ids?.length) throw Error("The speaker source changed.");
        const rows = request.utterance_ids.map(uid => peopleTranscript(args.id).find(row => row.utterance_id === uid));
        if (rows.some(row => !row?.source_fingerprint)) throw Error("This speaker has no stable source.");
        const link = request.link_id ? store.speaker_links.find(link => link.id === request.link_id) : null;
        if (request.link_id && !link) throw Error("This speaker link is no longer available.");
        const next = { id: link?.id || personId(), person_id: request.person_id, utterance_ids: clone(request.utterance_ids), state: "linked", _fingerprints: rows.map(row => row.source_fingerprint) };
        if (link) Object.assign(link, next); else store.speaker_links.push(next);
        break;
      }
      case "unlink_speaker": store.speaker_links = store.speaker_links.filter(link => link.id !== request.link_id); break;
      case "insert_mention": {
        findPerson(request.person_id);
        if (before.notes_revision !== request.expected_notes_revision || peopleNotes(args.id) !== request.notes) throw Error("The notes changed. Your text was kept.");
        if (saveFailure) throw Error("Synthetic notes storage unavailable.");
        const bytes = new TextEncoder().encode(request.notes), decoder = new TextDecoder("utf-8", { fatal: true });
        if (!Number.isInteger(request.start_byte) || !Number.isInteger(request.end_byte) || request.start_byte < 0 || request.start_byte > request.end_byte || request.end_byte > bytes.length) throw Error("The note selection changed.");
        const prefix = decoder.decode(bytes.slice(0, request.start_byte)), suffix = decoder.decode(bytes.slice(request.end_byte));
        const name = String(request.visible_name || "").trim();
        if (!name) throw Error("Enter the visible name.");
        const text = `@${name}`, result = `${prefix}${text}${suffix}`;
        store.mentions.push({ id: personId(), person_id: request.person_id, visible_name: name, start_byte: request.start_byte,
          end_byte: request.start_byte + new TextEncoder().encode(text).length, state: "linked", _notes_hash: await fingerprint(result) });
        if (retainedMeetings.has(args.id)) retainedMeetings.get(args.id).notes = result; else notes = result;
        if (options.people_confirmation_pending) { store.recovery_status = "confirmation_pending"; peoplePendingConfirmation.add(store.meeting_uid); }
        edited = { notes: result, selection_start: prefix.length + text.length, selection_end: prefix.length + text.length };
        break;
      }
      case "relink_mention": {
        findPerson(request.person_id);
        const mention = findMention(request.mention_id);
        if (before.notes_revision !== request.expected_notes_revision || mention._notes_hash !== before.notes_revision) throw Error("The notes changed. Your text was kept.");
        const bytes = new TextEncoder().encode(peopleNotes(args.id));
        if (new TextDecoder("utf-8", { fatal: true }).decode(bytes.slice(mention.start_byte, mention.end_byte)) !== `@${mention.visible_name}`) throw Error("This mention's text changed.");
        mention.person_id = request.person_id;
        break;
      }
      case "unlink_mention": store.mentions = store.mentions.filter(mention => mention.id !== request.mention_id); break;
      default: throw Error("Unknown synthetic participant action.");
    }
    peopleTrace.push(clone(request));
    store.revision++;
    emit("meeting-people-changed", { meeting_uid: store.meeting_uid, revision: store.revision });
    const response = { snapshot: await peopleRead(args.id), ...edited };
    if (request.action === "insert_mention" && peopleInsertDelay) await new Promise(resolve => {
      const finish = () => { clearTimeout(timer); pendingInsertResponses.delete(finish); resolve(); };
      const timer = setTimeout(finish, peopleInsertDelay);
      pendingInsertResponses.add(finish);
    });
    return response;
  }
  const insightMetadata = value => emit("meeting-insights-changed", Object.fromEntries(["meeting_uid", "revision", "job_id", "state", "physical_running"].filter(key => key in value).map(key => [key, value[key]])));
  const insightStore = id => {
    const uid = uidFor(id);
    if (!insightStores.has(uid)) {
      const items = options.insights_seeded ? transcript.map((row, index) => ({ id: insightId(), dedup_key: `owned-ui-${index}`, revision: 1,
        kind: ["action", "decision", "open_question", "changed_commitment"][index], commitment: ["confirmed", "tentative", "unresolved", "changed"][index],
        text: row.text, edited_text: null, edit_claim: null, review_state: "proposed", owner: index === 0 ? { text: "Noor", source_index: 0, start_byte: 0, end_byte: 4 } : null,
        due: index === 0 ? { text: "Friday", source_index: 0, start_byte: row.text.indexOf("Friday"), end_byte: row.text.indexOf("Friday") + 6 } : null,
        sources: [{ meeting_uid: uid, utterance_id: row.utterance_id, source_fingerprint: row.source_fingerprint, quote: row.text, start_byte: 0, end_byte: new TextEncoder().encode(row.text).length, start_ms: row.start_ms, end_ms: row.end_ms }],
        evidence_cutoff_ms: 32000, transcript_revision: transcriptRevision, transcript_digest: "a".repeat(64), related_item_ids: [],
        processing: { identity: { protocol: "perplexity", endpoint: "https://decision.example.invalid/v1", requested_model: "owned-ui-fixture", requested_upstream: null, actual_model: "owned-ui-fixture", actual_provider: "owned", actual_revision: "owned-revision", request_id: null }, qualification_id: "owned-ui-only-not-production", language_policy: "en", task: "meeting_suggestions_v1" },
        source_state: "current", source_history: [], source_reassociated: false })) : [];
      if (items[3]) items[3].related_item_ids = [items[0].id];
      if (options.insight_unresolved_link && items[3]) { items[3].related_item_ids = []; items[3].prior_evidence = [{ kind: "action", source: clone(items[0].sources[0]), target_item_id: null }]; }
      insightStores.set(uid, { meeting_uid: uid, revision: items.length ? 1 : 0, status: options.insight_store_status || (items.length ? "ready" : "missing"), items, insertion_receipts: [] });
    }
    return insightStores.get(uid);
  };
  const insightRead = id => {
    const value = clone(insightStore(id));
    if (!["missing", "ready"].includes(value.status)) { value.items = []; return value; }
    for (const item of value.items) item.source_state = item.sources.some(source => !transcript.some(row => row.utterance_id === source.utterance_id)) ? "missing"
      : item.sources.some(source => !transcript.some(row => row.utterance_id === source.utterance_id && row.source_fingerprint === source.source_fingerprint && row.text.includes(source.quote))) ? "stale" : "current";
    return value;
  };
  const insightWrite = (id, revision) => { const value = insightStore(id); if (!["missing", "ready"].includes(value.status) || value.revision !== revision) throw Error("Reviewed items changed. Refresh before saving."); return value; };
  const insightItem = (store, id, revision) => { const item = store.items.find(item => item.id === id); if (!item || item.revision !== revision) throw Error("This item changed. Refresh before saving."); return item; };
  const itemMarkdown = item => item.edited_text !== null ? item.edited_text : `${item.text}\n\n${item.sources.map(source => `> [${Math.floor(source.start_ms / 60000)}:${String(Math.floor(source.start_ms / 1000) % 60).padStart(2, "0")}] ${source.quote}\n`).join("")}`;
  const exportMarkdown = item => {
    const clock = ms => `${String(Math.floor(ms / 60000)).padStart(2, "0")}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
    const destination = item.processing.identity.endpoint.match(/^https:\/\/([a-zA-Z0-9.:[\]-]+)(?:\/|$)/)?.[1] || "unavailable";
    const evidence = item.sources.map(source => `Meeting UID: \`${source.meeting_uid}\`\n\nUtterance: \`${source.utterance_id}\`\n\nEvidence revision: \`${source.source_fingerprint}\`\n\nUtterance range: [${clock(source.start_ms)}]–[${clock(source.end_ms)}] (${source.start_ms}–${source.end_ms} ms)\n\n> [${clock(source.start_ms)}] ${source.quote.replaceAll("\n", "\n> ")}`).join("\n\n");
    return `${item.edited_text ?? item.text}\n\n---\n\n**Source evidence (original transcript)**\n\nMeeting file: [Transcript](transcript.md).\n\nEvidence cutoff: [${clock(item.evidence_cutoff_ms)}] (${item.evidence_cutoff_ms} ms)\n\nProcessing destination: \`${destination}\`\n\nSource status: ${item.source_state}\n\n${evidence}`;
  };
  const insightSave = store => { store.status = "ready"; store.revision++; insightMetadata(store); };
  async function insightCommand(command, args) {
    const request = args.request || {};
    if (command === "insights_snapshot") return insightRead(args.id);
    if (command === "insights_availability") return { configured: !!options.insights_seeded, available: !!options.insight_qualified && args.languagePolicy !== "unknown", qualified: !!options.insight_qualified && args.languagePolicy !== "unknown",
      reason_code: args.languagePolicy === "unknown" ? "unknown_language" : options.insight_qualified ? "" : "unqualified", provider_id: options.insights_seeded ? "owned-ui" : null,
      model: options.insights_seeded ? "owned-ui-fixture" : null, endpoint: options.insights_seeded ? "https://decision.example.invalid/v1" : null, language_policy: args.languagePolicy, task: "meeting_suggestions_v1", use_case: "manual_finished_suggestions", qualification_id: options.insight_qualified ? "owned-ui-only" : null };
    if (command === "claim_insight_edit" || command === "review_insight") {
      if (options.insight_write_fail) throw Error("Synthetic review storage unavailable.");
      if (options.insight_review_delay_ms) await new Promise(resolve => setTimeout(resolve, options.insight_review_delay_ms));
      const store = insightWrite(args.id, request.store_revision), item = insightItem(store, request.item_id, request.item_revision);
      if (command === "claim_insight_edit") item.edit_claim = request.claim_id || item.edit_claim || insightId();
      else {
        const update = request.update || {};
        if (item.edit_claim && update.edit_claim !== item.edit_claim) throw Error("The edit claim changed.");
        if (update.review_state) item.review_state = update.review_state;
        if (typeof update.edited_text === "string") item.edited_text = update.edited_text;
        if (update.clear_edit) item.edited_text = null;
        if (update.release_edit_claim) item.edit_claim = null;
      }
      item.revision++; insightSave(store); return insightRead(args.id);
    }
    if (command === "insights_export") return insightRead(args.id).items.filter(item => item.review_state === "accepted").map(exportMarkdown).join("\n");
    if (command === "insight_markdown") { const item = insightItem(insightStore(args.id), args.itemId, args.itemRevision); if (item.review_state !== "accepted") throw Error("Accept this item first."); return itemMarkdown(item); }
    if (command === "prepare_insight_insertion") {
      if (options.insight_prepare_delay_ms) await new Promise(resolve => setTimeout(resolve, options.insight_prepare_delay_ms));
      const store = insightWrite(args.id, request.store_revision), item = insightItem(store, request.item_id, request.item_revision);
      if (item.review_state !== "accepted" || request.expected_notes !== notes) throw Error("The notes changed. Refresh before insertion.");
      const existing = store.insertion_receipts.find(receipt => receipt.item_id === item.id && receipt.state !== "not_inserted");
      if (existing) return { snapshot: insightRead(args.id), receipt: clone(existing), stage: false, intended_notes: null };
      const receipt = { attempt_id: request.attempt_id || insightId(), item_id: item.id, item_revision: item.revision, before_notes_hash: await fingerprint(notes), intended_notes_hash: await fingerprint(request.intended_notes), markdown: request.markdown, state: "pending", manual_resolution: null };
      store.insertion_receipts.push(receipt); insightSave(store); return { snapshot: insightRead(args.id), receipt: clone(receipt), stage: true, intended_notes: request.intended_notes };
    }
    if (command === "reconcile_insight_insertions" || command === "resolve_insight_insertion") {
      const store = insightWrite(args.id, args.storeRevision ?? request.store_revision), hash = await fingerprint(notes);
      if (command === "reconcile_insight_insertions") {
        for (const receipt of store.insertion_receipts) if (["pending", "needs_review"].includes(receipt.state)) receipt.state = hash === receipt.intended_notes_hash ? "acknowledged" : hash === receipt.before_notes_hash ? "not_inserted" : "needs_review";
      } else {
        if (request.expected_notes !== notes) throw Error("Notes changed during review.");
        const receipt = store.insertion_receipts.find(receipt => receipt.attempt_id === request.attempt_id); if (!receipt) throw Error("Insertion not found.");
        receipt.state = request.decision; receipt.manual_resolution = { state: request.decision, notes_hash: hash };
      }
      insightSave(store); return insightRead(args.id);
    }
    if (command === "reassociate_insight_source") {
      const store = insightWrite(args.id, request.store_revision), item = insightItem(store, request.item_id, request.item_revision);
      if (request.source.quote !== item.sources[request.source_index]?.quote) throw Error("The original quote must match exactly.");
      item.source_history.push(clone(item.sources[request.source_index])); item.sources[request.source_index] = clone(request.source);
      item.source_reassociated = true; item.evidence_cutoff_ms = request.evidence_cutoff_ms; item.revision++; insightSave(store); return insightRead(args.id);
    }
    if (command === "begin_insights_analysis") {
      if (!options.insight_qualified || args.languagePolicy === "unknown") throw Error("This setup is not qualified.");
      const store = insightStore(args.id), job = { job_id: insightId(), meeting_uid: store.meeting_uid, state: "running", revision: 1, error_code: null, physical_running: true, included_through_ms: 32000, completed_windows: 0, total_windows: 1 };
      insightJobs.set(job.job_id, { job, id: args.id, polls: 0 }); insightMetadata(job); return clone(job);
    }
    if (command === "insights_job" || command === "cancel_insights_analysis") {
      const entry = insightJobs.get(args.jobId); if (!entry) throw Error("Analysis not found.");
      if (command === "cancel_insights_analysis") entry.job.state = "cancelled";
      else if (++entry.polls >= (options.insight_job_polls || 2)) {
        entry.job.physical_running = false;
        if (entry.job.state !== "cancelled") { entry.job.state = options.insight_analysis_fail ? "failed" : "completed"; entry.job.completed_windows = 1;
          if (!options.insight_analysis_fail) { const store = insightStore(entry.id); if (options.insight_remove_proposed) store.items = store.items.filter(item => item.review_state !== "proposed" || item.edited_text !== null || item.edit_claim || item.source_reassociated); insightSave(store); }
        }
      }
      entry.job.revision++; insightMetadata(entry.job); return clone(entry.job);
    }
    return undefined;
  }

  async function invoke(command, args = {}) {
    calls.push(command);
    if (conversationCommands[command]) {
      if (!args.request || typeof args.request !== "object" || Array.isArray(args.request)) throw Error("The conversation request is invalid.");
      return (await conversations()).invoke(conversationCommands[command], args.request);
    }
    if (options.rich_utterances && (["meeting_status", "meeting_detail", "begin_live_question", "ask_meetings"].includes(command) || command.includes("insight") || command.includes("people"))) await refreshFingerprints();
    if (command === "meeting_people_snapshot") {
      const store = peopleStore(args.id);
      if (peoplePendingConfirmation.delete(store.meeting_uid)) store.recovery_status = "ready";
      return peopleRead(args.id);
    }
    if (command === "update_meeting_people") return peopleUpdate(args);
    if (command === "pick_meeting_contact") {
      if (contactDelay) await new Promise(resolve => setTimeout(resolve, contactDelay));
      return options.platform && options.platform !== "macos" ? { status: "unavailable", reason: "System contact selection is unavailable on this platform." } : clone(contactResult);
    }
    if (["insights_snapshot", "insights_availability", "claim_insight_edit", "review_insight", "insights_export", "insight_markdown",
      "prepare_insight_insertion", "reconcile_insight_insertions", "resolve_insight_insertion", "reassociate_insight_source",
      "begin_insights_analysis", "insights_job", "cancel_insights_analysis"].includes(command)) return insightCommand(command, args);
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
      case "credential_store_status": return clone(credentialStore);
      case "migrate_saved_credentials":
        if (options.credential_migration_error) throw new Error("Synthetic credential migration failure");
        credentialStore = { status: "ready", pending: 0 };
        return info();
      case "save_settings": {
        if (options.settings_save_error) throw new Error(options.settings_save_error);
        settings = clone(args.settings);
        setupTrace.savedChoices.push(Object.fromEntries(["dictation_stt", "meeting_stt", "llm"].map(slot => [slot, clone(settings[slot])])));
        settings.ui_language = ["en", "nl", "system"].includes(settings.ui_language) ? settings.ui_language : "system";
        if (profileKey) { try { globalThis.localStorage?.setItem(profileKey, JSON.stringify(settings)); } catch {} }
        emit("settings-changed", null);
        return info();
      }
      case "apply_task_model_choice": {
        if (!["dictation_stt", "meeting_stt", "llm"].includes(args.slot)) throw new Error("Unknown model task.");
        const choice = args.choice;
        if (!choice || typeof choice.provider_id !== "string" || typeof choice.model !== "string") throw new Error("Choose a valid provider and model.");
        const account = settings.providers.find(provider => provider.id === choice.provider_id);
        const definition = kinds.find(kind => kind.kind === account?.kind);
        if (!account || !definition) throw new Error("The selected provider is unavailable.");
        if (!(args.slot === "llm" ? definition.llm : definition.stt)) throw new Error("This provider does not support the selected task.");
        const candidate = { provider_id: choice.provider_id, model: choice.model.trim() };
        if (!definition.is_cli_agent && !candidate.model) throw new Error("Choose a model for this task.");
        if (account.kind === "local" && !models.some(model => model.id === candidate.model && model.state.kind === "downloaded")) throw new Error("Download the selected speech model first.");
        if (options.settings_save_error) throw new Error(options.settings_save_error);
        settings[args.slot] = candidate;
        setupTrace.appliedTasks.push({ slot: args.slot, choice: clone(candidate) });
        setupTrace.savedChoices.push(Object.fromEntries(["dictation_stt", "meeting_stt", "llm"].map(slot => [slot, clone(settings[slot])])));
        if (profileKey) { try { globalThis.localStorage?.setItem(profileKey, JSON.stringify(settings)); } catch {} }
        emit("settings-changed", null);
        return info();
      }
      case "get_permissions": return clone(permissions);
      case "request_permission": {
        if (["windows", "linux"].includes(options.platform) && args.kind === "microphone" && permissions.find(permission => permission.kind === args.kind)?.status === "unknown") return clone(permissions);
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
      case "frontend_flushed": frontendFlushes.push(clone(args)); return null;
      case "copy_text": copiedTexts.push(String(args.text ?? "")); return null;
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
      case "add_provider": {
        const definition = kinds.find(kind => kind.kind === args.kind);
        const prefix = `qa-${args.kind}`;
        let id = prefix;
        for (let suffix = 2; settings.providers.some(provider => provider.id === id); suffix++) id = `${prefix}-${suffix}`;
        settings.providers.push({ id, kind: args.kind, name: definition?.label || args.kind, base_url: definition?.default_base_url || "" });
        return info();
      }
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
        ["pennel_cloud", "Pennel Cloud", "https://api.pennel.app"],
      ].map(([protocol, name, default_base_url]) => ({ protocol, name, default_base_url, decision: protocol !== "tei", reranking: ["tei", "pennel_cloud"].includes(protocol), extraction: protocol === "mercury", requires_key: !["tei", "local_decider", "pennel_cloud"].includes(protocol), managed: protocol === "pennel_cloud", allowed_provider_kinds: protocol === "pennel_cloud" ? ["pennel_cloud"] : protocol.startsWith("open_router") ? ["open_router", "custom"] : ["custom"] }));
      case "inference_availability": {
        const config = settings[args.capability];
        const enabled = config?.protocol === "pennel_cloud" && signedIn && options.inference_cloud !== "unavailable";
        return { capability: args.capability, configured: !!config, available: !!enabled, reason: !config ? "Configure this optional connection first." : !signedIn ? "Sign in to Pennel Cloud first." : "Pennel Cloud models are temporarily unavailable.", provider_id: config?.provider_id ?? null, model: config?.model ?? null, protocol: config?.protocol ?? null, endpoint: config?.protocol === "pennel_cloud" ? `https://api.pennel.app/v1/${args.capability === "decision" ? "decisions" : "rerank"}` : null };
      }
      case "cleanup_verification_availability": return cleanupVerification();
      case "provider_models": {
        setupTrace.catalogRequests.push(args.id);
        const attempt = (catalogAttempts.get(args.id) || 0) + 1;
        catalogAttempts.set(args.id, attempt);
        if (options.provider_catalog_delay_ms) await new Promise(resolve => setTimeout(resolve, options.provider_catalog_delay_ms));
        const failures = options.provider_catalog_fail_count ?? (options.provider_catalog_fail_once ? 1 : Infinity);
        if (options.provider_catalog_error && attempt <= failures) throw new Error(options.provider_catalog_error);
        return clone(options.remote_models || [
          { id: "gpt-4o-mini-transcribe", name: "GPT-4o mini Transcribe", audio_input: true, is_router: false },
          { id: "llama-3.3-70b-versatile", name: "Llama 3.3", audio_input: false, is_router: false },
          { id: "gpt-4.1-mini", name: "GPT-4.1 mini", audio_input: false, is_router: false },
        ]);
      }
      case "test_provider_task": case "test_provider_task_choice": {
        if (!["dictation_stt", "meeting_stt", "llm"].includes(args.slot)) throw new Error("Onbekende QA-taak");
        const choice = command === "test_provider_task_choice" ? clone(args.choice) : clone(settings[args.slot]);
        if (!choice || typeof choice.provider_id !== "string" || typeof choice.model !== "string") throw new Error("Ongeldige QA-modelkeuze");
        setupTrace.samples.push({ slot: args.slot, choice: clone(choice), draft: command === "test_provider_task_choice" });
        if (options.sample_delay_ms) await new Promise(resolve => setTimeout(resolve, options.sample_delay_ms));
        if (options.sample_error) return { verified: false, message: options.sample_error };
        const provider = settings.providers.find(provider => provider.id === choice.provider_id);
        const kind = kinds.find(kind => kind.kind === provider?.kind);
        const configured = Boolean(provider && kind && kind.availability.status === "available" && (args.slot === "llm" ? kind.llm : kind.stt) && choice.model && (!kind.requires_key || keys[provider.id])
          && (provider.kind !== "pennel_cloud" || signedIn)
          && (provider.kind !== "local" || modelFor(choice.model)?.state.kind === "downloaded"));
        return { verified: configured, message: configured ? "Test geslaagd met uitsluitend synthetische voorbeelddata." : "Stel eerst het gekozen model en de provider in." };
      }
      case "detect_cli_agents": return [];
      case "check_cli_agent": return "Niet uitgevoerd in deze fixture.";
      case "meeting_status": return meetingStatus();
      case "meeting_runtime_status": return meetingStatus(false);
      case "set_live_speaker_attribution": {
        if (liveSpeakerDelay) await new Promise(resolve => setTimeout(resolve, liveSpeakerDelay));
        if (args.sessionId !== liveSession || !meetingStatus().recording) throw new Error("The recording session changed.");
        if (options.live_speaker_toggle_error) throw new Error("Synthetic speaker attribution failed.");
        if (args.enabled && !liveSpeakerStatus.available) throw new Error("Synthetic live speaker setup unavailable.");
        liveSpeakerStatus = { ...liveSpeakerStatus, enabled: Boolean(args.enabled), state: args.enabled ? "running" : "off", reason_code: null, revision: liveSpeakerStatus.revision + 1 };
        return clone(liveSpeakerStatus);
      }
      case "set_meeting_title": meeting.title = String(args.title ?? ""); return null;
      case "set_meeting_notes": case "save_meeting_notes": {
        if (noteSaveDelay) await new Promise(resolve => {
          const finish = () => { clearTimeout(timer); pendingNoteSaves.delete(finish); resolve(); };
          const timer = setTimeout(finish, noteSaveDelay); pendingNoteSaves.add(finish);
        });
        if (saveFailure) throw new Error("Synthetic notes storage unavailable.");
        if (args.id && args.id !== activeMeetingId && retainedMeetings.has(args.id)) { retainedMeetings.get(args.id).notes = String(args.text ?? ""); return null; }
        if (args.insightReceipt) {
          const store = insightStore(args.id || activeMeetingId), receipt = store.insertion_receipts.find(receipt => receipt.attempt_id === args.insightReceipt);
          if (!receipt || await fingerprint(String(args.text ?? "")) !== receipt.intended_notes_hash
            || ![receipt.before_notes_hash, receipt.intended_notes_hash].includes(await fingerprint(notes))) throw Error("Insertion notes changed.");
          notes = String(args.text ?? "");
          if (!options.insight_ack_fail) { receipt.state = "acknowledged"; insightSave(store); }
          return null;
        }
        notes = String(args.text ?? ""); return null;
      }
      case "begin_live_question": {
        if (args.meetingUid !== meetingUid || args.sessionId !== liveSession || args.expectedNotesText !== notes) throw new Error("The saved notes or meeting session changed. Try the question again.");
        if ([...liveRequests.values()].some(request => request.snapshot.meeting_uid === args.meetingUid && request.snapshot.session_id === args.sessionId && !request.cancelled && !request.completed)) throw new Error("A question is already running.");
        const frozen = clone(transcript);
        const anchors = [];
        const occurrences = new Map();
        const passage = (kind, start_ms, text) => ({ meeting_id: meetingUid, title: meeting.title, start_iso: meeting.start_iso, kind, start_ms, text, snippet: text.slice(0, 180) });
        for (const segment of frozen) {
          const base = `${segment.speaker}:${segment.start_ms}`;
          const count = occurrences.get(base) || 0; occurrences.set(base, count + 1);
          if (segment.failed || !segment.text.trim()) continue;
          anchors.push({ passage: { ...passage("transcript", segment.start_ms, `${segment.speaker === "me" ? "Ik" : "Anderen"}: ${segment.text}`),
              ...(segment.utterance_id ? { meeting_uid: meetingUid, utterance_ids: [segment.utterance_id], utterance_fingerprints: [segment.source_fingerprint] } : {}) },
            segment_keys: [segment.utterance_id ? `utterance:${segment.utterance_id}` : `${base}:${count}`], segment_fingerprints: [await fingerprint(JSON.stringify(evidenceValue(segment)))],
            notes_start: null, notes_end: null, source_text: segment.text });
        }
        let notesOffset = 0;
        for (const line of notes.split("\n")) {
          if (line.trim()) anchors.push({ passage: passage("notes", null, line), segment_keys: [], segment_fingerprints: [], notes_start: notesOffset, notes_end: notesOffset + line.length, source_text: line });
          notesOffset += line.length + 1;
        }
        const selectedAnchors = options.live_question_selected_scope ? anchors.slice(0, Math.max(1, Math.floor(anchors.length / 2))) : anchors;
        const snapshot = { request_id: args.requestId, meeting_uid: meetingUid, session_id: liveSession,
          transcript_revision: transcriptRevision, notes_revision: await fingerprint(notes), included_through_ms: Math.max(0, ...frozen.filter(segment => !segment.failed && segment.text.trim()).map(segment => segment.end_ms)),
          pending_transcription: meetingStatus().pending, anchors: selectedAnchors,
          ...(options.live_question_selected_scope ? { selection: { scope: "selected_excerpts", available_sources: anchors.length,
            included_sources: selectedAnchors.length, available_history_turns: 8, included_history_turns: 2, history_reduced: true } } : {}) };
        liveRequests.set(args.requestId, { snapshot, cancelled: false, completed: false });
        if (!options.live_question_manual) setTimeout(() => completeLiveQuestion(args.requestId), options.live_question_delay_ms ?? 80);
        return clone(snapshot);
      }
      case "cancel_live_question": {
        const request = liveRequests.get(args.requestId);
        if (request && request.snapshot.meeting_uid === args.meetingUid && request.snapshot.session_id === args.sessionId) request.cancelled = true;
        return null;
      }
      case "stop_meeting": {
        const status = meetingStatus(); recordingStopped = true;
        const event = { id: activeMeetingId, meeting_uid: meetingUid, session_id: status.session_id || liveSession, error: null, flush_warning: saveFailure ? "Synthetic notes storage unavailable." : null };
        queueMicrotask(() => emit("meeting-finished", event));
        return null;
      }
      case "retry_meeting_persistence": {
        const status = meetingStatus();
        if (!status.can_retry_persistence || args.sessionId !== status.session_id) throw new Error("Deze QA-opnamesessie is niet meer actief.");
        recordingStorageError = false; recordingRecovered = true;
        emit("meeting-finished", { id: meetingId, session_id: status.session_id, error: null, flush_warning: null });
        emit("meeting-updated", null); return null;
      }
      case "meetings": return options.empty ? [] : clone(library);
      case "meeting_detail": {
        if (deletedMeetingUids.has(uidFor(args.id))) throw Error("This meeting is no longer available.");
        return { info: clone(retainedMeetings.get(args.id)?.info || meetingInfo(args.id)), notes: retainedMeetings.get(args.id)?.notes ?? (args.id === activeMeetingId ? notes : recordForId(args.id).notes), summary, transcript: clone(retainedMeetings.get(args.id)?.transcript || (recordingStopped ? storedTranscript(transcript) : transcript)), speakers: [{ voice: 1, name: "Noor" }], dir: `/QA/Pennel Preview/Meetings/${args.id}`, attendees: ["Noor", "Sam"], audio_tracks: [{ speaker: "me", source_id: "microphone", available: !!options.saved_audio }, { speaker: "others", source_id: "system", available: !!options.saved_audio }],
        ...(options.rich_utterances ? { meeting_uid: uidFor(args.id), metadata_status: metadataStatus, revision: transcriptRevision } : {}),
        summary_outdated: summaryOutdated, retranscription_status: clone(retranscriptionStatus),
        retranscription_plan: { recognition: frozenMeetingRecognition(), available: !!options.saved_audio && !retranscriptionStatus?.running && frozenMeetingRecognition().provider_kind === "local", reason: options.saved_audio ? null : "Er is geen bewaarde audio om opnieuw te transcriberen." } };
      }
      case "search_library": return { meetings: options.empty ? [] : clone(library), passages: [] };
      case "ask_meetings": return { text: "De planning wordt maandag afgerond. Jij bevestigt de volgende afspraak.", citations: [{ number: 1, passage: { meeting_id: meetingId, title: "Projectoverleg", start_iso: meeting.start_iso, kind: "transcript", start_ms: 0, text: transcript[0].text, snippet: transcript[0].text,
        ...(transcript[0].utterance_id ? { meeting_uid: meetingUid, utterance_ids: [transcript[0].utterance_id], utterance_fingerprints: [transcript[0].source_fingerprint] } : {}) } }] };
      case "play_meeting_utterance_audio": {
        const utterance = transcript.find(segment => segment.utterance_id === args.utteranceId);
        if (!utterance?.source_id || !options.saved_audio) throw new Error("Synthetic utterance audio is unavailable");
        playbackRequest = { ...clone(args), source_id: utterance.source_id, start_ms: utterance.start_ms };
        return null;
      }
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
      case "rename_meeting": {
        const uid = uidFor(args.id), renamed = `qa-renamed-${String(args.title || "Meeting").replace(/[^\p{L}\p{N} -]/gu, "").trim()}`;
        if (!uid || deletedMeetingUids.has(uid)) throw Error("This meeting is no longer available.");
        const info = library.find(info => info.id === args.id); if (info) { info.id = renamed; info.title = args.title; }
        if (retainedMeetings.has(args.id)) { const value = retainedMeetings.get(args.id); value.info = { ...value.info, id: renamed, title: args.title }; retainedMeetings.delete(args.id); retainedMeetings.set(renamed, value); }
        libraryUids.delete(args.id); libraryUids.set(renamed, uid);
        if (activeMeetingId === args.id) { activeMeetingId = renamed; meeting.id = renamed; meeting.title = args.title; }
        emit("library-changed", null); return renamed;
      }
      case "delete_meeting": {
        const uid = uidFor(args.id); if (!uid) throw Error("This meeting is no longer available.");
        deletedMeetingUids.add(uid); retainedMeetings.delete(args.id);
        const index = library.findIndex(info => info.id === args.id); if (index >= 0) library.splice(index, 1);
        conversationFixture?.deleteMeeting(uid); emit("library-changed", null); return null;
      }
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
      case "cloud_login_start": cloudAccountEpoch++; return { user_code: "QAFX-2026", verification_uri: "https://example.com/qa-login", device_code: "qa-device-code", interval_secs: options.login_auto_complete ? 1 : 5 };
      case "cloud_login_poll":
        if (!options.login_auto_complete) return { kind: "pending" };
        signedIn = true; cloudAccountEpoch++; syncSetupPhase = "paused";
        return { kind: "done", email: cloudAccount.email };
      case "cloud_sync_now": return syncReport();
      case "cloud_logout": signedIn = false; cloudAccountEpoch++; settings.cloud_sync = false; syncSetupPhase = "signed_out"; deviceLink = emptyLink(); emit("sync-changed", syncReport()); emit("settings-changed", null); return null;
      case "cloud_sync_disable": settings.cloud_sync = false; syncSetupPhase = "paused"; emit("sync-changed", syncReport()); return null;
      case "cloud_rename_device": case "cloud_revoke_device": case "cloud_sync_confirm_deletes":
      case "delete_conflict_copy": case "open_url": case "copy_secret_text": return null;
      case "show_window": case "show_workspace": case "play_meeting_audio": case "stop_audio":
      case "answer_record_prompt": case "toggle_dictation": case "install_update": case "share_meeting": case "open_log": case "open_path":
      case "set_meeting_attendees": case "rename_speaker":
      case "delete_template": case "test_apple_intelligence": return null;
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
  globalThis.__PENNEL_FIXTURE__ = { options: fixture, calls, unknown, emit, info, completeDownload, setupTrace, frontendFlushes: () => clone(frontendFlushes), copiedTexts: () => clone(copiedTexts),
    conversations: { complete: async (requestId, result) => (await conversations()).complete(requestId, result),
      requests: () => conversationFixture?.requests() || [], failStorage: value => { conversationStorageFailure = value; },
      beginDelay: value => { if (!conversationFixture) throw Error("Load the synthetic conversation before setting its response delay."); conversationFixture.beginDelay(value); },
      pendingBeginResponses: () => conversationFixture?.pendingBeginResponses() || 0, releaseBegins: () => conversationFixture?.releaseBegins(),
      cancelDelay: value => { if (!conversationFixture) throw Error("Load the synthetic conversation before setting its response delay."); conversationFixture.cancelDelay(value); },
      pendingCancelResponses: () => conversationFixture?.pendingCancelResponses() || 0, releaseCancels: () => conversationFixture?.releaseCancels(),
      checkpoint: value => {
        if (!profileKey) throw Error("A conversation reload checkpoint requires its own synthetic profile.");
        const key = `${profileKey}:conversation-qa-checkpoint`;
        if (value !== undefined) globalThis.localStorage.setItem(key, JSON.stringify(value));
        return JSON.parse(globalThis.localStorage.getItem(key) || "null");
      } },
    people: { read: () => peopleRead(activeMeetingId), trace: () => clone(peopleTrace), failWrites: value => { peopleWriteFailure = value; },
      insertDelay: value => { peopleInsertDelay = value; }, pendingInsertResponses: () => pendingInsertResponses.size,
      releaseInserts: () => { for (const finish of [...pendingInsertResponses]) finish(); },
      contactDelay: value => { contactDelay = value; }, contactResult: value => { contactResult = clone(value); } },
    workspace: { append: segment => { transcript.push(options.rich_utterances ? richUtterance(clone(segment), transcript.length) : clone(segment)); transcriptRevision++; },
      attribution: (index, update) => { Object.assign(transcript[index], clone(update)); transcript[index].attribution_revision = (transcript[index].attribution_revision || 0) + 1; transcriptRevision++; },
      metadata: state => { metadataStatus = state; }, playback: () => clone(playbackRequest),
      liveSpeakers: update => { liveSpeakerStatus = { ...liveSpeakerStatus, ...clone(update), revision: update.revision ?? liveSpeakerStatus.revision + 1 }; },
      liveSpeakerDelay: delay => { liveSpeakerDelay = delay; },
      preview: (source, update) => {
        const track = previewTracks.find(track => track.source === source);
        if (!track) throw new Error("Unknown synthetic preview source");
        Object.assign(track, clone(update), { source, revision: ++previewRevision });
      },
      finalizePreview: (source, chunkStart, segment) => {
        transcript.push(clone(segment)); transcriptRevision++;
        const track = previewTracks.find(track => track.source === source);
        if (track?.chunk_start_ms === chunkStart) { track.text = ""; track.revision = ++previewRevision; }
      },
      requests: () => [...liveRequests.values()].map(request => clone(request.snapshot)).concat(conversationFixture?.requests() || []),
      complete: requestId => liveRequests.has(requestId) ? completeLiveQuestion(requestId) : conversationFixture?.complete(requestId),
      forceComplete: requestId => liveRequests.has(requestId) ? completeLiveQuestion(requestId, true) : conversationFixture?.complete(requestId),
      replaceTranscript: segments => { transcript.splice(0, transcript.length, ...clone(segments)); transcriptRevision++; },
      restart: (next = { id: "qa-next-meeting", uid: "qa-next-uid", session: "qa-next-session", title: "Next synthetic meeting", notes: "New meeting notes", transcript: [] }) => {
        retainedMeetings.set(activeMeetingId, { info: { ...meeting, id: activeMeetingId }, meeting_uid: meetingUid, session_id: liveSession, notes, transcript: recordingStopped ? storedTranscript(transcript) : clone(transcript) });
        activeMeetingId = next.id; meetingUid = next.uid; liveSession = next.session; meeting.title = next.title;
        notes = next.notes ?? ""; transcript.splice(0, transcript.length, ...clone(next.transcript ?? []));
        transcriptRevision = 1; recordingStopped = false; recordingRecovered = false;
        previewRevision = 1;
        liveSpeakerStatus = { ...initialLiveSpeakers(), enabled: false, state: options.live_speaker_reason ? "unavailable" : "off" };
        for (const track of previewTracks) Object.assign(track, { revision: 1, epoch: 0, chunk_start_ms: 0, start_ms: 0, end_ms: 0, text: "", state: settings.live_text_enabled ? "ready" : "disabled" });
        return { id: activeMeetingId, meeting_uid: meetingUid, session_id: liveSession, notes };
      },
      failSaves: value => { saveFailure = value; }, notes: () => notes, recording: () => meetingStatus().recording,
      notesDelay: value => { noteSaveDelay = value; }, pendingNoteSaves: () => pendingNoteSaves.size,
      releaseNotes: () => { for (const finish of [...pendingNoteSaves]) finish(); } } };
})();
