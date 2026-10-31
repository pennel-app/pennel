import Foundation
import PennelTypingFFI

public enum TypingCandidateKind: String, Codable, Sendable {
    case completion, correction, prediction, snippet, swipe
}

public struct TypingPlatformCandidate: Codable, Sendable {
    public var text: String
    public var language: String
    public var kind: TypingCandidateKind

    public init(text: String, language: String, kind: TypingCandidateKind = .correction) {
        self.text = text
        self.language = language
        self.kind = kind
    }
}

public struct TypingWord: Codable, Sendable {
    public var text: String
    public var language: String
    public var frequency: UInt32

    public init(text: String, language: String, frequency: UInt32 = 1) {
        self.text = text
        self.language = language
        self.frequency = frequency
    }
}

public struct TypingNextWord: Codable, Sendable {
    public var previous: String
    public var text: String
    public var count: UInt32
    public var language: String

    public init(previous: String, text: String, count: UInt32, language: String) {
        self.previous = previous
        self.text = text
        self.count = count
        self.language = language
    }
}

public struct TypingSnippet: Codable, Sendable {
    public var trigger: String
    public var replacement: String

    public init(trigger: String, replacement: String) {
        self.trigger = trigger
        self.replacement = replacement
    }
}

public struct TypingPoint: Codable, Sendable {
    public var x: Double
    public var y: Double
    public var timeMs: Double?

    public init(x: Double, y: Double, timeMs: Double? = nil) {
        self.x = x
        self.y = y
        self.timeMs = timeMs
    }
}

public struct TypingKey: Codable, Sendable {
    public var key: String
    public var x: Double
    public var y: Double

    public init(key: String, x: Double, y: Double) {
        self.key = key
        self.x = x
        self.y = y
    }
}

public struct TypingRequest: Codable, Sendable {
    public var token: String
    public var enabledLanguages: [String]
    public var platformCandidates: [TypingPlatformCandidate]
    public var validInAnyLanguage: Bool
    public var personalWords: [String]
    public var vocabulary: [TypingWord]
    public var atWordBoundary: Bool
    public var allowAutomaticCorrection: Bool
    public var rejectedCandidates: [String]
    public var previousToken: String?
    public var learnedNextWords: [TypingNextWord]
    public var snippets: [TypingSnippet]
    public var allowSnippetExpansion: Bool
    public var touches: [TypingPoint]
    public var keys: [TypingKey]
    public var maxCandidates: Int

    public init(
        token: String,
        enabledLanguages: [String] = ["en", "nl"],
        platformCandidates: [TypingPlatformCandidate] = [],
        validInAnyLanguage: Bool = false,
        personalWords: [String] = [],
        vocabulary: [TypingWord] = [],
        atWordBoundary: Bool = false,
        allowAutomaticCorrection: Bool = false,
        rejectedCandidates: [String] = [],
        previousToken: String? = nil,
        learnedNextWords: [TypingNextWord] = [],
        snippets: [TypingSnippet] = [],
        allowSnippetExpansion: Bool = false,
        touches: [TypingPoint] = [],
        keys: [TypingKey] = [],
        maxCandidates: Int = 3
    ) {
        self.token = token
        self.enabledLanguages = enabledLanguages
        self.platformCandidates = platformCandidates
        self.validInAnyLanguage = validInAnyLanguage
        self.personalWords = personalWords
        self.vocabulary = vocabulary
        self.atWordBoundary = atWordBoundary
        self.allowAutomaticCorrection = allowAutomaticCorrection
        self.rejectedCandidates = rejectedCandidates
        self.previousToken = previousToken
        self.learnedNextWords = learnedNextWords
        self.snippets = snippets
        self.allowSnippetExpansion = allowSnippetExpansion
        self.touches = touches
        self.keys = keys
        self.maxCandidates = maxCandidates
    }
}

public struct SwipeRequest: Codable, Sendable {
    public var path: [TypingPoint]
    public var keys: [TypingKey]
    public var enabledLanguages: [String]
    public var vocabulary: [TypingWord]
    public var personalWords: [String]
    public var maxCandidates: Int

    public init(
        path: [TypingPoint],
        keys: [TypingKey],
        enabledLanguages: [String] = ["en", "nl"],
        vocabulary: [TypingWord] = [],
        personalWords: [String] = [],
        maxCandidates: Int = 3
    ) {
        self.path = path
        self.keys = keys
        self.enabledLanguages = enabledLanguages
        self.vocabulary = vocabulary
        self.personalWords = personalWords
        self.maxCandidates = maxCandidates
    }
}

public struct TypingCandidate: Codable, Sendable {
    public let text: String
    public let kind: TypingCandidateKind
    public let score: Double

    public init(text: String, kind: TypingCandidateKind, score: Double) {
        self.text = text
        self.kind = kind
        self.score = score
    }
}

public struct TypingResponse: Codable, Sendable {
    public let candidates: [TypingCandidate]
    public let automaticReplacement: String?
    public let replacementKind: TypingCandidateKind?
    public let error: String?

    public init(
        candidates: [TypingCandidate] = [],
        automaticReplacement: String? = nil,
        replacementKind: TypingCandidateKind? = nil,
        error: String? = nil
    ) {
        self.candidates = candidates
        self.automaticReplacement = automaticReplacement
        self.replacementKind = replacementKind
        self.error = error
    }
}

public enum TypingEngine {
    public static let maximumRequestBytes = 1_048_576

    public static func suggestions(for request: TypingRequest) -> TypingResponse {
        evaluate(request, swipe: false)
    }

    public static func swipe(_ request: SwipeRequest) -> TypingResponse {
        evaluate(request, swipe: true)
    }

    private static func evaluate<Request: Encodable>(_ request: Request, swipe: Bool) -> TypingResponse {
        do {
            let data = try JSONEncoder().encode(request)
            guard !data.isEmpty, data.count <= maximumRequestBytes else {
                return TypingResponse(error: "request_too_large")
            }
            return data.withUnsafeBytes { bytes in
                let address = bytes.bindMemory(to: UInt8.self).baseAddress
                let pointer = swipe
                    ? pennel_typing_swipe_json(address, data.count)
                    : pennel_typing_request_json(address, data.count)
                guard let pointer else { return TypingResponse(error: "typing_unavailable") }
                defer { pennel_typing_free_response(pointer) }
                let response = Data(bytes: pointer, count: strlen(pointer))
                return (try? JSONDecoder().decode(TypingResponse.self, from: response))
                    ?? TypingResponse(error: "invalid_response")
            }
        } catch {
            return TypingResponse(error: "invalid_request")
        }
    }
}
