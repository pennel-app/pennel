import Foundation

public enum KeyboardInterfaceLanguage: String, CaseIterable, Sendable {
    case english = "en"
    case dutch = "nl"
}

public enum KeyboardPreferences {
    public static let suggestionsKey = "keyboardSuggestionsEnabled"
    public static let dictionaryKey = "keyboardDictionary"
    public static let languageKey = "keyboardInterfaceLanguage"
    public static let feedbackKey = "keyboardFeedbackEnabled"
    public static let typingFeedbackKey = "keyboardTypingFeedbackEnabled"

    public static func feedbackEnabled(in defaults: UserDefaults?) -> Bool {
        defaults?.object(forKey: feedbackKey) as? Bool ?? true
    }

    public static func setFeedbackEnabled(_ enabled: Bool, in defaults: UserDefaults?) {
        defaults?.set(enabled, forKey: feedbackKey)
    }

    public static func typingFeedbackEnabled(in defaults: UserDefaults?) -> Bool {
        defaults?.bool(forKey: typingFeedbackKey) ?? false
    }

    public static func setTypingFeedbackEnabled(_ enabled: Bool, in defaults: UserDefaults?) {
        defaults?.set(enabled, forKey: typingFeedbackKey)
    }

    public static func mirrorFeedback(from shared: UserDefaults?, to local: UserDefaults?) {
        setFeedbackEnabled(feedbackEnabled(in: shared), in: local)
        setTypingFeedbackEnabled(typingFeedbackEnabled(in: shared), in: local)
    }

    public static func suggestionsEnabled(in defaults: UserDefaults?) -> Bool {
        defaults?.bool(forKey: suggestionsKey) ?? false
    }

    public static func setSuggestionsEnabled(_ enabled: Bool, in defaults: UserDefaults?) {
        defaults?.set(enabled, forKey: suggestionsKey)
        if !enabled { defaults?.removeObject(forKey: dictionaryKey) }
        defaults?.set(UUID().uuidString, forKey: suggestionsRevisionKey)
        touchTypingRevision(in: defaults)
    }

    public static func dictionary(in defaults: UserDefaults?) -> [String] {
        guard suggestionsEnabled(in: defaults) else { return [] }
        return bounded(defaults?.stringArray(forKey: dictionaryKey) ?? [])
    }

    public static func publish(dictionary: [String], interfaceLanguage: String, in defaults: UserDefaults?) {
        let language = KeyboardInterfaceLanguage(rawValue: interfaceLanguage)?.rawValue
        let previousLanguage = defaults?.string(forKey: languageKey)
        let previousWords = defaults?.stringArray(forKey: dictionaryKey)
        defaults?.set(language, forKey: languageKey)
        if suggestionsEnabled(in: defaults) {
            defaults?.set(bounded(dictionary), forKey: dictionaryKey)
        } else {
            defaults?.removeObject(forKey: dictionaryKey)
        }
        if previousLanguage != language || previousWords != defaults?.stringArray(forKey: dictionaryKey) {
            touchTypingRevision(in: defaults)
        }
    }

    public static func interfaceLanguage(
        in defaults: UserDefaults?, preferredLanguages: [String] = Locale.preferredLanguages
    ) -> KeyboardInterfaceLanguage {
        if let stored = defaults?.string(forKey: languageKey), let language = KeyboardInterfaceLanguage(rawValue: stored) {
            return language
        }
        for name in preferredLanguages {
            if let language = KeyboardInterfaceLanguage(rawValue: String(name.prefix(2)).lowercased()) { return language }
        }
        return .english
    }

    private static func bounded(_ words: [String]) -> [String] {
        var result: Set<String> = []
        for raw in words {
            let word = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !word.isEmpty, word.count <= 64 else { continue }
            result.insert(word)
            if result.count == 1_024 { break }
        }
        return result.sorted()
    }
}

public enum KeyboardFailureRecovery: Equatable, Sendable { case recordAgain, checkSettings }

public struct KeyboardCopy: Sendable {
    public enum Key: CaseIterable, Sendable {
        case fullAccess, settings, ready, listening, processing, retry, insertLast, sessionOff, manualStart
        case stopDictation, recordAgain, checkAgain, setupDictation, hideHelp, accentedLetters, dismissAccents, showDictation, showTyping
        case space, returnKey, go, search, send, next, done, join, route, continueKey
        case shift, capsLock, delete, numbers, symbols, letters, showNumbers, showSymbols, showLetters, nextKeyboard, cancel, microphone
        case suggestions, suggestionsOff, suggestionsPrivacy, insertSuggestion
    }

    public let language: KeyboardInterfaceLanguage

    public init(language: KeyboardInterfaceLanguage) { self.language = language }

    public func recoveryAction(code: String?, message: String?) -> KeyboardFailureRecovery {
        let code = code ?? Self.legacyFailureCode(message)
        return ["provider_auth", "http_401", "http_403", "missing_api_key", "model_not_downloaded", "microphone_denied", "auth", "not_signed_in", "access", "vault_locked", "storage"].contains(code) ? .checkSettings : .recordAgain
    }

    public func failure(code: String?, message: String?) -> String {
        let aliases = ["not_signed_in": "auth", "http_401": "provider_auth", "http_403": "provider_auth"]
        let resolved = code.map { aliases[$0] ?? $0 } ?? Self.legacyFailureCode(message)
        switch (language, resolved) {
        case (.english, "no_speech"): return "No speech recognized. Try again."
        case (.dutch, "no_speech"): return "Geen spraak herkend. Probeer opnieuw."
        case (.english, "network"): return "No connection. Try again."
        case (.dutch, "network"): return "Geen verbinding. Probeer opnieuw."
        case (.english, "missing_api_key"): return "Set the provider API key in Pennel Settings."
        case (.dutch, "missing_api_key"): return "Stel de API-sleutel van je provider in bij Instellingen in Pennel."
        case (.english, "provider_auth"): return "Check your provider API key and model access in Pennel Settings."
        case (.dutch, "provider_auth"): return "Controleer de API-sleutel en modeltoegang van je provider bij Instellingen in Pennel."
        case (.english, "auth"), (.english, "access"): return "Check your account and provider key in Pennel Settings."
        case (.dutch, "auth"), (.dutch, "access"): return "Controleer je account en providersleutel bij Instellingen in Pennel."
        case (.english, "model_not_downloaded"): return "Download the selected model in Pennel."
        case (.dutch, "model_not_downloaded"): return "Download het gekozen model in Pennel."
        case (.english, "microphone_denied"): return "Allow microphone access in Settings."
        case (.dutch, "microphone_denied"): return "Sta de microfoon toe in Instellingen."
        case (.english, "vault_locked"): return "Unlock sync with your recovery code in Pennel."
        case (.dutch, "vault_locked"): return "Open sync met je herstelcode in Pennel."
        case (.english, "storage"): return "The dictation could not be saved. Check device storage in Pennel."
        case (.dutch, "storage"): return "Het dictaat kon niet worden bewaard. Controleer de opslag in Pennel."
        case (.english, "cancelled"): return "Dictation cancelled."
        case (.dutch, "cancelled"): return "Dictaat geannuleerd."
        case (.english, _): return "Dictation failed. Check your settings in Pennel and try again."
        case (.dutch, _): return "Dicteren mislukt. Controleer je instellingen in Pennel en probeer opnieuw."
        }
    }

    private static func legacyFailureCode(_ message: String?) -> String {
        switch message {
        case "Geen spraak herkend. Probeer opnieuw.", "Geen spraak gehoord.", "No speech recognized. Try again.": "no_speech"
        case "Geen verbinding", "No connection": "network"
        case "Geen API-sleutel ingesteld", "No API key configured": "missing_api_key"
        case "API-sleutel geweigerd", "API key rejected": "provider_auth"
        case "Model nog niet gedownload", "Model not downloaded": "model_not_downloaded"
        case "Log in bij Pennel Cloud in Instellingen → Account.", "Sign in to Pennel Cloud in Settings → Account.": "auth"
        case "Voer je herstelcode in om sync aan te zetten.", "Enter your recovery code to enable sync.": "vault_locked"
        default: "operation_failed"
        }
    }

    public func text(_ key: Key) -> String {
        switch (language, key) {
        case (.english, .fullAccess): "Enable Full Access to dictate. Typing works without it."
        case (.dutch, .fullAccess): "Zet Volledige toegang aan om te dicteren. Typen werkt ook zonder."
        case (.english, .settings): "Open Settings"
        case (.dutch, .settings): "Open Instellingen"
        case (.english, .ready): "Tap or hold to speak"
        case (.dutch, .ready): "Tik of hou ingedrukt om te praten"
        case (.english, .listening): "Listening…"
        case (.dutch, .listening): "Luistert…"
        case (.english, .processing): "Pennel is processing your dictation."
        case (.dutch, .processing): "Pennel verwerkt je dictaat."
        case (.english, .retry): "Try again"
        case (.dutch, .retry): "Opnieuw proberen"
        case (.english, .insertLast): "Insert last dictation"
        case (.dutch, .insertLast): "Laatste dictaat invoegen"
        case (.english, .sessionOff): "Session off"
        case (.dutch, .sessionOff): "Sessie uit"
        case (.english, .manualStart): "Open Pennel → Dictation → Start session, then return here."
        case (.dutch, .manualStart): "Open Pennel → Dicteren → Start sessie en kom daarna hier terug."
        case (.english, .stopDictation): "Stop dictation"
        case (.dutch, .stopDictation): "Stop dicteren"
        case (.english, .recordAgain): "Record again"
        case (.dutch, .recordAgain): "Opnieuw opnemen"
        case (.english, .checkAgain): "Check again"
        case (.dutch, .checkAgain): "Opnieuw controleren"
        case (.english, .setupDictation): "Set up keyboard dictation"
        case (.dutch, .setupDictation): "Toetsenborddicteren instellen"
        case (.english, .hideHelp): "Hide help"
        case (.dutch, .hideHelp): "Uitleg sluiten"
        case (.english, .accentedLetters): "Accented letters"
        case (.dutch, .accentedLetters): "Letters met accenten"
        case (.english, .dismissAccents): "Close accented letters"
        case (.dutch, .dismissAccents): "Accentletters sluiten"
        case (.english, .showDictation): "Show dictation controls"
        case (.dutch, .showDictation): "Dicteerknoppen tonen"
        case (.english, .showTyping): "Show letter keyboard"
        case (.dutch, .showTyping): "Lettertoetsenbord tonen"
        case (.english, .space): "space"
        case (.dutch, .space): "spatie"
        case (.english, .returnKey): "return"
        case (.dutch, .returnKey): "return"
        case (.english, .go): "go"
        case (.dutch, .go): "ga"
        case (.english, .search): "search"
        case (.dutch, .search): "zoek"
        case (.english, .send): "send"
        case (.dutch, .send): "stuur"
        case (.english, .next): "next"
        case (.dutch, .next): "volgende"
        case (.english, .done): "done"
        case (.dutch, .done): "gereed"
        case (.english, .join): "join"
        case (.dutch, .join): "verbind"
        case (.english, .route): "route"
        case (.dutch, .route): "route"
        case (.english, .continueKey): "continue"
        case (.dutch, .continueKey): "verder"
        case (.english, .shift): "Shift"
        case (.dutch, .shift): "Hoofdletter"
        case (.english, .capsLock): "Caps Lock"
        case (.dutch, .capsLock): "Hoofdletters vast"
        case (.english, .delete): "Delete"
        case (.dutch, .delete): "Verwijder"
        case (_, .numbers): "123"
        case (_, .symbols): "#+="
        case (_, .letters): "ABC"
        case (.english, .showNumbers): "Numbers and punctuation"
        case (.dutch, .showNumbers): "Cijfers en leestekens"
        case (.english, .showSymbols): "Symbols"
        case (.dutch, .showSymbols): "Symbolen"
        case (.english, .showLetters): "Letters"
        case (.dutch, .showLetters): "Letters"
        case (.english, .nextKeyboard): "Next keyboard"
        case (.dutch, .nextKeyboard): "Volgend toetsenbord"
        case (.english, .cancel): "Cancel"
        case (.dutch, .cancel): "Annuleren"
        case (.english, .microphone): "Pennel dictation"
        case (.dutch, .microphone): "Dicteren met Pennel"
        case (.english, .suggestions): "Local suggestions on"
        case (.dutch, .suggestions): "Lokale suggesties aan"
        case (.english, .suggestionsOff): "Local suggestions off"
        case (.dutch, .suggestionsOff): "Lokale suggesties uit"
        case (.english, .suggestionsPrivacy): "Suggestions stay on this device."
        case (.dutch, .suggestionsPrivacy): "Suggesties blijven op dit toestel."
        case (.english, .insertSuggestion): "Insert suggestion"
        case (.dutch, .insertSuggestion): "Voeg suggestie in"
        }
    }
}

public struct ActivityCopy: Sendable {
    public enum Key: CaseIterable, Sendable {
        case sessionReady, listening, processing, sessionOff, stop, sessionStoppedByCall
        case recordingActive, recordingPaused, recordingPausedByCall, finishingRecording, stopRecording
        case dictate, dictateDescription, dictationSession, dictationSessionDescription
        case recordingControl, recordingDescription
    }

    public let language: KeyboardInterfaceLanguage

    public init(language: KeyboardInterfaceLanguage) { self.language = language }

    public static var current: ActivityCopy {
        preferences(in: UserDefaults(suiteName: HandoffStore.appGroup))
    }

    public static func preferences(
        in defaults: UserDefaults?, preferredLanguages: [String] = Locale.preferredLanguages
    ) -> ActivityCopy {
        ActivityCopy(language: KeyboardPreferences.interfaceLanguage(in: defaults, preferredLanguages: preferredLanguages))
    }

    public func text(_ key: Key) -> String {
        switch (language, key) {
        case (.english, .sessionReady): "Pennel is ready"
        case (.dutch, .sessionReady): "Pennel staat klaar"
        case (_, .listening): KeyboardCopy(language: language).text(.listening)
        case (_, .processing): KeyboardCopy(language: language).text(.processing)
        case (_, .sessionOff): KeyboardCopy(language: language).text(.sessionOff)
        case (_, .stop): "Stop"
        case (.english, .sessionStoppedByCall): "Session stopped by a call."
        case (.dutch, .sessionStoppedByCall): "Sessie gestopt door een oproep."
        case (.english, .recordingActive): "Recording in progress"
        case (.dutch, .recordingActive): "Opname loopt"
        case (.english, .recordingPaused): "Recording paused"
        case (.dutch, .recordingPaused): "Opname gepauzeerd"
        case (.english, .recordingPausedByCall): "Recording paused by a call. Pennel resumes after the call ends."
        case (.dutch, .recordingPausedByCall): "Opname gepauzeerd door een oproep. Pennel neemt verder op zodra de oproep voorbij is."
        case (.english, .finishingRecording): "Finishing recording…"
        case (.dutch, .finishingRecording): "Opname afronden…"
        case (.english, .stopRecording): "Stop recording"
        case (.dutch, .stopRecording): "Stop opname"
        case (.english, .dictate): "Dictate"
        case (.dutch, .dictate): "Dicteer"
        case (.english, .dictateDescription): "Dictate to clipboard."
        case (.dutch, .dictateDescription): "Dicteer naar het klembord."
        case (.english, .dictationSession): "Dictation session"
        case (.dutch, .dictationSession): "Dicteersessie"
        case (.english, .dictationSessionDescription): "Prepare the microphone for the Pennel keyboard."
        case (.dutch, .dictationSessionDescription): "Zet de microfoon klaar voor het Pennel-toetsenbord."
        case (.english, .recordingControl): "Recording"
        case (.dutch, .recordingControl): "Opname"
        case (.english, .recordingDescription): "Start a new recording."
        case (.dutch, .recordingDescription): "Start een nieuwe opname."
        }
    }

    public func resource(_ key: Key) -> LocalizedStringResource {
        LocalizedStringResource(String.LocalizationValue(stringLiteral: text(key)), locale: Locale(identifier: language.rawValue))
    }

    public func finishing(pending: UInt32) -> String {
        switch language {
        case .english: pending == 0 ? "Saving recording…" : "Processing \(pending) remaining audio chunks…"
        case .dutch: pending == 0 ? "Opname bewaren…" : "Nog \(pending) audiodelen verwerken…"
        }
    }

    // Older native activity states carry text rather than reason codes. Translate only known system copy.
    public func sessionMessage(_ message: String?) -> String {
        guard let message, !message.isEmpty else { return text(.sessionOff) }
        if ["Sessie gestopt door een oproep.", "Session stopped by a call."].contains(message) { return text(.sessionStoppedByCall) }
        return message
    }

    public func recordingMessage(_ message: String?) -> String {
        guard let message, !message.isEmpty else { return text(.recordingPaused) }
        if [
            "Opname gepauzeerd door een oproep. Pennel neemt verder op zodra de oproep voorbij is.",
            "Recording paused by a call. Pennel resumes after the call ends.",
        ].contains(message) { return text(.recordingPausedByCall) }
        return message
    }
}
