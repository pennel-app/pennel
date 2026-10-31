import PennelMobileKit
import UIKit

@MainActor
final class KeyboardFeedbackController {
    private weak var view: UIView?
    private let preferences: () -> KeyboardFeedbackPreferences
    private var impacts: [KeyboardFeedbackImpactStyle: UIImpactFeedbackGenerator] = [:]
    private var selection: UISelectionFeedbackGenerator?
    private var notification: UINotificationFeedbackGenerator?
    private var observations: [KeyboardFeedbackObservation] = []
    private var hostActive = true
    private var stopped = false

    init(view: UIView, preferences: @escaping () -> KeyboardFeedbackPreferences) {
        self.view = view
        self.preferences = preferences
        for name in [Notification.Name.NSExtensionHostWillResignActive, .NSExtensionHostDidEnterBackground] {
            observe(name) { controller in
                controller.hostActive = false
                controller.releaseGenerators()
            }
        }
        observe(.NSExtensionHostDidBecomeActive) { $0.hostActive = true }
    }

    func prepareTyping() {
        guard let view = activeView else { return }
        let current = preferences()
        guard KeyboardFeedbackPolicy.preparesTyping(for: current),
              case let .impact(style, _) = KeyboardFeedbackPolicy.typingPulse(
                for: current.typingStyle, trialProfile: current.trialProfile)
        else { return }
        impact(style, in: view).prepare()
    }

    func play(_ event: FeedbackEvent, at location: CGPoint? = nil) {
        guard let view = activeView else { return }
        let current = preferences()
        guard let pulse = KeyboardFeedbackPolicy.pulse(for: event, preferences: current) else { return }
        deliver(pulse, in: view, at: location)
        if event == .keyPress, KeyboardFeedbackPolicy.preparesTyping(for: current) {
            prepareTyping()
        }
    }

    func preview(_ profile: KeyboardFeedbackTrialProfile) {
        guard let view = activeView, let pulse = KeyboardFeedbackPolicy.previewPulse(for: profile) else { return }
        deliver(pulse, in: view, at: nil)
    }

    func stop() {
        stopped = true
        observations = []
        releaseGenerators()
        view = nil
    }

    private var activeView: UIView? {
        guard !stopped, hostActive, let view, view.window != nil else { return nil }
        return view
    }

    private func impact(_ style: KeyboardFeedbackImpactStyle, in view: UIView) -> UIImpactFeedbackGenerator {
        if let generator = impacts[style] { return generator }
        let feedbackStyle: UIImpactFeedbackGenerator.FeedbackStyle
        switch style {
        case .soft: feedbackStyle = .soft
        case .light: feedbackStyle = .light
        case .medium: feedbackStyle = .medium
        case .rigid: feedbackStyle = .rigid
        }
        let generator = UIImpactFeedbackGenerator(style: feedbackStyle, view: view)
        impacts[style] = generator
        return generator
    }

    private func deliver(_ pulse: KeyboardFeedbackPulse, in view: UIView, at location: CGPoint?) {
        switch pulse {
        case let .impact(style, intensity):
            let generator = impact(style, in: view)
            if let location {
                generator.impactOccurred(intensity: CGFloat(intensity), at: location)
            } else {
                generator.impactOccurred(intensity: CGFloat(intensity))
            }
        case .selection:
            let generator = selection ?? UISelectionFeedbackGenerator(view: view)
            selection = generator
            if let location { generator.selectionChanged(at: location) } else { generator.selectionChanged() }
        case .success, .warning, .error:
            let generator = notification ?? UINotificationFeedbackGenerator(view: view)
            notification = generator
            let type: UINotificationFeedbackGenerator.FeedbackType
            switch pulse {
            case .success: type = .success
            case .warning: type = .warning
            default: type = .error
            }
            if let location {
                generator.notificationOccurred(type, at: location)
            } else {
                generator.notificationOccurred(type)
            }
        }
    }

    private func releaseGenerators() {
        // A view retains generators as interactions until they are explicitly removed.
        for generator in impacts.values { generator.view?.removeInteraction(generator) }
        if let selection { selection.view?.removeInteraction(selection) }
        if let notification { notification.view?.removeInteraction(notification) }
        impacts = [:]
        selection = nil
        notification = nil
    }

    private func observe(_ name: Notification.Name, action: @escaping @MainActor (KeyboardFeedbackController) -> Void) {
        let token = NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { @Sendable [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, !self.stopped else { return }
                action(self)
            }
        }
        observations.append(KeyboardFeedbackObservation(token))
    }
}

private final class KeyboardFeedbackObservation: @unchecked Sendable {
    private let token: NSObjectProtocol

    init(_ token: NSObjectProtocol) { self.token = token }

    deinit { NotificationCenter.default.removeObserver(token) }
}
