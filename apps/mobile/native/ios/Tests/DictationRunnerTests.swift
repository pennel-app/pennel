import Foundation
import Testing
@testable internal import PennelMobile

private final class FakeTranscriber: SystemTranscriber, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [(count: Int, sampleRate: UInt32, language: String?, vocabulary: [String])] = []
    var calls: [(count: Int, sampleRate: UInt32, language: String?, vocabulary: [String])] { lock.withLock { recorded } }

    func transcribe(samples: [Float], sampleRate: UInt32, language: String?, vocabulary: [String]) throws -> String {
        lock.withLock { recorded.append((samples.count, sampleRate, language, vocabulary)) }
        return "hallo wereld"
    }
}

private func tempEngine() throws -> PennelEngine {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-test-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return try PennelEngine.newPreview(dataDir: dir.path)
}

private func appleSpeechSettings(_ engine: PennelEngine) throws {
    try FirstRunDefaults.apply(to: engine, appleIntelligenceAvailable: false, hasKey: { _ in false })
}

private func tone(seconds: Double) -> [Float] {
    (0..<Int(16_000 * seconds)).map { 0.4 * sin(Float($0) * 0.05) }
}

struct DictationRunnerTests {
    @Test func apple_preview_uses_frozen_vocabulary_and_never_becomes_the_final_result() async throws {
        let engine = try tempEngine()
        let transcriber = FakeTranscriber()
        engine.setSystemTranscriber(transcriber: transcriber)
        try appleSpeechSettings(engine)
        var settings = engine.settings()
        settings.language = "nl"
        settings.dictionary = ["Pennel Mobile", "pennel mobile"]
        try engine.updateSettings(settings: settings)
        let factory = PreviewFactoryFixture()
        let values = PreviewTextFixture()
        let runner = DictationRunner(engine: engine, keys: { ApiKeys(byProvider: [:]) }, previewFactory: { factory.make(language: $0, vocabulary: $1, onText: $2) })
        settings.dictionary = ["later changed"]
        try engine.updateSettings(settings: settings)
        runner.startPreview { values.append($0) }
        runner.feed(samples: [0.2, 0.3])
        #expect(factory.language == "nl")
        #expect(factory.vocabulary == ["Pennel Mobile"])
        #expect(values.values == ["voorlopige woorden"])
        let result = try await runner.run(samples: tone(seconds: 1)).get()
        #expect(result.text == "hallo wereld")
        #expect(transcriber.calls.first?.vocabulary == ["Pennel Mobile"])
        factory.preview.emitLate()
        #expect(values.values == ["voorlopige woorden"])
    }

    @Test func non_apple_provider_never_starts_a_shadow_apple_preview() throws {
        let engine = try tempEngine()
        var settings = engine.settings()
        let provider = try #require(settings.providers.first { $0.kind == .groq })
        settings.dictationStt = ModelChoice(providerId: provider.id, model: "whisper-large-v3-turbo")
        try engine.updateSettings(settings: settings)
        let factory = PreviewFactoryFixture()
        let runner = DictationRunner(engine: engine, previewFactory: { factory.make(language: $0, vocabulary: $1, onText: $2) })
        runner.startPreview { _ in Issue.record("unexpected preview") }
        runner.feed(samples: [0.2])
        runner.cancel()
        #expect(factory.calls == 0)
        #expect(try engine.history(limit: 10).isEmpty)
    }

    @Test func apple_auto_language_means_device_preferred_and_context_is_bounded() {
        #expect(AppleSpeechTranscriber.requestedLocale(language: nil, preferredLanguages: ["nl-BE", "en-GB"]).identifier == "nl-BE")
        #expect(AppleSpeechTranscriber.requestedLocale(language: "en-GB", preferredLanguages: ["nl-BE"]).identifier == "en-GB")
        #expect(AppleSpeechTranscriber.analysisContext(vocabulary: (0..<120).map { "Naam \($0)" }).contextualStrings[.general]?.count == 100)
    }

    @Test func runner_freezes_cleanup_verification_before_recognition() throws {
        let engine = try tempEngine()
        let runner = DictationRunner(engine: engine, keys: { ApiKeys(byProvider: [:]) })
        #expect(runner.cleanupVerificationAvailability()?.enabled == false)
        var settings = engine.settings()
        settings.cleanupVerificationEnabled = true
        try engine.updateSettings(settings: settings)
        #expect(engine.cleanupVerificationAvailability().enabled)
        #expect(runner.cleanupVerificationAvailability()?.enabled == false)
        runner.cancel()
        #expect(try engine.history(limit: 10).isEmpty)
    }

    @Test func runner_passes_16k_and_no_bundle_id() async throws {
        let engine = try tempEngine()
        let fake = FakeTranscriber()
        engine.setSystemTranscriber(transcriber: fake)
        try appleSpeechSettings(engine)

        let samples = tone(seconds: 1)
        let result = try await DictationRunner(engine: engine, keys: { ApiKeys(byProvider: [:]) }).run(samples: samples).get()

        #expect(fake.calls.count == 1)
        #expect(fake.calls.first?.sampleRate == 16_000)
        #expect(fake.calls.first?.count == samples.count)
        #expect(result.raw == "hallo wereld")
        let entry = try #require(try engine.history(limit: 10).first)
        #expect(entry.appBundleId == nil)
    }

    @Test func no_speech_has_an_english_diagnostic_and_stable_presentation_code() async throws {
        let engine = try tempEngine()
        engine.setSystemTranscriber(transcriber: FakeTranscriber())
        try appleSpeechSettings(engine)

        let result = await DictationRunner(engine: engine, keys: { ApiKeys(byProvider: [:]) })
            .run(samples: [Float](repeating: 0, count: 16_000))

        guard case .failure(let error) = result else { Issue.record("expected a failure"); return }
        #expect(error == .NoSpeech)
        #expect(ErrorText.message(error) == "No speech recognized. Try again.")
        #expect(ErrorText.failureCode(error) == "no_speech")
    }

    @Test func first_run_defaults_pick_apple_providers_without_overwriting() throws {
        let fresh = try tempEngine()
        try FirstRunDefaults.apply(to: fresh, appleIntelligenceAvailable: true, hasKey: { _ in false })
        var settings = fresh.settings()
        func kind(_ choice: ModelChoice) -> ProviderKind? { settings.providers.first { $0.id == choice.providerId }?.kind }
        #expect(kind(settings.dictationStt) == .appleSpeech)
        #expect(kind(settings.meetingStt) == .appleSpeech)
        #expect(kind(settings.llm) == .appleIntelligence)
        #expect(settings.mode == .clean)

        let unavailable = try tempEngine()
        try FirstRunDefaults.apply(to: unavailable, appleIntelligenceAvailable: false, hasKey: { _ in false })
        #expect(unavailable.settings().mode == .fast)
        #expect(unavailable.settings().providers.contains { $0.kind == .appleIntelligence } == false)

        let groq = try #require(settings.providers.first { $0.kind == .groq } ?? fresh.settings().providers.first { $0.kind == .groq })
        settings.dictationStt = ModelChoice(providerId: groq.id, model: "whisper-large-v3")
        settings.llm = ModelChoice(providerId: groq.id, model: "llama")
        settings.mode = .clean
        try fresh.updateSettings(settings: settings)
        try FirstRunDefaults.apply(to: fresh, appleIntelligenceAvailable: true, hasKey: { _ in false })
        let kept = fresh.settings()
        #expect(kept.dictationStt == ModelChoice(providerId: groq.id, model: "whisper-large-v3"))
        #expect(kept.llm == ModelChoice(providerId: groq.id, model: "llama"))
        #expect(kept.mode == .clean)
    }

    @Test func first_run_defaults_keep_a_keyed_factory_llm() throws {
        let engine = try tempEngine()
        let before = engine.settings()
        try FirstRunDefaults.apply(to: engine, appleIntelligenceAvailable: true, hasKey: { _ in true })
        let after = engine.settings()
        #expect(after.llm == before.llm)
        #expect(after.providers.first { $0.id == after.llm.providerId }?.kind == .groq)
        #expect(after.mode == before.mode)
    }
}

private final class PreviewTextFixture: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [String] = []
    var values: [String] { lock.withLock { recorded } }
    func append(_ text: String) { lock.withLock { recorded.append(text) } }
}

private final class PreviewFactoryFixture: @unchecked Sendable {
    private let lock = NSLock()
    let preview = PreviewFixture()
    private var savedLanguage: String?
    private var savedVocabulary: [String] = []
    private var count = 0
    var language: String? { lock.withLock { savedLanguage } }
    var vocabulary: [String] { lock.withLock { savedVocabulary } }
    var calls: Int { lock.withLock { count } }
    func make(language: String?, vocabulary: [String], onText: @escaping @Sendable (String) -> Void) -> any DictationPreviewing {
        lock.withLock { savedLanguage = language; savedVocabulary = vocabulary; count += 1 }
        preview.setCallback(onText)
        return preview
    }
}

private final class PreviewFixture: DictationPreviewing, @unchecked Sendable {
    private let lock = NSLock()
    private var callback: (@Sendable (String) -> Void)?
    func setCallback(_ callback: @escaping @Sendable (String) -> Void) { lock.withLock { self.callback = callback } }
    func start() -> Bool { true }
    func feed(_ samples: [Float]) { lock.withLock { callback }?("voorlopige woorden") }
    func stop() {}
    func emitLate() { lock.withLock { callback }?("late words") }
}
