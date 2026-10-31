import Foundation
import PennelMobileKit
import PennelTyping
import Testing

@MainActor
private final class TypingTestSpelling: KeyboardSpellingProviding {
    var validTokens: Set<String> = []
    var calls: [(token: String, languages: [String])] = []

    func entries(for token: String, languages: [String]) -> KeyboardSpellingEntries {
        MainActor.assertIsolated()
        calls.append((token, languages))
        return KeyboardSpellingEntries(validInAnyLanguage: validTokens.contains(token))
    }
}

@MainActor
struct KeyboardTypingIntegrationTests {
    private func settled(_ model: KeyboardTypingModel) async -> Bool {
        for _ in 0..<200 {
            if model.predictionsReady { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return false
    }

    @Test func native_candidates_enter_real_rust_policy_without_blocking_insertion() async {
        let spelling = TypingTestSpelling()
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true, spelling: spelling)
        model.update(before: "Please type teh", capitalization: .none)
        #expect(model.suggestions.isEmpty)
        #expect(await settled(model))
        #expect(model.suggestions.contains("the"))
        #expect(model.replacementAtBoundary(before: "Please type teh", after: "")?.text == "the")
        #expect(model.replacementAtBoundary(before: "Please type teh", after: nil)?.text == "the")
        #expect(model.replacementAtBoundary(before: "Please type teh", after: "word") == nil)
    }

    @Test func valid_bilingual_words_and_host_traits_prevent_automatic_changes() async {
        let spelling = TypingTestSpelling()
        spelling.validTokens = ["teh"]
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .dutch), suggestionsEnabled: true, spelling: spelling)
        model.update(before: "teh", capitalization: .none)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "teh", after: "") == nil)
        spelling.validTokens = []
        model.update(before: "mischien", capitalization: .none, autocorrectionAllowed: false)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "mischien", after: "") == nil)
    }

    @Test func stale_and_cancelled_predictions_cannot_replace_the_new_token() async {
        let spelling = TypingTestSpelling()
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true, spelling: spelling)
        model.update(before: "teh", capitalization: .none)
        model.update(before: "mischien", capitalization: .none)
        #expect(await settled(model))
        #expect(model.currentPrefix == "mischien")
        #expect(model.replacementAtBoundary(before: "teh", after: "") == nil)
        #expect(model.replacementAtBoundary(before: "mischien", after: "")?.text == "misschien")
        #expect(!spelling.calls.contains(where: { $0.token == "teh" }))
        model.update(before: "teh", capitalization: .none)
        model.cancelPrediction()
        for _ in 0..<5 { await Task.yield() }
        #expect(!model.predictionsReady)
        #expect(model.replacementAtBoundary(before: "teh", after: "") == nil)
    }

    @Test func undo_rejection_survives_a_delimiter_and_does_not_reapply() async {
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true, spelling: TypingTestSpelling())
        model.reject(original: "teh", replacement: "the")
        model.update(before: "teh ", capitalization: .none)
        #expect(await settled(model))
        model.update(before: "teh", capitalization: .none)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "teh", after: "") == nil)
        #expect(!model.suggestions.contains("the"))
    }

    @Test func system_shortcuts_and_multiword_vocabulary_are_explicit_suggestions() async {
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true, spelling: TypingTestSpelling())
        model.setLexicon([KeyboardLexiconEntry(input: "omw", replacement: "On my way")], words: ["Pennel Cloud"])
        model.update(before: "omw", capitalization: .none)
        #expect(await settled(model))
        #expect(model.suggestions.contains("On my way"))
        #expect(model.replacementAtBoundary(before: "omw", after: "") == nil)
        model.update(before: "Pen", capitalization: .none)
        #expect(await settled(model))
        #expect(model.suggestions.contains("Pennel Cloud"))
    }

    @Test func input_language_remains_independent_from_interface_language_and_typing_is_not_learned() async {
        let name = "pennel-native-typing-\(UUID())"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        KeyboardPreferences.setTypingLanguages(["nl"], in: defaults)
        let spelling = TypingTestSpelling()
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true,
                                        defaults: defaults, learningDefaults: defaults, spelling: spelling)
        model.update(before: "privé", capitalization: .none)
        #expect(await settled(model))
        #expect(model.copy.language == .english)
        #expect(spelling.calls.last?.languages == ["nl"])
        #expect(KeyboardPreferences.learnedWords(in: defaults).isEmpty)
        #expect(defaults.object(forKey: KeyboardPreferences.dictionaryKey) == nil)
    }

    @Test func punctuation_stays_at_the_cursor_and_cannot_be_deleted_as_a_word_suffix() async {
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true, spelling: TypingTestSpelling())
        for text in ["(teh)", "“teh”", "[teh]"] {
            model.update(before: text, capitalization: .none)
            #expect(await settled(model))
            #expect(model.suggestions.isEmpty)
            #expect(model.replacementAtBoundary(before: text, after: "") == nil)
        }
        model.update(before: "(teh", capitalization: .none)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "(teh", after: "")?.text == "the")
    }

    @Test func opted_in_semicolon_snippet_expands_without_autocorrection_or_suggestions() async {
        let name = "pennel-native-snippet-\(UUID())"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        KeyboardPreferences.setAutocorrectionEnabled(false, in: defaults)
        KeyboardPreferences.setSnippetExpansionEnabled(true, in: defaults)
        KeyboardPreferences.setSnippets([KeyboardSnippet(title: "Signature", keyword: ";sig", text: "Kind regards, Lander")], in: defaults)
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: false,
            defaults: defaults, spelling: TypingTestSpelling())
        model.update(before: ";sig", capitalization: .none, autocorrectionAllowed: false)
        #expect(await settled(model))
        #expect(model.currentPrefix == ";sig")
        #expect(model.suggestions.isEmpty)
        #expect(model.replacementAtBoundary(before: ";sig", after: "")?.text == "Kind regards, Lander")
        model.update(before: ";sig", capitalization: .none, assistanceAllowed: false, autocorrectionAllowed: false)
        #expect(model.replacementAtBoundary(before: ";sig", after: "") == nil)
    }

    @Test func missing_after_context_allows_only_current_known_prefix_corrections() async {
        let spelling = TypingTestSpelling()
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true, spelling: spelling)
        model.update(before: "Teh teh", capitalization: .none)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "Teh teh", after: nil)?.text == "the")
        #expect(model.replacementAtBoundary(before: "Other teh", after: nil) == nil)
        #expect(model.replacementAtBoundary(before: nil, after: nil) == nil)
        #expect(model.replacementAtBoundary(before: "Teh teh", after: "nical") == nil)
        model.update(before: "Teh Teh", capitalization: .none)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "Teh Teh", after: nil) == nil)
        spelling.validTokens = ["teh"]
        model.update(before: "Teh teh", capitalization: .none)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "Teh teh", after: nil) == nil)
        spelling.validTokens = []
        model.update(before: "Teh teh", capitalization: .none, autocorrectionAllowed: false)
        #expect(await settled(model))
        #expect(model.replacementAtBoundary(before: "Teh teh", after: nil) == nil)
    }

    @Test func interrupted_edit_original_is_explicitly_recoverable_without_persistent_storage() throws {
        let name = "pennel-interrupted-edit-\(UUID())"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        let model = KeyboardTypingModel(copy: KeyboardCopy(language: .english), suggestionsEnabled: true,
            defaults: defaults, learningDefaults: defaults, spelling: TypingTestSpelling())
        model.retainInterruptedEdit("teh ")
        let recovered = try #require(model.snippets.first)
        #expect(recovered.text == "teh ")
        #expect(recovered.keyword.isEmpty)
        #expect(KeyboardPreferences.snippets(in: defaults).isEmpty)
        #expect(defaults.object(forKey: KeyboardPreferences.snippetsKey) == nil)
        model.reloadPreferences()
        #expect(model.snippets.first?.id == recovered.id)
        model.deleteSnippet(recovered)
        #expect(model.snippets.isEmpty)
        #expect(defaults.object(forKey: KeyboardPreferences.snippetsKey) == nil)
    }
}
