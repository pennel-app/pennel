import Foundation
import Observation
import PennelMobileKit
import PennelTyping
import UIKit

private actor KeyboardScoringWorker {
    static let shared = KeyboardScoringWorker()

    func suggestions(_ request: TypingRequest) -> TypingResponse? {
        guard !Task.isCancelled else { return nil }
        return TypingEngine.suggestions(for: request)
    }

    func swipe(_ request: SwipeRequest) -> TypingResponse? {
        guard !Task.isCancelled else { return nil }
        return TypingEngine.swipe(request)
    }
}

@MainActor
protocol KeyboardSpellingProviding {
    func entries(for token: String, languages: [String]) -> KeyboardSpellingEntries
}

struct KeyboardSpellingEntries {
    var candidates: [TypingPlatformCandidate] = []
    var validInAnyLanguage = false
}

@MainActor
final class KeyboardSystemSpelling: KeyboardSpellingProviding {
    private let checker = UITextChecker()
    private let availableLanguages = UITextChecker.availableLanguages
    private var cache: [String: KeyboardSpellingEntries] = [:]
    private var order: [String] = []

    func entries(for token: String, languages: [String]) -> KeyboardSpellingEntries {
        guard !token.isEmpty, token.count <= 64, token.allSatisfy({ $0.isLetter || $0 == "'" || $0 == "’" }) else { return KeyboardSpellingEntries() }
        let key = languages.joined(separator: ",") + ":" + token
        if let cached = cache[key] { return cached }
        var result = KeyboardSpellingEntries()
        let range = NSRange(location: 0, length: (token as NSString).length)
        for language in languages {
            let preferred = language == "nl" ? "nl_NL" : "en_US"
            let identifier = availableLanguages.first { $0.replacingOccurrences(of: "-", with: "_") == preferred }
                ?? availableLanguages.sorted().first { $0.lowercased().hasPrefix(language) }
            guard let identifier else { continue }
            let misspelling = checker.rangeOfMisspelledWord(in: token, range: range, startingAt: 0, wrap: false, language: identifier)
            if misspelling.location == NSNotFound { result.validInAnyLanguage = true }
            for word in (checker.guesses(forWordRange: range, in: token, language: identifier) ?? []).prefix(12) where word.utf8.count <= 128 {
                result.candidates.append(TypingPlatformCandidate(text: word, language: language, kind: .correction))
            }
            for word in (checker.completions(forPartialWordRange: range, in: token, language: identifier) ?? []).prefix(12) where word.utf8.count <= 128 {
                result.candidates.append(TypingPlatformCandidate(text: word, language: language, kind: .completion))
            }
        }
        cache[key] = result
        order.append(key)
        if order.count > 128 { cache.removeValue(forKey: order.removeFirst()) }
        return result
    }
}

@MainActor
@Observable
final class KeyboardTypingModel {
    var state = KeyboardTypingState()
    var suggestions: [String] = []
    var suggestionsEnabled: Bool
    var returnKey: KeyboardCopy.Key = .returnKey
    var returnEnabled = true
    var allowsPeriod = true
    var showsAt = false
    var showsURL = false
    var selectedText: String?
    var correctionOriginal: String?
    var writingPreview: String?
    var writingOriginal: String?
    var writingStatus: String?
    var writingIsProcessing = false
    var writingCanApply = false
    var writingCanRestore = false
    var writingAvailable = false
    var snippets: [KeyboardSnippet] = []
    var swipeEnabled = false
    private(set) var typingLanguages: [String] = ["nl", "en"]
    private(set) var autocorrectionEnabled = true
    private(set) var currentPrefix = ""
    private(set) var candidateRevision: UInt64 = 0
    let copy: KeyboardCopy
    var predictionsReady: Bool { response != nil && responseToken == currentPrefix && responseRevision == candidateRevision }

    @ObservationIgnored private let spelling: any KeyboardSpellingProviding
    @ObservationIgnored private var defaults: UserDefaults?
    @ObservationIgnored private var learningDefaults: UserDefaults?
    @ObservationIgnored private var writableDefaults: UserDefaults?
    @ObservationIgnored private var predictionTask: Task<Void, Never>?
    @ObservationIgnored private var lexicon: [KeyboardLexiconEntry] = []
    @ObservationIgnored private var personalWords: [String] = []
    @ObservationIgnored private var response: TypingResponse?
    @ObservationIgnored private var responseToken = ""
    @ObservationIgnored private var currentBeforeContext: String?
    @ObservationIgnored private var responseBeforeContext: String?
    @ObservationIgnored private var responseRevision: UInt64?
    #if PENNEL_KEYBOARD_FEASIBILITY
    @ObservationIgnored private var probeWordValidity: Bool?
    #endif
    @ObservationIgnored private var previousToken: String?
    @ObservationIgnored private var rejectedToken: String?
    @ObservationIgnored private var rejectedCandidates: [String] = []
    @ObservationIgnored private var selectionIsEmpty = true
    @ObservationIgnored private var permitsAssistance = true
    @ObservationIgnored private var permitsAutocorrection = true
    @ObservationIgnored private var wordTouches: [TypingPoint] = []
    @ObservationIgnored private var wordKeys: [TypingKey] = []
    @ObservationIgnored private var interruptedEdit: KeyboardSnippet?

    init(copy: KeyboardCopy, suggestionsEnabled: Bool, defaults: UserDefaults? = nil,
         learningDefaults: UserDefaults? = nil, writableDefaults: UserDefaults? = nil,
         spelling: (any KeyboardSpellingProviding)? = nil) {
        self.copy = copy
        self.suggestionsEnabled = suggestionsEnabled
        self.defaults = defaults
        self.learningDefaults = learningDefaults ?? defaults
        self.writableDefaults = writableDefaults ?? defaults
        self.spelling = spelling ?? KeyboardSystemSpelling()
        reloadPreferences()
    }

    func reloadPreferences() {
        typingLanguages = KeyboardPreferences.typingLanguages(in: defaults)
        autocorrectionEnabled = KeyboardPreferences.autocorrectionEnabled(in: defaults)
        swipeEnabled = KeyboardPreferences.swipeEnabled(in: defaults)
        snippets = KeyboardPreferences.snippets(in: defaults)
        if defaults !== learningDefaults {
            let local = KeyboardPreferences.snippets(in: learningDefaults)
            for snippet in local where !snippets.contains(where: { $0.id == snippet.id }) { snippets.append(snippet) }
        }
        snippets = Array(snippets.prefix(64))
        let hidden = Set(learningDefaults?.stringArray(forKey: "keyboardHiddenSnippetIDs") ?? [])
        snippets.removeAll { hidden.contains($0.id) }
        if let interruptedEdit { snippets.insert(interruptedEdit, at: 0) }
    }

    func setLexicon(_ entries: [KeyboardLexiconEntry], words: [String]) {
        lexicon = Array(entries.prefix(2_048))
        personalWords = Array(words.prefix(1_024))
        schedulePrediction()
    }

    func update(before: String?, capitalization: KeyboardCapitalization,
                assistanceAllowed: Bool = true, autocorrectionAllowed: Bool = true, selectionIsEmpty: Bool = true) {
        state.updateCapitalization(before: before, mode: capitalization)
        currentPrefix = KeyboardTypingContext.token(before: before)
        currentBeforeContext = before.map { String($0.suffix(128)) }
        previousToken = KeyboardTypingContext.previousToken(before: before).flatMap { Self.isEngineWord($0) ? $0 : nil }
        permitsAssistance = assistanceAllowed && before?.hasSuffix(currentPrefix) == true
        permitsAutocorrection = autocorrectionAllowed
        self.selectionIsEmpty = selectionIsEmpty
        if !currentPrefix.isEmpty, currentPrefix != rejectedToken { rejectedToken = nil; rejectedCandidates = [] }
        schedulePrediction()
    }

    func cancelPrediction() {
        candidateRevision &+= 1
        predictionTask?.cancel()
        predictionTask = nil
        response = nil
        responseToken = ""
        responseBeforeContext = nil
        responseRevision = nil
        #if PENNEL_KEYBOARD_FEASIBILITY
        probeWordValidity = nil
        #endif
    }

    func recordTouch(_ gesture: KeyboardSwipeGesture) {
        guard gesture.points.count == 1, let point = gesture.points.first,
              gesture.keys.count <= 64, wordTouches.count < 64 else { return }
        wordTouches.append(TypingPoint(x: point.x, y: point.y))
        wordKeys = gesture.keys.map { TypingKey(key: $0.key, x: $0.x, y: $0.y) }
    }

    func resetTouches() {
        wordTouches = []
        wordKeys = []
    }

    func replacementAtBoundary(before: String?, after: String?) -> (text: String, original: String)? {
        let token = KeyboardTypingContext.token(before: before)
        guard permitsAssistance, selectionIsEmpty, predictionsReady,
              token == currentPrefix, token == responseToken,
              before?.hasSuffix(token) == true,
              before.map({ String($0.suffix(128)) }) == responseBeforeContext,
              after?.first.map({ !$0.isLetter && !$0.isNumber }) ?? true,
              let text = response?.automaticReplacement, text != token else { return nil }
        switch response?.replacementKind {
        case .snippet:
            guard KeyboardPreferences.snippetExpansionEnabled(in: defaults) else { return nil }
        case .correction:
            guard autocorrectionEnabled, permitsAutocorrection else { return nil }
        default: return nil
        }
        return (text, token)
    }

    #if PENNEL_KEYBOARD_FEASIBILITY
    func boundaryProbe(before: String?, after: String?) -> [String: Any] {
        ["cacheReady": predictionsReady,
         "replacementAvailable": response?.automaticReplacement != nil,
         "wordValidityKnown": probeWordValidity != nil,
         "wordValid": probeWordValidity ?? false,
         "hostAllowsAutocorrection": permitsAutocorrection,
         "assistanceAllowed": permitsAssistance,
         "autocorrectionEnabled": autocorrectionEnabled,
         "selectionIsEmpty": selectionIsEmpty,
         "tokenMatchesResponse": currentPrefix == responseToken,
         "tokenIsBeforeSuffix": before?.hasSuffix(currentPrefix) == true,
         "currentTokenLength": currentPrefix.count,
         "candidateCount": response?.candidates.count ?? 0,
         "requestFailed": response?.error != nil,
         "beforeIsNil": before == nil,
         "afterIsNil": after == nil,
         "beforeLength": before?.count ?? 0,
         "afterLength": after?.count ?? 0]
    }
    #endif

    func reject(original: String, replacement: String) {
        rejectedToken = original
        rejectedCandidates = [replacement]
        response = nil
        responseToken = ""
        correctionOriginal = nil
    }

    func learnAccepted(_ word: String, previous: String?) {
        KeyboardPreferences.learn(word, previous: previous, language: typingLanguages.first ?? "en", in: learningDefaults)
    }

    func decodeSwipe(_ gesture: KeyboardSwipeGesture) async -> [String] {
        guard swipeEnabled, permitsAssistance, selectionIsEmpty, gesture.points.count >= 3,
              !gesture.keys.isEmpty else { return [] }
        let request = SwipeRequest(
            path: gesture.points.map { TypingPoint(x: $0.x, y: $0.y) },
            keys: gesture.keys.map { TypingKey(key: $0.key, x: $0.x, y: $0.y) },
            enabledLanguages: typingLanguages, vocabulary: vocabulary(), personalWords: allPersonalWords())
        return await KeyboardScoringWorker.shared.swipe(request)?.candidates.map(\.text) ?? []
    }

    func saveSnippet(title: String, text: String) {
        guard !title.isEmpty, !text.isEmpty, title.count <= 80, text.count <= 4_096 else { return }
        var stored = KeyboardPreferences.snippets(in: writableDefaults)
        stored.append(KeyboardSnippet(title: title, text: text))
        KeyboardPreferences.setSnippets(stored, in: writableDefaults)
        reloadPreferences()
    }

    func deleteSnippet(_ snippet: KeyboardSnippet) {
        if snippet.id == interruptedEdit?.id {
            interruptedEdit = nil
            reloadPreferences()
            return
        }
        let stored = KeyboardPreferences.snippets(in: writableDefaults).filter { $0.id != snippet.id }
        KeyboardPreferences.setSnippets(stored, in: writableDefaults)
        if defaults !== writableDefaults {
            var hidden = learningDefaults?.stringArray(forKey: "keyboardHiddenSnippetIDs") ?? []
            if !hidden.contains(snippet.id) { hidden.append(snippet.id) }
            learningDefaults?.set(Array(hidden.suffix(64)), forKey: "keyboardHiddenSnippetIDs")
        }
        reloadPreferences()
    }

    func retainInterruptedEdit(_ text: String) {
        guard !text.isEmpty, text.count <= 4_098 else { return }
        interruptedEdit = KeyboardSnippet(title: copy.language == .dutch ? "Onderbroken bewerking (tijdelijk)" : "Interrupted edit (temporary)", text: text)
        reloadPreferences()
    }

    private func schedulePrediction() {
        cancelPrediction()
        guard permitsAssistance, selectionIsEmpty,
              suggestionsEnabled || (autocorrectionEnabled && permitsAutocorrection)
                || KeyboardPreferences.snippetExpansionEnabled(in: defaults)
        else { suggestions = []; response = nil; responseToken = ""; return }
        let token = currentPrefix
        let revision = candidateRevision
        let previous = previousToken
        let requestedBefore = currentBeforeContext
        response = nil
        responseToken = ""
        suggestions = []
        #if PENNEL_KEYBOARD_FEASIBILITY
        probeWordValidity = nil
        #endif
        predictionTask = Task { @MainActor [weak self] in
            await Task.yield()
            guard let self, !Task.isCancelled, self.candidateRevision == revision else { return }
            let spelling = self.spelling.entries(for: token, languages: self.typingLanguages)
            var candidates = spelling.candidates
            for entry in self.lexicon where entry.input.lowercased().hasPrefix(token.lowercased())
                && !token.isEmpty && Self.isEngineWord(entry.replacement) && entry.replacement.lowercased().hasPrefix(token.lowercased()) {
                candidates.append(TypingPlatformCandidate(text: entry.replacement, language: self.typingLanguages.first ?? "en", kind: .completion))
                if candidates.count >= 64 { break }
            }
            let pairs = KeyboardPreferences.learnedPairs(in: self.learningDefaults).filter {
                Self.isEngineWord($0.previous) && Self.isEngineWord($0.text)
            }.map {
                TypingNextWord(previous: $0.previous, text: $0.text, count: $0.count, language: $0.language)
            }
            var snippets = self.snippets.filter {
                Self.isEngineWord($0.keyword) && $0.text.utf8.count <= 2_048
            }.map { TypingSnippet(trigger: $0.keyword, replacement: $0.text) }
            var hasPrefixExpansion = false
            if Self.isEngineWord(token) {
                for entry in self.lexicon where entry.input.lowercased().hasPrefix(token.lowercased())
                    && entry.input != entry.replacement && !entry.replacement.isEmpty && entry.replacement.utf8.count <= 2_048 {
                    snippets.append(TypingSnippet(trigger: token, replacement: entry.replacement))
                    if entry.input != token { hasPrefixExpansion = true }
                    if snippets.count >= 96 { break }
                }
                for phrase in self.personalWords where phrase.contains(where: \.isWhitespace)
                    && phrase.lowercased().hasPrefix(token.lowercased()) && phrase.utf8.count <= 2_048 {
                    snippets.append(TypingSnippet(trigger: token, replacement: phrase))
                    hasPrefixExpansion = true
                    if snippets.count >= 128 { break }
                }
            }
            let request = TypingRequest(
                token: token, enabledLanguages: self.typingLanguages, platformCandidates: Array(candidates.prefix(64)),
                validInAnyLanguage: spelling.validInAnyLanguage, personalWords: self.allPersonalWords(),
                vocabulary: self.vocabulary(), atWordBoundary: true,
                allowAutomaticCorrection: self.autocorrectionEnabled && self.permitsAutocorrection,
                rejectedCandidates: self.rejectedCandidates, previousToken: previous, learnedNextWords: pairs,
                snippets: Array(snippets.prefix(128)),
                allowSnippetExpansion: !hasPrefixExpansion && KeyboardPreferences.snippetExpansionEnabled(in: self.defaults),
                touches: self.wordTouches, keys: self.wordKeys)
            guard let result = await KeyboardScoringWorker.shared.suggestions(request) else { return }
            guard !Task.isCancelled, self.candidateRevision == revision, self.currentPrefix == token else { return }
            self.response = result
            self.responseToken = token
            self.responseBeforeContext = requestedBefore
            self.responseRevision = revision
            #if PENNEL_KEYBOARD_FEASIBILITY
            self.probeWordValidity = spelling.validInAnyLanguage
            #endif
            if self.suggestionsEnabled { self.suggestions = result.candidates.map(\.text) }
        }
    }

    private func allPersonalWords() -> [String] {
        Array((personalWords.flatMap { $0.split(whereSeparator: \.isWhitespace).map(String.init) }
               + KeyboardPreferences.learnedWords(in: learningDefaults)).filter(Self.isEngineWord).prefix(1_536))
    }

    private func vocabulary() -> [TypingWord] {
        Array(personalWords.filter(Self.isEngineWord).prefix(1_024)).map { TypingWord(text: $0, language: typingLanguages.first ?? "en", frequency: 8) }
    }

    private static func isEngineWord(_ word: String) -> Bool {
        !word.isEmpty && word.count <= 64 && word.utf8.count <= 128
            && !word.contains(where: { $0.isWhitespace || $0.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) })
    }
}
