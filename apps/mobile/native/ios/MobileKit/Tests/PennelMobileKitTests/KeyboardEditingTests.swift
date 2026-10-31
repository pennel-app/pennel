import Foundation
import Testing
@testable import PennelMobileKit

struct KeyboardEditingTests {
    @Test func correction_undo_requires_the_same_document_cursor_and_text() {
        let document = UUID()
        let lifetime = UUID()
        let edit = KeyboardCorrectionTransaction(
            original: "teh", replacement: "the", delimiter: " ", before: "Type teh", after: " next",
            document: document, lifetime: lifetime)
        #expect(edit?.original == "teh")
        #expect(edit?.permits(before: "Type the ", after: " next", document: document, lifetime: lifetime) == true)
        #expect(edit?.permits(before: "Type the ", after: " changed", document: document, lifetime: lifetime) == false)
        #expect(edit?.permits(before: "Other the ", after: " next", document: document, lifetime: lifetime) == false)
        #expect(edit?.permits(before: "Type the x", after: " next", document: document, lifetime: lifetime) == false)
        #expect(edit?.permits(before: "Type the ", after: " next", document: UUID(), lifetime: lifetime) == false)
        #expect(edit?.permits(before: "Type the ", after: " next", document: document, lifetime: UUID()) == false)
    }

    @Test func correction_transactions_count_graphemes_and_reject_missing_context() {
        let document = UUID()
        let lifetime = UUID()
        let edit = KeyboardCorrectionTransaction(
            original: "cooperatie", replacement: "coöperatie", delimiter: ".", before: "Een cooperatie", after: "",
            document: document, lifetime: lifetime)
        #expect(edit?.replacementCharacterCount == "coöperatie.".count)
        #expect(edit?.restoredText == "cooperatie.")
        #expect(KeyboardCorrectionTransaction(original: "teh", replacement: "the", delimiter: " ", before: nil,
                                              after: "", document: document, lifetime: lifetime) == nil)
        #expect(KeyboardCorrectionTransaction(original: "teh", replacement: "the", delimiter: " ", before: "other",
                                              after: "", document: document, lifetime: lifetime) == nil)
    }

    @Test func token_context_preserves_addresses_and_handles_dutch_apostrophes() {
        #expect(KeyboardTypingContext.token(before: "Ik zeg 's avonds") == "avonds")
        #expect(KeyboardTypingContext.token(before: "Please email user@example.cmo") == "user@example.cmo")
        #expect(KeyboardTypingContext.token(before: "Een coöperatie") == "coöperatie")
        #expect(KeyboardTypingContext.token(before: "(teh") == "teh")
        #expect(KeyboardTypingContext.token(before: "(teh)") == "teh)")
        #expect(KeyboardTypingContext.token(before: "“teh”") == "teh”")
        #expect(KeyboardTypingContext.token(before: ";sig") == ";sig")
        #expect(KeyboardTypingContext.token(before: "Finished. ").isEmpty)
        #expect(KeyboardTypingContext.previousToken(before: "Ik heb ") == "heb")
        #expect(KeyboardTypingContext.previousToken(before: "Ik he") == "Ik")
        #expect(KeyboardTypingContext.wordDeletionCount(before: "goed 👩🏽‍💻 ") == 2)
        #expect(KeyboardTypingContext.wordDeletionCount(before: "goed coöperatie") == "coöperatie".count)
    }

    @Test func writing_replacement_requires_the_original_selection_and_exact_restore() {
        let document = UUID()
        let lifetime = UUID()
        let snapshot = KeyboardSelectionSnapshot(text: "Hello there", before: "Prefix ", after: " suffix",
                                                 document: document, lifetime: lifetime)
        #expect(snapshot.permits(selectedText: "Hello there", before: "Prefix ", after: " suffix", document: document, lifetime: lifetime))
        #expect(!snapshot.permits(selectedText: "Other", before: "Prefix ", after: " suffix", document: document, lifetime: lifetime))
        #expect(!snapshot.permits(selectedText: "Hello there", before: "Prefix ", after: " suffix", document: UUID(), lifetime: lifetime))
        let restore = snapshot.restoration(replacement: "Hello")
        #expect(restore.permits(before: "Prefix Hello", after: " suffix", selectedText: nil, document: document, lifetime: lifetime))
        #expect(!restore.permits(before: "Prefix Hello!", after: " suffix", selectedText: nil, document: document, lifetime: lifetime))
    }

    @Test func growing_edits_remain_reversible_with_a_bounded_host_context() {
        let document = UUID()
        let lifetime = UUID()
        let before = String(repeating: "x", count: 43) + "mischien"
        let edit = KeyboardCorrectionTransaction(original: "mischien", replacement: "misschien", delimiter: " ",
            before: before, after: "", document: document, lifetime: lifetime)
        let visible = String((String(repeating: "x", count: 43) + "misschien ").suffix(50))
        #expect(edit?.permits(before: visible, after: "", document: document, lifetime: lifetime) == true)
        #expect(edit?.permits(before: "other misschien ", after: "", document: document, lifetime: lifetime) == false)
        let snapshot = KeyboardSelectionSnapshot(text: "old", before: String(repeating: "x", count: 50), after: "",
            document: document, lifetime: lifetime)
        let restoration = snapshot.restoration(replacement: "replacement")
        #expect(restoration.permits(before: String((String(repeating: "x", count: 50) + "replacement").suffix(50)),
            after: "", selectedText: nil, document: document, lifetime: lifetime))
        let tooLong = snapshot.restoration(replacement: String(repeating: "y", count: 51))
        #expect(!tooLong.permits(before: String(repeating: "y", count: 50), after: "", selectedText: nil,
            document: document, lifetime: lifetime))
        #expect(KeyboardCorrectionTransaction(original: "teh", replacement: "the", delimiter: " ", before: "(teh)",
            after: "", document: document, lifetime: lifetime) == nil)
    }

    @Test func explicit_completion_accepts_missing_after_context_without_changing_the_suffix_guard() {
        #expect(KeyboardTypingContext.permitsManualReplacement(token: "teh", before: "Type teh", after: nil))
        #expect(!KeyboardTypingContext.permitsManualReplacement(token: "teh", before: "(teh)", after: nil))
        #expect(!KeyboardTypingContext.permitsManualReplacement(token: "teh", before: nil, after: nil))
        #expect(!KeyboardTypingContext.permitsManualReplacement(token: "teh", before: "Type teh", after: "word"))
        let document = UUID()
        let lifetime = UUID()
        let correction = KeyboardCorrectionTransaction(original: "teh", replacement: "the", delimiter: " ", before: "Type teh",
            after: nil, document: document, lifetime: lifetime)
        #expect(correction?.permits(before: "Type the ", after: nil, document: document, lifetime: lifetime) == true)
        #expect(correction?.permits(before: "Type the ", after: "", document: document, lifetime: lifetime) == false)
    }

    @Test func missing_following_context_keeps_prefix_proof_and_generation_required_for_undo() {
        let document = UUID()
        let lifetime = UUID()
        let original = KeyboardCorrectionTransaction(original: "teh", replacement: "the", delimiter: " ",
            before: "Teh teh", after: nil, document: document, lifetime: lifetime)
        let bound = original?.bound(to: KeyboardInsertionContext(document: document, lifetime: lifetime, generation: 12))
        #expect(bound?.permits(before: "Teh the ", after: nil, document: document, lifetime: lifetime, generation: 12) == true)
        #expect(bound?.permits(before: "Teh the ", after: nil, document: document, lifetime: lifetime, generation: 13) == false)
        #expect(bound?.permits(before: "Teh the ", after: nil, document: document, lifetime: lifetime) == false)
        #expect(bound?.permits(before: "Other the ", after: nil, document: document, lifetime: lifetime, generation: 12) == false)
        #expect(bound?.permits(before: "Teh the ", after: " changed", document: document, lifetime: lifetime, generation: 12) == false)
        #expect(original?.bound(to: KeyboardInsertionContext(document: UUID(), lifetime: lifetime, generation: 12)) == nil)
        #expect(KeyboardCorrectionTransaction(original: "teh", replacement: "the", delimiter: " ",
            before: nil, after: nil, document: document, lifetime: lifetime) == nil)
    }
}

struct KeyboardTypingConfigurationTests {
    @Test func bilingual_languages_are_independent_from_interface_language() {
        let name = "pennel-typing-config-\(UUID())"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        KeyboardPreferences.publish(dictionary: [], interfaceLanguage: "en", in: defaults)
        #expect(KeyboardPreferences.typingLanguages(in: defaults) == ["nl", "en"])
        KeyboardPreferences.setTypingLanguages(["fr", "EN-us", "nl-BE", "en"], in: defaults)
        #expect(KeyboardPreferences.typingLanguages(in: defaults) == ["en", "nl"])
        #expect(KeyboardPreferences.interfaceLanguage(in: defaults) == .english)
        #expect(!KeyboardPreferences.learningEnabled(in: defaults))
    }

    @Test func learning_is_explicit_bounded_and_resettable() {
        let name = "pennel-typing-learning-\(UUID())"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        KeyboardPreferences.learn("Pennel", previous: "Use", in: defaults)
        #expect(KeyboardPreferences.learnedWords(in: defaults).isEmpty)
        KeyboardPreferences.setLearningEnabled(true, in: defaults)
        KeyboardPreferences.learn("Pennel", previous: "Use", in: defaults)
        KeyboardPreferences.learn("Pennel", previous: "Use", in: defaults)
        KeyboardPreferences.learn("user@example.com", previous: "email", in: defaults)
        #expect(KeyboardPreferences.learnedWords(in: defaults) == ["Pennel"])
        #expect(KeyboardPreferences.learnedPairs(in: defaults).first?.count == 2)
        KeyboardPreferences.resetLearning(in: defaults)
        #expect(KeyboardPreferences.learnedWords(in: defaults).isEmpty)
        #expect(KeyboardPreferences.learnedPairs(in: defaults).isEmpty)
    }

    @Test func snippets_are_bounded_and_expansion_is_separately_opt_in() {
        let name = "pennel-typing-snippets-\(UUID())"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        let snippets = [KeyboardSnippet(title: "Greeting", keyword: "greet", text: "Hallo, hello!"),
                        KeyboardSnippet(title: "Empty", text: "")]
        KeyboardPreferences.setSnippets(snippets, in: defaults)
        #expect(KeyboardPreferences.snippets(in: defaults).map(\.title) == ["Greeting"])
        #expect(!KeyboardPreferences.snippetExpansionEnabled(in: defaults))
    }

    @Test func local_suggestion_override_cannot_resurrect_after_a_shared_choice() {
        let sharedName = "pennel-suggestion-shared-\(UUID())"
        let localName = "pennel-suggestion-local-\(UUID())"
        let shared = UserDefaults(suiteName: sharedName)!
        let local = UserDefaults(suiteName: localName)!
        defer { shared.removePersistentDomain(forName: sharedName); local.removePersistentDomain(forName: localName) }
        KeyboardPreferences.setSuggestionsEnabled(true, in: shared)
        KeyboardPreferences.setSuggestionsEnabled(false, shared: shared, local: local, canWriteShared: false)
        #expect(!KeyboardPreferences.effectiveSuggestionsEnabled(shared: shared, local: local))
        KeyboardPreferences.setSwipeEnabled(true, in: shared)
        #expect(!KeyboardPreferences.effectiveSuggestionsEnabled(shared: shared, local: local))
        KeyboardPreferences.setSuggestionsEnabled(true, shared: shared, local: local, canWriteShared: true)
        #expect(KeyboardPreferences.effectiveSuggestionsEnabled(shared: shared, local: local))
        #expect(local.object(forKey: KeyboardPreferences.localSuggestionsOverrideKey) == nil)
        KeyboardPreferences.setSuggestionsEnabled(false, shared: shared, local: local, canWriteShared: false)
        KeyboardPreferences.setSuggestionsEnabled(false, in: shared)
        KeyboardPreferences.setSuggestionsEnabled(true, in: shared)
        #expect(KeyboardPreferences.effectiveSuggestionsEnabled(shared: shared, local: local))
    }

    @Test func vocabulary_migration_respects_forgetting_and_reset_without_losing_other_terms() {
        let sharedName = "pennel-learning-shared-\(UUID())"
        let localName = "pennel-learning-local-\(UUID())"
        let shared = UserDefaults(suiteName: sharedName)!
        let local = UserDefaults(suiteName: localName)!
        defer { shared.removePersistentDomain(forName: sharedName); local.removePersistentDomain(forName: localName) }
        KeyboardPreferences.setLearningEnabled(true, in: shared)
        KeyboardPreferences.setLearningEnabled(true, in: local)
        KeyboardPreferences.learn("Pennel", previous: "Use", in: local)
        KeyboardPreferences.learn("Britt", previous: "Ask", in: local)
        KeyboardPreferences.forgetLearnedWord("pennel", in: shared)
        KeyboardPreferences.mergeExtensionVocabulary(from: local, to: shared)
        #expect(KeyboardPreferences.learnedWords(in: shared) == ["Britt"])
        #expect(KeyboardPreferences.learnedPairs(in: shared).allSatisfy { $0.text != "Pennel" })
        #expect(KeyboardPreferences.learnedWords(in: local).isEmpty)
        KeyboardPreferences.learn("Pennel", previous: "Use", in: shared)
        #expect(KeyboardPreferences.learnedWords(in: shared) == ["Britt"])
        KeyboardPreferences.learn("Later", previous: "See", in: local)
        KeyboardPreferences.resetLearning(in: shared)
        KeyboardPreferences.mirrorTypingConfiguration(from: shared, to: local)
        #expect(KeyboardPreferences.learnedWords(in: local).isEmpty)
        #expect(KeyboardPreferences.learnedPairs(in: local).isEmpty)
    }
}
