import CoreFoundation
import Foundation
import PennelMobileKit

@MainActor
struct NativeKeyboardSettingsPatch {
    private var languages: [String]?
    private var autocorrection: Bool?
    private var learning: Bool?
    private var swipe: Bool?
    private var snippetExpansion: Bool?
    private var snippets: [KeyboardSnippet]?
    private var feedbackStyle: KeyboardTypingFeedbackStyle?
    private var resetLearning = false

    init(removingFrom requested: inout [String: Any]) throws {
        for key in ["keyboard_learned_count", "learned_count", "feedback_style", "trial_active", "keyboard_learned_words", "learned_words"] {
            if requested.removeValue(forKey: key) != nil { throw Self.invalid("Keyboard learning details are read-only.") }
        }
        if let raw = requested.removeValue(forKey: "keyboard_typing_languages") {
            guard let values = raw as? [String], (1...2).contains(values.count),
                  Set(values).count == values.count, values.allSatisfy({ ["en", "nl"].contains($0) })
            else { throw Self.invalid("Choose Dutch, English, or both typing languages.") }
            languages = values
        }
        autocorrection = try Self.boolean("keyboard_autocorrection", from: &requested)
        learning = try Self.boolean("keyboard_learning", from: &requested)
        swipe = try Self.boolean("keyboard_swipe", from: &requested)
        snippetExpansion = try Self.boolean("keyboard_snippet_expansion", from: &requested)
        resetLearning = try Self.boolean("keyboard_learning_reset", from: &requested) ?? false
        if let raw = requested.removeValue(forKey: "keyboard_haptic_style") {
            guard let value = raw as? String, let style = KeyboardTypingFeedbackStyle(rawValue: value)
            else { throw Self.invalid("Unknown keyboard feedback style.") }
            feedbackStyle = style
        }
        if let raw = requested.removeValue(forKey: "keyboard_snippets") {
            snippets = try Self.parseSnippets(raw)
        }
    }

    func apply(in defaults: UserDefaults?) {
        if let languages { KeyboardPreferences.setTypingLanguages(languages, in: defaults) }
        if let autocorrection { KeyboardPreferences.setAutocorrectionEnabled(autocorrection, in: defaults) }
        if let learning { KeyboardPreferences.setLearningEnabled(learning, in: defaults) }
        if let swipe { KeyboardPreferences.setSwipeEnabled(swipe, in: defaults) }
        if let snippetExpansion { KeyboardPreferences.setSnippetExpansionEnabled(snippetExpansion, in: defaults) }
        if let snippets { KeyboardPreferences.setSnippets(snippets, in: defaults) }
        if let feedbackStyle { KeyboardPreferences.setTypingFeedbackStyle(feedbackStyle, in: defaults) }
        if resetLearning { KeyboardPreferences.resetLearning(in: defaults) }
    }

    static func projection(in defaults: UserDefaults?) -> [String: Any] {
        let style = KeyboardPreferences.typingFeedbackStyle(in: defaults).rawValue
        return [
            "keyboard_typing_languages": KeyboardPreferences.typingLanguages(in: defaults),
            "keyboard_autocorrection": KeyboardPreferences.autocorrectionEnabled(in: defaults),
            "keyboard_learning": KeyboardPreferences.learningEnabled(in: defaults),
            "keyboard_swipe": KeyboardPreferences.swipeEnabled(in: defaults),
            "keyboard_snippet_expansion": KeyboardPreferences.snippetExpansionEnabled(in: defaults),
            "keyboard_snippets": KeyboardPreferences.snippets(in: defaults).map {
                ["id": $0.id, "title": $0.title, "keyword": $0.keyword, "text": $0.text]
            },
            "keyboard_haptic_style": style,
            "keyboard_learned_count": KeyboardPreferences.learnedWords(in: defaults).count,
            "feedback_style": style,
            "trial_active": KeyboardPreferences.feedbackTrialProfile(in: defaults) != nil
        ]
    }

    private static func boolean(_ key: String, from values: inout [String: Any]) throws -> Bool? {
        guard let raw = values.removeValue(forKey: key) else { return nil }
        guard let number = raw as? NSNumber, CFGetTypeID(number) == CFBooleanGetTypeID() else {
            throw invalid("Invalid keyboard preference.")
        }
        return number.boolValue
    }

    private static func parseSnippets(_ raw: Any) throws -> [KeyboardSnippet] {
        guard let values = raw as? [[String: Any]], values.count <= 64 else {
            throw invalid("Save at most 64 keyboard snippets.")
        }
        var ids: Set<String> = []
        var keywords: Set<String> = []
        return try values.map { value in
            guard Set(value.keys) == Set(["id", "title", "keyword", "text"]),
                  let id = value["id"] as? String, !id.isEmpty, id.count <= 128,
                  !id.contains(where: \.isWhitespace), !id.unicodeScalars.contains(where: { isControl($0) }),
                  ids.insert(id).inserted,
                  let title = value["title"] as? String, !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, title.count <= 80,
                  !title.unicodeScalars.contains(where: { isControl($0) }),
                  let keyword = value["keyword"] as? String, keyword.count <= 32, keyword.utf8.count <= 128,
                  !keyword.contains(where: \.isWhitespace), !keyword.unicodeScalars.contains(where: { isControl($0) }),
                  keyword.isEmpty || keywords.insert(keyword).inserted,
                  let text = value["text"] as? String, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.utf8.count <= 2048,
                  !text.unicodeScalars.contains(where: { isControl($0) && $0 != "\n" && $0 != "\t" })
            else { throw invalid("Check snippet names, unique shortcuts and text within the supported length.") }
            return KeyboardSnippet(id: id, title: title, keyword: keyword, text: text)
        }
    }

    private static func isControl(_ scalar: UnicodeScalar) -> Bool {
        scalar.value < 0x20 || (0x7f...0x9f).contains(scalar.value)
    }

    private static func invalid(_ message: String) -> PennelBridgeError {
        PennelBridgeError("invalid_request", message)
    }
}
