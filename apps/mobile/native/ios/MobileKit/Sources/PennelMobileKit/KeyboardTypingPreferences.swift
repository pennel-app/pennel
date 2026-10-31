import Foundation

public struct KeyboardSnippet: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var title: String
    public var keyword: String
    public var text: String

    public init(id: String = UUID().uuidString, title: String, keyword: String = "", text: String) {
        self.id = id
        self.title = title
        self.keyword = keyword
        self.text = text
    }
}

public struct KeyboardLearnedPair: Codable, Equatable, Sendable {
    public var previous: String
    public var text: String
    public var count: UInt32
    public var language: String
}

public extension KeyboardPreferences {
    static let typingLanguagesKey = "keyboardTypingLanguages"
    static let autocorrectionKey = "keyboardAutocorrectionEnabled"
    static let learningKey = "keyboardLearningEnabled"
    static let swipeKey = "keyboardSwipeEnabled"
    static let learnedWordsKey = "keyboardLearnedWords"
    static let learnedPairsKey = "keyboardLearnedPairs"
    static let snippetsKey = "keyboardSnippets"
    static let snippetExpansionKey = "keyboardSnippetExpansionEnabled"
    static let typingRevisionKey = "keyboardTypingConfigurationRevision"
    static let learningResetKey = "keyboardLearningResetRevision"
    static let forgottenWordsKey = "keyboardForgottenWords"
    static let localSuggestionsOverrideKey = "keyboardLocalSuggestionsOverride"
    static let localSuggestionsOverrideRevisionKey = "keyboardLocalSuggestionsOverrideRevision"
    static let suggestionsRevisionKey = "keyboardSuggestionsRevision"

    static func effectiveSuggestionsEnabled(shared: UserDefaults?, local: UserDefaults?) -> Bool {
        let revision = shared?.string(forKey: suggestionsRevisionKey) ?? ""
        if local?.string(forKey: localSuggestionsOverrideRevisionKey) == revision,
           let overridden = local?.object(forKey: localSuggestionsOverrideKey) as? Bool { return overridden }
        return suggestionsEnabled(in: shared)
    }

    static func setSuggestionsEnabled(_ enabled: Bool, shared: UserDefaults?, local: UserDefaults?, canWriteShared: Bool) {
        if canWriteShared {
            setSuggestionsEnabled(enabled, in: shared)
            local?.removeObject(forKey: localSuggestionsOverrideKey)
            local?.removeObject(forKey: localSuggestionsOverrideRevisionKey)
        } else {
            local?.set(enabled, forKey: localSuggestionsOverrideKey)
            local?.set(shared?.string(forKey: suggestionsRevisionKey) ?? "", forKey: localSuggestionsOverrideRevisionKey)
        }
        local?.set(enabled, forKey: suggestionsKey)
    }

    static func typingLanguages(in defaults: UserDefaults?) -> [String] {
        let stored = defaults?.stringArray(forKey: typingLanguagesKey) ?? ["nl", "en"]
        var seen: Set<String> = []
        let languages = stored.compactMap { value -> String? in
            let language = String(value.prefix(2)).lowercased()
            return ["nl", "en"].contains(language) && seen.insert(language).inserted ? language : nil
        }
        return languages.isEmpty ? ["nl", "en"] : languages
    }

    static func setTypingLanguages(_ languages: [String], in defaults: UserDefaults?) {
        var seen: Set<String> = []
        let clean = languages.map { String($0.prefix(2)).lowercased() }
            .filter { ["en", "nl"].contains($0) && seen.insert($0).inserted }
        defaults?.set(clean.isEmpty ? ["nl", "en"] : clean, forKey: typingLanguagesKey)
        touchTypingRevision(in: defaults)
    }

    static func autocorrectionEnabled(in defaults: UserDefaults?) -> Bool {
        defaults?.object(forKey: autocorrectionKey) as? Bool ?? true
    }

    static func setAutocorrectionEnabled(_ enabled: Bool, in defaults: UserDefaults?) {
        defaults?.set(enabled, forKey: autocorrectionKey)
        touchTypingRevision(in: defaults)
    }

    static func learningEnabled(in defaults: UserDefaults?) -> Bool {
        defaults?.bool(forKey: learningKey) ?? false
    }

    static func setLearningEnabled(_ enabled: Bool, in defaults: UserDefaults?) {
        defaults?.set(enabled, forKey: learningKey)
        touchTypingRevision(in: defaults)
    }

    static func swipeEnabled(in defaults: UserDefaults?) -> Bool {
        defaults?.object(forKey: swipeKey) as? Bool ?? false
    }

    static func setSwipeEnabled(_ enabled: Bool, in defaults: UserDefaults?) {
        defaults?.set(enabled, forKey: swipeKey)
        touchTypingRevision(in: defaults)
    }

    static func snippetExpansionEnabled(in defaults: UserDefaults?) -> Bool {
        defaults?.bool(forKey: snippetExpansionKey) ?? false
    }

    static func setSnippetExpansionEnabled(_ enabled: Bool, in defaults: UserDefaults?) {
        defaults?.set(enabled, forKey: snippetExpansionKey)
        touchTypingRevision(in: defaults)
    }

    static func learnedWords(in defaults: UserDefaults?) -> [String] {
        Array((defaults?.stringArray(forKey: learnedWordsKey) ?? []).filter(isLearnableWord).prefix(512))
    }

    static func learnedPairs(in defaults: UserDefaults?) -> [KeyboardLearnedPair] {
        guard let data = defaults?.data(forKey: learnedPairsKey), data.count <= 256_000,
              let pairs = try? JSONDecoder().decode([KeyboardLearnedPair].self, from: data) else { return [] }
        return Array(pairs.filter { isLearnableWord($0.previous) && isLearnableWord($0.text) && ["nl", "en"].contains($0.language) }.prefix(512))
    }

    static func learn(_ word: String, previous: String?, language: String = "en", in defaults: UserDefaults?) {
        let forgotten = Set(defaults?.stringArray(forKey: forgottenWordsKey) ?? [])
        guard learningEnabled(in: defaults), isLearnableWord(word), !forgotten.contains(word.lowercased()) else { return }
        var words = learnedWords(in: defaults)
        if !words.contains(where: { $0.caseInsensitiveCompare(word) == .orderedSame }) {
            words.append(word)
            defaults?.set(Array(words.suffix(512)), forKey: learnedWordsKey)
        }
        guard let previous, isLearnableWord(previous), !forgotten.contains(previous.lowercased()) else { return }
        var pairs = learnedPairs(in: defaults)
        if let index = pairs.firstIndex(where: { $0.previous == previous && $0.text == word }) {
            pairs[index].count = min(pairs[index].count, 9_999) + 1
        } else {
            pairs.append(KeyboardLearnedPair(previous: previous, text: word, count: 1, language: language == "nl" ? "nl" : "en"))
        }
        if let data = try? JSONEncoder().encode(Array(pairs.suffix(512))) { defaults?.set(data, forKey: learnedPairsKey) }
    }

    static func resetLearning(in defaults: UserDefaults?) {
        defaults?.removeObject(forKey: learnedWordsKey)
        defaults?.removeObject(forKey: learnedPairsKey)
        defaults?.removeObject(forKey: forgottenWordsKey)
        defaults?.set(UUID().uuidString, forKey: learningResetKey)
        touchTypingRevision(in: defaults)
    }

    static func mirrorTypingConfiguration(from shared: UserDefaults?, to local: UserDefaults?) {
        guard let shared, let local, shared !== local else { return }
        for key in [typingLanguagesKey, autocorrectionKey, learningKey, swipeKey, snippetExpansionKey] {
            if let value = shared.object(forKey: key) { local.set(value, forKey: key) }
        }
        if let reset = shared.string(forKey: learningResetKey), local.string(forKey: learningResetKey) != reset {
            local.removeObject(forKey: learnedWordsKey)
            local.removeObject(forKey: learnedPairsKey)
            local.set(reset, forKey: learningResetKey)
        }
        let forgotten = Set((shared.stringArray(forKey: forgottenWordsKey) ?? []).prefix(512))
        local.set(Array(forgotten), forKey: forgottenWordsKey)
        if !forgotten.isEmpty {
            local.set(learnedWords(in: local).filter { !forgotten.contains($0.lowercased()) }, forKey: learnedWordsKey)
            let pairs = learnedPairs(in: local).filter { !forgotten.contains($0.previous.lowercased()) && !forgotten.contains($0.text.lowercased()) }
            if let data = try? JSONEncoder().encode(pairs) { local.set(data, forKey: learnedPairsKey) }
        }
    }

    static func forgetLearnedWord(_ word: String, in defaults: UserDefaults?) {
        guard isLearnableWord(word) else { return }
        let folded = word.lowercased()
        var forgotten = defaults?.stringArray(forKey: forgottenWordsKey) ?? []
        if !forgotten.contains(folded) { forgotten.append(folded) }
        defaults?.set(Array(forgotten.suffix(512)), forKey: forgottenWordsKey)
        defaults?.set(learnedWords(in: defaults).filter { $0.lowercased() != folded }, forKey: learnedWordsKey)
        let pairs = learnedPairs(in: defaults).filter { $0.previous.lowercased() != folded && $0.text.lowercased() != folded }
        if let data = try? JSONEncoder().encode(pairs) { defaults?.set(data, forKey: learnedPairsKey) }
        touchTypingRevision(in: defaults)
    }

    static func mergeExtensionVocabulary(from local: UserDefaults?, to shared: UserDefaults?) {
        guard let local, let shared, local !== shared else { return }
        let forgotten = Set(shared.stringArray(forKey: forgottenWordsKey) ?? [])
        if learningEnabled(in: shared) {
            var words = learnedWords(in: shared)
            for word in learnedWords(in: local) where !forgotten.contains(word.lowercased())
                && !words.contains(where: { $0.caseInsensitiveCompare(word) == .orderedSame }) {
                words.append(word)
            }
            shared.set(Array(words.suffix(512)), forKey: learnedWordsKey)
            var pairs = learnedPairs(in: shared)
            for incoming in learnedPairs(in: local) where !forgotten.contains(incoming.previous.lowercased()) && !forgotten.contains(incoming.text.lowercased()) {
                if let index = pairs.firstIndex(where: { $0.previous == incoming.previous && $0.text == incoming.text }) {
                    pairs[index].count = min(10_000, min(pairs[index].count, 10_000) + min(incoming.count, 10_000))
                } else { pairs.append(incoming) }
            }
            if let data = try? JSONEncoder().encode(Array(pairs.suffix(512))) { shared.set(data, forKey: learnedPairsKey) }
            local.removeObject(forKey: learnedWordsKey)
            local.removeObject(forKey: learnedPairsKey)
        }
        let hidden = Set(local.stringArray(forKey: "keyboardHiddenSnippetIDs") ?? [])
        var stored = snippets(in: shared).filter { !hidden.contains($0.id) }
        for snippet in snippets(in: local) where !hidden.contains(snippet.id) && !stored.contains(where: { $0.id == snippet.id }) {
            stored.append(snippet)
        }
        if !hidden.isEmpty || !snippets(in: local).isEmpty {
            setSnippets(stored, in: shared)
            local.removeObject(forKey: snippetsKey)
            local.removeObject(forKey: "keyboardHiddenSnippetIDs")
        }
    }

    static func snippets(in defaults: UserDefaults?) -> [KeyboardSnippet] {
        let data = defaults?.data(forKey: snippetsKey) ?? defaults?.string(forKey: snippetsKey)?.data(using: .utf8)
        guard let data, data.count <= 512_000,
              let snippets = try? JSONDecoder().decode([KeyboardSnippet].self, from: data) else { return [] }
        var ids: Set<String> = []
        return Array(snippets.filter {
            !$0.id.isEmpty && $0.id.count <= 128 && ids.insert($0.id).inserted
                && !$0.title.isEmpty && $0.title.count <= 80 && !$0.text.isEmpty && $0.text.count <= 4_096
                && $0.keyword.count <= 32
        }.prefix(64))
    }

    static func setSnippets(_ snippets: [KeyboardSnippet], in defaults: UserDefaults?) {
        let clean = Array(snippets.filter { !$0.title.isEmpty && $0.title.count <= 80 && !$0.text.isEmpty && $0.text.count <= 4_096 && $0.keyword.count <= 32 }.prefix(64))
        if let data = try? JSONEncoder().encode(clean) { defaults?.set(data, forKey: snippetsKey) }
        touchTypingRevision(in: defaults)
    }

    static func touchTypingRevision(in defaults: UserDefaults?) {
        defaults?.set(UUID().uuidString, forKey: typingRevisionKey)
    }

    private static func isLearnableWord(_ word: String) -> Bool {
        (2...64).contains(word.count) && word.allSatisfy { $0.isLetter || $0 == "'" || $0 == "’" || $0 == "-" }
    }
}
