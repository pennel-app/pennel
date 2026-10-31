import Foundation

public enum KeyboardTypingContext {
    public static func token(before: String?) -> String {
        guard let before, before.last?.isWhitespace != true else { return "" }
        let raw = before.suffix(128).reversed().prefix { !$0.isWhitespace }.reversed()
        let opening = CharacterSet(charactersIn: "\"“‘([{")
        return String(raw.drop { character in character.unicodeScalars.allSatisfy { opening.contains($0) } })
    }

    public static func previousToken(before: String?) -> String? {
        guard let before else { return nil }
        let words = before.suffix(256).split(whereSeparator: \.isWhitespace)
        let index = before.last?.isWhitespace == true ? words.count - 1 : words.count - 2
        guard words.indices.contains(index) else { return nil }
        let word = String(words[index]).trimmingCharacters(in: .punctuationCharacters)
        return word.isEmpty ? nil : word
    }

    public static func wordDeletionCount(before: String?) -> Int {
        guard let before, !before.isEmpty else { return 0 }
        let characters = Array(before.suffix(128))
        var count = 0
        for character in characters.reversed() {
            if !character.isWhitespace { break }
            count += 1
        }
        for character in characters.dropLast(count).reversed() {
            if character.isWhitespace { break }
            count += 1
        }
        return min(count, 128)
    }

    public static func permitsManualReplacement(token: String, before: String?, after: String?) -> Bool {
        guard token.count <= 128, let before, before.hasSuffix(token) else { return false }
        return after?.first.map { !$0.isLetter && !$0.isNumber } ?? true
    }
}

public struct KeyboardCorrectionTransaction: Sendable {
    public let original: String
    public let replacement: String
    public let delimiter: String
    public let document: UUID
    public let lifetime: UUID
    private let expectedBefore: String
    private let expectedAfter: String?
    private var contextGeneration: UInt64?

    public init?(original: String, replacement: String, delimiter: String, before: String?, after: String?,
                 document: UUID, lifetime: UUID) {
        guard !original.isEmpty, original.count <= 128, !replacement.isEmpty, replacement.count <= 4_096,
              delimiter.count <= 2, let before, before.hasSuffix(original) else { return nil }
        self.original = original
        self.replacement = replacement
        self.delimiter = delimiter
        self.document = document
        self.lifetime = lifetime
        expectedBefore = String(before.dropLast(original.count).suffix(96)) + replacement + delimiter
        expectedAfter = after.map { String($0.prefix(96)) }
    }

    public var replacementCharacterCount: Int { replacement.count + delimiter.count }
    public var restoredText: String { original + delimiter }

    public func bound(to context: KeyboardInsertionContext) -> Self? {
        guard context.document == document, context.lifetime == lifetime else { return nil }
        var transaction = self
        transaction.contextGeneration = context.generation
        return transaction
    }

    public func permits(before: String?, after: String?, document: UUID, lifetime: UUID, generation: UInt64? = nil) -> Bool {
        guard self.document == document, self.lifetime == lifetime, let before,
              contextGeneration == nil || contextGeneration == generation else { return false }
        guard before.count >= replacementCharacterCount else { return false }
        let visibleCount = min(before.count, expectedBefore.count)
        return before.suffix(visibleCount) == expectedBefore.suffix(visibleCount)
            && after.map { String($0.prefix(96)) } == expectedAfter
    }
}

public struct KeyboardSelectionSnapshot: Sendable {
    public let text: String
    public let document: UUID
    public let lifetime: UUID
    private let before: String?
    private let after: String?

    public init(text: String, before: String?, after: String?, document: UUID, lifetime: UUID) {
        self.text = text
        self.before = before.map { String($0.suffix(96)) }
        self.after = after.map { String($0.prefix(96)) }
        self.document = document
        self.lifetime = lifetime
    }

    public func permits(selectedText: String?, before: String?, after: String?, document: UUID, lifetime: UUID) -> Bool {
        self.document == document && self.lifetime == lifetime && selectedText == text
            && self.before == before.map { String($0.suffix(96)) }
            && self.after == after.map { String($0.prefix(96)) }
    }

    public func restoration(replacement: String) -> KeyboardWritingRestoration {
        KeyboardWritingRestoration(original: text, replacement: replacement, before: before, after: after,
                                   document: document, lifetime: lifetime)
    }
}

public struct KeyboardWritingRestoration: Sendable {
    public let original: String
    public let replacement: String
    private let before: String?
    private let after: String?
    private let document: UUID
    private let lifetime: UUID

    init(original: String, replacement: String, before: String?, after: String?, document: UUID, lifetime: UUID) {
        self.original = original
        self.replacement = replacement
        self.before = before
        self.after = after
        self.document = document
        self.lifetime = lifetime
    }

    public func permits(before: String?, after: String?, selectedText: String?, document: UUID, lifetime: UUID) -> Bool {
        guard self.document == document, self.lifetime == lifetime, selectedText?.isEmpty != false,
              let prefix = self.before, let suffix = self.after, let before, let after else { return false }
        guard before.count >= replacement.count else { return false }
        let expected = prefix + replacement
        let visibleCount = min(before.count, expected.count)
        return before.suffix(visibleCount) == expected.suffix(visibleCount)
            && String(after.prefix(96)) == suffix
    }
}
