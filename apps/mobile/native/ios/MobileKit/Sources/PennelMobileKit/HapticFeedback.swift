import Foundation
#if canImport(UIKit)
import UIKit
#endif

public enum FeedbackEvent: String, CaseIterable, Sendable {
    case selection, recordingStarted, recordingStopped, recordingCancelled
    case resultDelivered, meetingSaved, deviceConnected, warning, error, keyPress
}

public enum FeedbackPattern: Equatable, Sendable {
    case selection, softImpact, lightImpact, mediumImpact, success, warning, error
}

public enum FeedbackPolicy {
    public static func pattern(for event: FeedbackEvent, feedbackEnabled: Bool, typingEnabled: Bool) -> FeedbackPattern? {
        if event == .keyPress { return typingEnabled ? .softImpact : nil }
        guard feedbackEnabled else { return nil }
        switch event {
        case .selection: return .selection
        case .recordingStarted: return .lightImpact
        case .recordingStopped: return .mediumImpact
        case .recordingCancelled: return .softImpact
        case .resultDelivered, .meetingSaved, .deviceConnected: return .success
        case .warning: return .warning
        case .error: return .error
        case .keyPress: return nil
        }
    }
}

public enum KeyboardFeedbackImpactStyle: String, Hashable, Sendable {
    case soft, light, medium, rigid
}

public enum KeyboardFeedbackPulse: Equatable, Sendable {
    case impact(style: KeyboardFeedbackImpactStyle, intensity: Double)
    case selection, success, warning, error
}

public enum KeyboardFeedbackPolicy {
    public static func typingPulse(
        for style: KeyboardTypingFeedbackStyle,
        trialProfile: KeyboardFeedbackTrialProfile? = nil
    ) -> KeyboardFeedbackPulse? {
        guard style != .off else { return nil }
        if let trialProfile { return previewPulse(for: trialProfile) }
        switch style {
        case .off: return nil
        case .gentle: return .impact(style: .soft, intensity: 0.4)
        case .crisp: return .impact(style: .rigid, intensity: 0.65)
        }
    }

    public static func previewPulse(for profile: KeyboardFeedbackTrialProfile) -> KeyboardFeedbackPulse? {
        switch profile {
        case .baseline, .prepared: return .impact(style: .soft, intensity: 0.4)
        case .light: return .impact(style: .light, intensity: 0.35)
        case .rigid: return .impact(style: .rigid, intensity: 0.25)
        case .off: return nil
        }
    }

    public static func preparesTyping(for preferences: KeyboardFeedbackPreferences) -> Bool {
        typingPulse(for: preferences.typingStyle, trialProfile: preferences.trialProfile) != nil
            && preferences.trialProfile != .baseline
    }

    public static func pulse(for event: FeedbackEvent, preferences: KeyboardFeedbackPreferences) -> KeyboardFeedbackPulse? {
        if event == .keyPress {
            return typingPulse(for: preferences.typingStyle, trialProfile: preferences.trialProfile)
        }
        guard preferences.actionFeedbackEnabled else { return nil }
        switch event {
        case .selection: return .selection
        case .recordingStarted: return .impact(style: .light, intensity: 1)
        case .recordingStopped: return .impact(style: .medium, intensity: 1)
        case .recordingCancelled: return .impact(style: .soft, intensity: 0.4)
        case .resultDelivered, .meetingSaved, .deviceConnected: return .success
        case .warning: return .warning
        case .error: return .error
        case .keyPress: return nil
        }
    }
}

@MainActor
public enum HapticFeedback {
    #if canImport(UIKit)
    private static let selection = UISelectionFeedbackGenerator()
    private static let soft = UIImpactFeedbackGenerator(style: .soft)
    private static let light = UIImpactFeedbackGenerator(style: .light)
    private static let medium = UIImpactFeedbackGenerator(style: .medium)
    private static let notification = UINotificationFeedbackGenerator()
    #endif

    public static func play(_ event: FeedbackEvent, in defaults: UserDefaults? = KeyboardLayout.groupDefaults) {
        guard let pattern = FeedbackPolicy.pattern(for: event,
            feedbackEnabled: KeyboardPreferences.feedbackEnabled(in: defaults),
            typingEnabled: KeyboardPreferences.typingFeedbackEnabled(in: defaults)) else { return }
        #if canImport(UIKit)
        // UIKit keeps system settings, recording restrictions and unsupported hardware authoritative.
        switch pattern {
        case .selection: selection.selectionChanged()
        case .softImpact: soft.impactOccurred(intensity: 0.4)
        case .lightImpact: light.impactOccurred()
        case .mediumImpact: medium.impactOccurred()
        case .success: notification.notificationOccurred(.success)
        case .warning: notification.notificationOccurred(.warning)
        case .error: notification.notificationOccurred(.error)
        }
        #endif
    }
}
