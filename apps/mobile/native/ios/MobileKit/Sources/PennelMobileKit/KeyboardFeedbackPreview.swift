import Foundation
#if canImport(UIKit)
import UIKit
#endif

public enum KeyboardFeedbackPreviewProfile: String, CaseIterable, Sendable {
    case gentle, crisp, baseline, prepared, light, rigid, off

    public var pulse: KeyboardFeedbackPulse? {
        switch self {
        case .gentle: return KeyboardFeedbackPolicy.typingPulse(for: .gentle)
        case .crisp: return KeyboardFeedbackPolicy.typingPulse(for: .crisp)
        case .baseline: return KeyboardFeedbackPolicy.previewPulse(for: .baseline)
        case .prepared: return KeyboardFeedbackPolicy.previewPulse(for: .prepared)
        case .light: return KeyboardFeedbackPolicy.previewPulse(for: .light)
        case .rigid: return KeyboardFeedbackPolicy.previewPulse(for: .rigid)
        case .off: return nil
        }
    }

    public var preparesFeedback: Bool { self != .baseline && self != .off }
}

#if canImport(UIKit)
@MainActor
public final class KeyboardFeedbackPreview {
    private weak var view: UIView?
    private var generators: [KeyboardFeedbackImpactStyle: UIImpactFeedbackGenerator] = [:]
    private var observations: [KeyboardPreviewObservation] = []
    private var active = true
    private var stopped = false

    public init(view: UIView) {
        self.view = view
        for name in [UIApplication.willResignActiveNotification, UIApplication.didEnterBackgroundNotification] {
            observe(name) { preview in
                preview.active = false
                preview.releaseGenerators()
            }
        }
        observe(UIApplication.didBecomeActiveNotification) { $0.active = true }
    }

    public func prepare(profile: KeyboardFeedbackPreviewProfile = .gentle) {
        guard let view = activeView, profile.preparesFeedback,
              case let .impact(style, _) = profile.pulse
        else { return }
        generator(style, in: view).prepare()
    }

    public func play(profile: KeyboardFeedbackPreviewProfile) {
        guard let view = activeView else { return }
        guard case let .impact(style, intensity) = profile.pulse else {
            releaseGenerators()
            return
        }
        generator(style, in: view).impactOccurred(intensity: CGFloat(intensity))
    }

    public func stop() {
        stopped = true
        observations = []
        releaseGenerators()
        view = nil
    }

    private var activeView: UIView? {
        guard !stopped, active, let view, let window = view.window else { return nil }
        if let scene = window.windowScene, scene.activationState != .foregroundActive { return nil }
        return view
    }

    private func generator(_ style: KeyboardFeedbackImpactStyle, in view: UIView) -> UIImpactFeedbackGenerator {
        if let generator = generators[style] { return generator }
        let feedbackStyle: UIImpactFeedbackGenerator.FeedbackStyle
        switch style {
        case .soft: feedbackStyle = .soft
        case .light: feedbackStyle = .light
        case .medium: feedbackStyle = .medium
        case .rigid: feedbackStyle = .rigid
        }
        let generator = UIImpactFeedbackGenerator(style: feedbackStyle, view: view)
        generators[style] = generator
        return generator
    }

    private func releaseGenerators() {
        for generator in generators.values { generator.view?.removeInteraction(generator) }
        generators = [:]
    }

    private func observe(_ name: Notification.Name, action: @escaping @MainActor (KeyboardFeedbackPreview) -> Void) {
        let token = NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { @Sendable [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, !self.stopped else { return }
                action(self)
            }
        }
        observations.append(KeyboardPreviewObservation(token))
    }
}

private final class KeyboardPreviewObservation: @unchecked Sendable {
    private let token: NSObjectProtocol

    init(_ token: NSObjectProtocol) { self.token = token }

    deinit { NotificationCenter.default.removeObserver(token) }
}
#endif
