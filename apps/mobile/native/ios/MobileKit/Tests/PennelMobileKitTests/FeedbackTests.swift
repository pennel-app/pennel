import Foundation
import Testing
@testable import PennelMobileKit

struct FeedbackTests {
    @Test func defaults_enable_action_feedback_but_leave_typing_optional() {
        #expect(KeyboardPreferences.feedbackEnabled(in: nil))
        #expect(!KeyboardPreferences.typingFeedbackEnabled(in: nil))
        #expect(FeedbackPolicy.pattern(for: .recordingStarted, feedbackEnabled: true, typingEnabled: false) == .lightImpact)
        #expect(FeedbackPolicy.pattern(for: .keyPress, feedbackEnabled: true, typingEnabled: false) == nil)
    }

    @Test func typing_feedback_and_action_feedback_have_independent_preferences() {
        #expect(FeedbackPolicy.pattern(for: .keyPress, feedbackEnabled: false, typingEnabled: true) == .softImpact)
        #expect(FeedbackPolicy.pattern(for: .resultDelivered, feedbackEnabled: false, typingEnabled: true) == nil)
        #expect(FeedbackPolicy.pattern(for: .deviceConnected, feedbackEnabled: true, typingEnabled: false) == .success)
        #expect(FeedbackPolicy.pattern(for: .error, feedbackEnabled: true, typingEnabled: false) == .error)
    }

    @Test func feedback_choices_can_be_mirrored_without_full_access() {
        let sharedName = "pennel-feedback-shared-\(UUID().uuidString)"
        let localName = "pennel-feedback-local-\(UUID().uuidString)"
        let shared = UserDefaults(suiteName: sharedName)!
        let local = UserDefaults(suiteName: localName)!
        defer { shared.removePersistentDomain(forName: sharedName); local.removePersistentDomain(forName: localName) }
        KeyboardPreferences.setFeedbackEnabled(false, in: shared)
        KeyboardPreferences.setTypingFeedbackEnabled(true, in: shared)
        KeyboardPreferences.mirrorFeedback(from: shared, to: local)
        #expect(!KeyboardPreferences.feedbackEnabled(in: local))
        #expect(KeyboardPreferences.typingFeedbackEnabled(in: local))
    }

    @Test func legacy_typing_choice_maps_to_gentle_without_writing_during_reads() {
        let name = "pennel-feedback-legacy-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        #expect(KeyboardFeedbackPreferences(in: defaults).typingStyle == .off)
        KeyboardPreferences.setTypingFeedbackEnabled(true, in: defaults)
        #expect(KeyboardPreferences.typingFeedbackStyle(in: defaults) == .gentle)
        #expect(defaults.object(forKey: KeyboardPreferences.typingFeedbackStyleKey) == nil)
        #expect(KeyboardFeedbackPolicy.typingPulse(for: .gentle) == .impact(style: .soft, intensity: 0.4))
        KeyboardPreferences.setTypingFeedbackEnabled(false, in: defaults)
        #expect(KeyboardPreferences.typingFeedbackStyle(in: defaults) == .off)
    }

    @Test func off_preserves_the_chosen_feel_for_legacy_toggle_and_mirroring() {
        let sharedName = "pennel-feedback-profile-shared-\(UUID().uuidString)"
        let localName = "pennel-feedback-profile-local-\(UUID().uuidString)"
        let shared = UserDefaults(suiteName: sharedName)!
        let local = UserDefaults(suiteName: localName)!
        defer { shared.removePersistentDomain(forName: sharedName); local.removePersistentDomain(forName: localName) }
        KeyboardPreferences.setTypingFeedbackStyle(.crisp, in: shared)
        #expect(KeyboardPreferences.typingFeedbackEnabled(in: shared))
        #expect(KeyboardPreferences.typingFeedbackStyle(in: shared) == .crisp)
        KeyboardPreferences.setTypingFeedbackStyle(.off, in: shared)
        KeyboardPreferences.mirrorKeyboardFeedback(from: shared, to: local)
        #expect(KeyboardPreferences.typingFeedbackStyle(in: local) == .off)
        KeyboardPreferences.setTypingFeedbackEnabled(true, in: local)
        #expect(KeyboardPreferences.typingFeedbackStyle(in: local) == .crisp)
    }

    @Test func invalid_stored_styles_fall_back_without_enabling_typing() {
        let name = "pennel-feedback-invalid-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set("unknown", forKey: KeyboardPreferences.typingFeedbackStyleKey)
        defaults.set("unknown", forKey: KeyboardPreferences.feedbackTrialProfileKey)
        #expect(KeyboardPreferences.typingFeedbackStyle(in: defaults) == .off)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: defaults) == nil)
        KeyboardPreferences.setTypingFeedbackEnabled(true, in: defaults)
        #expect(KeyboardPreferences.typingFeedbackStyle(in: defaults) == .gentle)
    }

    @Test func trial_profiles_round_trip_and_clear_stale_mirrors() {
        let sharedName = "pennel-feedback-trial-shared-\(UUID().uuidString)"
        let localName = "pennel-feedback-trial-local-\(UUID().uuidString)"
        let shared = UserDefaults(suiteName: sharedName)!
        let local = UserDefaults(suiteName: localName)!
        defer { shared.removePersistentDomain(forName: sharedName); local.removePersistentDomain(forName: localName) }
        for profile in KeyboardFeedbackTrialProfile.allCases {
            KeyboardPreferences.setFeedbackTrialProfile(profile, in: shared)
            KeyboardPreferences.mirrorKeyboardFeedback(from: shared, to: local)
            #expect(KeyboardFeedbackPreferences(in: local).trialProfile == profile)
        }
        KeyboardPreferences.setFeedbackTrialProfile(nil, in: shared)
        KeyboardPreferences.mirrorKeyboardFeedback(from: shared, to: local)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: local) == nil)
    }

    @Test func baseline_and_prepared_compare_the_same_pulse_with_different_preparation() {
        let baseline = KeyboardFeedbackPreferences(actionFeedbackEnabled: true, typingStyle: .gentle, trialProfile: .baseline)
        let prepared = KeyboardFeedbackPreferences(actionFeedbackEnabled: true, typingStyle: .gentle, trialProfile: .prepared)
        #expect(KeyboardFeedbackPolicy.pulse(for: .keyPress, preferences: baseline)
            == KeyboardFeedbackPolicy.pulse(for: .keyPress, preferences: prepared))
        #expect(!KeyboardFeedbackPolicy.preparesTyping(for: baseline))
        #expect(KeyboardFeedbackPolicy.preparesTyping(for: prepared))
    }

    @Test func trial_never_enables_disabled_typing_and_off_only_suppresses_typing() {
        for trial in KeyboardFeedbackTrialProfile.allCases {
            let preferences = KeyboardFeedbackPreferences(actionFeedbackEnabled: true, typingStyle: .off, trialProfile: trial)
            #expect(KeyboardFeedbackPolicy.pulse(for: .keyPress, preferences: preferences) == nil)
            #expect(!KeyboardFeedbackPolicy.preparesTyping(for: preferences))
        }
        let preferences = KeyboardFeedbackPreferences(actionFeedbackEnabled: true, typingStyle: .gentle, trialProfile: .off)
        #expect(KeyboardFeedbackPolicy.pulse(for: .keyPress, preferences: preferences) == nil)
        #expect(KeyboardFeedbackPolicy.pulse(for: .resultDelivered, preferences: preferences) == .success)
        #expect(!KeyboardFeedbackPolicy.preparesTyping(for: preferences))
        #expect(KeyboardFeedbackPolicy.previewPulse(for: .off) == nil)
    }

    @Test func keyboard_action_feedback_remains_independent_and_uses_acknowledged_events() {
        let typingOnly = KeyboardFeedbackPreferences(actionFeedbackEnabled: false, typingStyle: .crisp)
        #expect(KeyboardFeedbackPolicy.pulse(for: .keyPress, preferences: typingOnly) == .impact(style: .rigid, intensity: 0.65))
        for event in FeedbackEvent.allCases where event != .keyPress {
            #expect(KeyboardFeedbackPolicy.pulse(for: event, preferences: typingOnly) == nil)
        }
        let actionsOnly = KeyboardFeedbackPreferences(actionFeedbackEnabled: true, typingStyle: .off)
        #expect(KeyboardFeedbackPolicy.pulse(for: .selection, preferences: actionsOnly) == .selection)
        #expect(KeyboardFeedbackPolicy.pulse(for: .recordingStarted, preferences: actionsOnly) == .impact(style: .light, intensity: 1))
        #expect(KeyboardFeedbackPolicy.pulse(for: .recordingStopped, preferences: actionsOnly) == .impact(style: .medium, intensity: 1))
        #expect(KeyboardFeedbackPolicy.pulse(for: .warning, preferences: actionsOnly) == .warning)
        #expect(KeyboardFeedbackPolicy.pulse(for: .error, preferences: actionsOnly) == .error)
    }

    @Test func feedback_snapshot_is_codable_for_native_settings() throws {
        let original = KeyboardFeedbackPreferences(actionFeedbackEnabled: false, typingStyle: .crisp, trialProfile: .light)
        let data = try JSONEncoder().encode(original)
        #expect(try JSONDecoder().decode(KeyboardFeedbackPreferences.self, from: data) == original)
    }

    @Test func app_samples_use_the_same_profiles_as_the_keyboard_without_changing_settings() {
        #expect(KeyboardFeedbackPreviewProfile.gentle.pulse == KeyboardFeedbackPolicy.typingPulse(for: .gentle))
        #expect(KeyboardFeedbackPreviewProfile.crisp.pulse == KeyboardFeedbackPolicy.typingPulse(for: .crisp))
        for trial in KeyboardFeedbackTrialProfile.allCases {
            let preview = KeyboardFeedbackPreviewProfile(rawValue: trial.rawValue)!
            #expect(preview.pulse == KeyboardFeedbackPolicy.previewPulse(for: trial))
        }
        #expect(!KeyboardFeedbackPreviewProfile.baseline.preparesFeedback)
        #expect(KeyboardFeedbackPreviewProfile.prepared.preparesFeedback)
        #expect(!KeyboardFeedbackPreviewProfile.off.preparesFeedback)
        #expect(KeyboardFeedbackPreviewProfile.off.pulse == nil)
        #expect(KeyboardFeedbackPreviewProfile(rawValue: "unknown") == nil)
    }

    @Test func daily_crisp_is_distinct_from_the_original_rigid_comparison_control() {
        #expect(KeyboardFeedbackPolicy.previewPulse(for: .rigid) == .impact(style: .rigid, intensity: 0.25))
        #expect(KeyboardFeedbackPreviewProfile.crisp.pulse != KeyboardFeedbackPreviewProfile.rigid.pulse)
        #expect(KeyboardFeedbackPolicy.typingPulse(for: .crisp, trialProfile: .rigid)
            == KeyboardFeedbackPreviewProfile.rigid.pulse)
        #expect(KeyboardFeedbackPolicy.typingPulse(for: .gentle) == .impact(style: .soft, intensity: 0.4))
    }

    @Test func temporary_trials_expire_without_reading_extending_or_rewriting_them() {
        let name = "pennel-feedback-expiry-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        let start = Date()
        KeyboardPreferences.setFeedbackTrialProfile(.rigid, in: defaults, at: start)
        let deadline = defaults.object(forKey: KeyboardPreferences.feedbackTrialExpiresAtKey) as? Double
        #expect(KeyboardPreferences.feedbackTrialProfile(in: defaults, at: start) == .rigid)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: defaults, at: start.addingTimeInterval(299)) == .rigid)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: defaults, at: start.addingTimeInterval(300)) == nil)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: defaults, at: start.addingTimeInterval(-1)) == nil)
        #expect(defaults.object(forKey: KeyboardPreferences.feedbackTrialExpiresAtKey) as? Double == deadline)
        KeyboardPreferences.setTypingFeedbackStyle(.gentle, in: defaults)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: defaults) == nil)
        #expect(defaults.object(forKey: KeyboardPreferences.feedbackTrialExpiresAtKey) == nil)
    }

    @Test func mirroring_a_trial_preserves_its_original_deadline() {
        let sharedName = "pennel-feedback-expiry-shared-\(UUID().uuidString)"
        let localName = "pennel-feedback-expiry-local-\(UUID().uuidString)"
        let shared = UserDefaults(suiteName: sharedName)!
        let local = UserDefaults(suiteName: localName)!
        defer { shared.removePersistentDomain(forName: sharedName); local.removePersistentDomain(forName: localName) }
        let start = Date().addingTimeInterval(-120)
        KeyboardPreferences.setFeedbackTrialProfile(.light, in: shared, at: start)
        KeyboardPreferences.mirrorKeyboardFeedback(from: shared, to: local)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: local) == .light)
        #expect(local.object(forKey: KeyboardPreferences.feedbackTrialExpiresAtKey) as? Double
            == shared.object(forKey: KeyboardPreferences.feedbackTrialExpiresAtKey) as? Double)
        #expect(KeyboardPreferences.feedbackTrialProfile(in: local, at: start.addingTimeInterval(300)) == nil)
    }
}
