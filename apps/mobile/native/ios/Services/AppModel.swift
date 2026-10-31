import Foundation
import Observation
import os

/// Owns the one `PennelEngine` of the app. Views read `settings` and write through `update`.
@MainActor
@Observable
final class AppModel {
    static let shared = AppModel.start()

    let engine: PennelEngine
    private(set) var settings: Settings
    private(set) var templates: [MeetingTemplate] = []
    var errorMessage: String?
    /// The Keychain is not observable; bumping this re-renders views that show key status.
    private var keyRevision = 0

    private static let log = Logger(subsystem: "app.pennel.ios", category: "engine")

    private init(engine: PennelEngine) {
        self.engine = engine
        settings = engine.settings()
        templates = engine.meetingTemplates()
        pinMeetingsDirectory()
        registerSystemProviders()
        registerSecretStore()
        applyFirstRunDefaults()
    }

    private static func start() -> AppModel {
        do {
            try FileManager.default.createDirectory(at: AppPaths.dataDirectory, withIntermediateDirectories: true)
            return AppModel(engine: try PennelEngine(dataDir: AppPaths.dataDirectory.path))
        } catch {
            let code = ErrorText.code(error)
            log.fault("engine failed to start: \(code, privacy: .public)")
            fatalError("engine failed to start: \(code)")
        }
    }

    /// The container path changes when iOS reinstalls or updates the app, so the meetings folder is re-pointed to
    /// the current `Documents/Meetings` on every launch instead of trusting the stored absolute path.
    private func pinMeetingsDirectory() {
        let meetings = AppPaths.meetingsDirectory
        do {
            try FileManager.default.createDirectory(at: meetings, withIntermediateDirectories: true)
        } catch {
            Self.log.error("meetings folder: \(ErrorText.code(error), privacy: .public)")
        }
        if settings.meetingsDir != meetings.path {
            update { $0.meetingsDir = meetings.path }
        }
    }

    private func registerSystemProviders() {
        engine.setSystemTranscriber(transcriber: AppleSpeechTranscriber())
        engine.setSystemLanguageModel(model: AppleIntelligenceModel())
    }

    private func registerSecretStore() {
        do {
            try engine.setSecretStore(store: KeychainSecretStore())
        } catch {
            Self.log.error("secret store: \(ErrorText.code(error), privacy: .public)")
        }
    }

    /// The core changes settings itself when sync is switched on or off.
    func reloadSettings() {
        settings = engine.settings()
    }

    private static let defaultsAppliedKey = "firstRunDefaultsApplied"

    /// Runs once: afterwards the user's own choices, including Fast mode, are never reset.
    private func applyFirstRunDefaults(availability: Bool = FoundationLanguageModel.isAvailable) {
        guard !AppPaths.defaults.bool(forKey: Self.defaultsAppliedKey) else { return }
        do {
            try FirstRunDefaults.apply(to: engine, appleIntelligenceAvailable: availability)
            AppPaths.defaults.set(true, forKey: Self.defaultsAppliedKey)
            settings = engine.settings()
        } catch {
            Self.log.error("first run defaults: \(ErrorText.code(error), privacy: .public)")
        }
    }

    func update(_ change: (inout Settings) -> Void) {
        var next = settings
        change(&next)
        guard next != settings else { return }
        do {
            try engine.updateSettings(settings: next)
        } catch {
            Self.log.error("settings not saved: \(ErrorText.code(error), privacy: .public)")
            errorMessage = ErrorText.failed(error)
        }
        settings = engine.settings()
    }

    func reloadTemplates() {
        templates = engine.meetingTemplates()
    }

    func hasKey(for providerId: String) -> Bool {
        _ = keyRevision
        return Keychain.hasKey(for: providerId)
    }

    func setKey(_ key: String?, for providerId: String) {
        if !Keychain.set(key: key, for: providerId) {
            Self.log.error("api key not saved")
            errorMessage = "Mislukt"
        }
        keyRevision += 1
    }
}
