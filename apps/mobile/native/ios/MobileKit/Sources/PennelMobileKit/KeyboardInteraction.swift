import Foundation

public struct KeyboardSwipePoint: Codable, Equatable, Sendable {
    public let x: Double
    public let y: Double

    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
}

public struct KeyboardSwipeKey: Codable, Equatable, Sendable {
    public let key: String
    public let x: Double
    public let y: Double

    public init(key: String, x: Double, y: Double) {
        self.key = key
        self.x = x
        self.y = y
    }
}

public struct KeyboardSwipeGesture: Equatable, Sendable {
    public let points: [KeyboardSwipePoint]
    public let keys: [KeyboardSwipeKey]

    public init(points: [KeyboardSwipePoint], keys: [KeyboardSwipeKey]) {
        self.points = Array(points.prefix(256))
        self.keys = Array(keys.prefix(64))
    }
}

public enum KeyboardWritingAction: String, CaseIterable, Sendable {
    case cleanup, shorten, translateToEnglish, translateToDutch

    public func title(language: KeyboardInterfaceLanguage) -> String {
        switch (language, self) {
        case (.english, .cleanup): "Clean up"
        case (.dutch, .cleanup): "Verzorg tekst"
        case (.english, .shorten): "Shorten"
        case (.dutch, .shorten): "Korter"
        case (.english, .translateToEnglish): "Translate to English"
        case (.dutch, .translateToEnglish): "Vertaal naar Engels"
        case (.english, .translateToDutch): "Translate to Dutch"
        case (.dutch, .translateToDutch): "Vertaal naar Nederlands"
        }
    }
}

public enum KeyboardGestureSteps {
    public static func offset(translation: Double, step: Double = 12) -> Int {
        guard translation.isFinite, step.isFinite, step > 0 else { return 0 }
        return Int(max(-256, min(256, translation / step)))
    }
}
