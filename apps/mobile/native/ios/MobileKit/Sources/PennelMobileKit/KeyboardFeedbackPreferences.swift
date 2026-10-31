import Foundation

public enum KeyboardTypingFeedbackStyle: String, CaseIterable, Codable, Sendable {
    case off, gentle, crisp
}

public enum KeyboardFeedbackTrialProfile: String, CaseIterable, Codable, Sendable {
    case baseline, prepared, light, rigid, off
}

public struct KeyboardFeedbackPreferences: Equatable, Codable, Sendable {
    public var actionFeedbackEnabled: Bool
    public var typingStyle: KeyboardTypingFeedbackStyle
    public var trialProfile: KeyboardFeedbackTrialProfile?

    public init(
        actionFeedbackEnabled: Bool,
        typingStyle: KeyboardTypingFeedbackStyle,
        trialProfile: KeyboardFeedbackTrialProfile? = nil
    ) {
        self.actionFeedbackEnabled = actionFeedbackEnabled
        self.typingStyle = typingStyle
        self.trialProfile = trialProfile
    }

    public init(in defaults: UserDefaults?) {
        self.init(
            actionFeedbackEnabled: KeyboardPreferences.feedbackEnabled(in: defaults),
            typingStyle: KeyboardPreferences.typingFeedbackStyle(in: defaults),
            trialProfile: KeyboardPreferences.feedbackTrialProfile(in: defaults))
    }
}

extension KeyboardPreferences {
    public static let typingFeedbackStyleKey = "keyboardTypingFeedbackStyle"
    public static let feedbackTrialProfileKey = "keyboardFeedbackTrialProfile"
    public static let feedbackTrialExpiresAtKey = "keyboardFeedbackTrialExpiresAt"
    public static let feedbackTrialDuration: TimeInterval = 300

    public static func typingFeedbackStyle(in defaults: UserDefaults?) -> KeyboardTypingFeedbackStyle {
        guard typingFeedbackEnabled(in: defaults) else { return .off }
        guard let stored = defaults?.string(forKey: typingFeedbackStyleKey),
              let style = KeyboardTypingFeedbackStyle(rawValue: stored), style != .off
        else { return .gentle }
        return style
    }

    public static func setTypingFeedbackStyle(_ style: KeyboardTypingFeedbackStyle, in defaults: UserDefaults?) {
        // Keep the last enabled feel so the existing on/off setting can restore it.
        if style != .off { defaults?.set(style.rawValue, forKey: typingFeedbackStyleKey) }
        setTypingFeedbackEnabled(style != .off, in: defaults)
        setFeedbackTrialProfile(nil, in: defaults)
    }

    public static func feedbackTrialProfile(in defaults: UserDefaults?, at date: Date = Date()) -> KeyboardFeedbackTrialProfile? {
        let now = date.timeIntervalSince1970
        guard let deadline = defaults?.object(forKey: feedbackTrialExpiresAtKey) as? Double,
              now.isFinite, deadline.isFinite, deadline > now, deadline <= now + feedbackTrialDuration
        else { return nil }
        return defaults?.string(forKey: feedbackTrialProfileKey).flatMap(KeyboardFeedbackTrialProfile.init(rawValue:))
    }

    public static func setFeedbackTrialProfile(
        _ profile: KeyboardFeedbackTrialProfile?, in defaults: UserDefaults?, at date: Date = Date()
    ) {
        if let profile {
            defaults?.set(profile.rawValue, forKey: feedbackTrialProfileKey)
            defaults?.set(date.timeIntervalSince1970 + feedbackTrialDuration, forKey: feedbackTrialExpiresAtKey)
        } else {
            defaults?.removeObject(forKey: feedbackTrialProfileKey)
            defaults?.removeObject(forKey: feedbackTrialExpiresAtKey)
        }
    }

    public static func mirrorKeyboardFeedback(from shared: UserDefaults?, to local: UserDefaults?) {
        mirrorFeedback(from: shared, to: local)
        let storedStyle = shared?.string(forKey: typingFeedbackStyleKey)
            .flatMap(KeyboardTypingFeedbackStyle.init(rawValue:))
        if let storedStyle, storedStyle != .off {
            local?.set(storedStyle.rawValue, forKey: typingFeedbackStyleKey)
        } else {
            local?.removeObject(forKey: typingFeedbackStyleKey)
        }
        if let profile = feedbackTrialProfile(in: shared) {
            local?.set(profile.rawValue, forKey: feedbackTrialProfileKey)
            local?.set(shared?.object(forKey: feedbackTrialExpiresAtKey), forKey: feedbackTrialExpiresAtKey)
        } else {
            setFeedbackTrialProfile(nil, in: local)
        }
    }
}
