import PennelMobileKit
import SwiftUI
import UIKit

struct KeyboardPreparationButtonStyle: ButtonStyle {
    let prepare: () -> Void
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.7 : 1)
            .onChange(of: configuration.isPressed) { _, pressed in if pressed { prepare() } }
    }
}

struct KeyboardSpaceKey: UIViewRepresentable {
    let label: String
    let language: KeyboardInterfaceLanguage
    let action: () -> Void
    let moveCursor: (Int) -> Void
    let prepareFeedback: () -> Void
    let feedback: (FeedbackEvent) -> Void
    var allowsCursorMovement = true

    func makeUIView(context: Context) -> ScrubbingSpaceButton { ScrubbingSpaceButton(type: .custom) }
    func updateUIView(_ button: ScrubbingSpaceButton, context: Context) {
        button.label = label
        if !button.isScrubbing { button.setTitle(label, for: .normal) }
        button.action = action; button.moveCursor = moveCursor
        button.prepareFeedback = prepareFeedback; button.feedback = feedback
        button.allowsCursorMovement = allowsCursorMovement
        button.accessibilityLabel = label
        button.accessibilityHint = allowsCursorMovement ? (language == .dutch ? "Hou ingedrukt en sleep om de cursor te verplaatsen." : "Touch and hold, then drag to move the cursor.") : ""
        button.accessibilityTraits = [.keyboardKey]
        button.accessibilityCustomActions = allowsCursorMovement ? [
            UIAccessibilityCustomAction(name: language == .dutch ? "Cursor naar links" : "Move cursor left") { [weak button] _ in button?.moveCursor?(-1); return button != nil },
            UIAccessibilityCustomAction(name: language == .dutch ? "Cursor naar rechts" : "Move cursor right") { [weak button] _ in button?.moveCursor?(1); return button != nil },
        ] : []
        styleKeyboardEditingButton(button)
    }
    static func dismantleUIView(_ view: ScrubbingSpaceButton, coordinator: ()) { view.cancelInteraction() }
}

final class ScrubbingSpaceButton: UIButton {
    var label = ""
    var action: (() -> Void)?
    var moveCursor: ((Int) -> Void)?
    var prepareFeedback: (() -> Void)?
    var feedback: ((FeedbackEvent) -> Void)?
    var allowsCursorMovement = true
    private(set) var isScrubbing = false
    private var start: CGPoint?
    private var beganAt: TimeInterval = 0
    private var lastOffset = 0
    private var holdTimer: Timer?

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first, start == nil else { return }
        start = touch.location(in: self); beganAt = touch.timestamp; lastOffset = 0
        prepareFeedback?(); isHighlighted = true
        if allowsCursorMovement { holdTimer = Timer.scheduledTimer(timeInterval: 0.28, target: self, selector: #selector(beginScrubbing), userInfo: nil, repeats: false) }
    }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first, let start else { return }
        let point = touch.location(in: self)
        if allowsCursorMovement, !isScrubbing, touch.timestamp - beganAt >= 0.18, abs(point.x - start.x) >= Brand.Spacing.s3 { beginScrubbing() }
        guard isScrubbing else { isHighlighted = bounds.contains(point); return }
        let offset = KeyboardGestureSteps.offset(translation: Double(point.x - start.x))
        let delta = offset - lastOffset
        if delta != 0 { moveCursor?(delta); lastOffset = offset }
    }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first, start != nil else { return }
        let commit = !isScrubbing && bounds.contains(touch.location(in: self))
        cancelInteraction()
        if commit { action?() }
    }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { cancelInteraction() }
    override func didMoveToWindow() { super.didMoveToWindow(); if window == nil { cancelInteraction() } }
    override func accessibilityActivate() -> Bool { prepareFeedback?(); action?(); return action != nil }
    @objc private func beginScrubbing() {
        guard allowsCursorMovement, start != nil, !isScrubbing else { return }
        holdTimer?.invalidate(); holdTimer = nil
        isScrubbing = true; setTitle("↔︎", for: .normal)
        feedback?(.selection)
    }
    func cancelInteraction() {
        holdTimer?.invalidate(); holdTimer = nil
        start = nil; isScrubbing = false; isHighlighted = false; lastOffset = 0
        setTitle(label, for: .normal)
    }
}

struct KeyboardSymbolLayerKey: UIViewRepresentable {
    let label: String
    let accessibilityLabel: String
    let inputTarget: KeyboardLetterInputTarget
    let begin: () -> Bool
    let finish: (Bool) -> Void
    let cancel: () -> Void
    let insert: (String) -> Void
    let prepareFeedback: () -> Void

    func makeUIView(context: Context) -> SlidingSymbolButton { SlidingSymbolButton(type: .custom) }
    func updateUIView(_ button: SlidingSymbolButton, context: Context) {
        button.setTitle(label, for: .normal); button.accessibilityLabel = accessibilityLabel
        button.accessibilityTraits = [.keyboardKey]
        button.inputTarget = inputTarget; button.begin = begin; button.finish = finish
        button.cancel = cancel; button.insert = insert; button.prepareFeedback = prepareFeedback
        styleKeyboardEditingButton(button)
    }
    static func dismantleUIView(_ view: SlidingSymbolButton, coordinator: ()) { view.cancelInteraction() }
}

final class SlidingSymbolButton: UIButton {
    weak var inputTarget: KeyboardLetterInputTarget?
    var begin: (() -> Bool)?
    var finish: ((Bool) -> Void)?
    var cancel: (() -> Void)?
    var insert: ((String) -> Void)?
    var prepareFeedback: (() -> Void)?
    private var start: CGPoint?
    private var slideEnabled = false
    private var moved = false

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first, start == nil else { return }
        start = touch.location(in: self); moved = false
        prepareFeedback?(); isHighlighted = true; slideEnabled = begin?() ?? false
    }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first, let start, slideEnabled else { return }
        let point = touch.location(in: self)
        if hypot(point.x - start.x, point.y - start.y) > Brand.Spacing.s3 { moved = true }
        if moved { inputTarget?.highlight(at: point, from: self) }
    }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first, start != nil else { return }
        let point = touch.location(in: self)
        if slideEnabled, moved, let value = inputTarget?.character(at: point, from: self) {
            reset(); finish?(true); insert?(value)
        } else if bounds.contains(point) { reset(); finish?(false) }
        else { cancelInteraction() }
    }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { cancelInteraction() }
    override func didMoveToWindow() { super.didMoveToWindow(); if window == nil { cancelInteraction() } }
    override func accessibilityActivate() -> Bool { prepareFeedback?(); _ = begin?(); finish?(false); return begin != nil }
    func cancelInteraction() { if start != nil { reset(); cancel?() } }
    private func reset() { start = nil; moved = false; slideEnabled = false; isHighlighted = false; inputTarget?.clearHighlight() }
}

@MainActor
private func styleKeyboardEditingButton(_ button: UIButton) {
    button.setTitleColor(UIColor(Brand.foreground.color), for: .normal)
    button.backgroundColor = UIColor(Brand.subtle.color)
    button.layer.cornerRadius = Brand.Radius.keycap
    let font = UIFont(name: "Figtree-Medium", size: Brand.FontSize.body) ?? .systemFont(ofSize: Brand.FontSize.body, weight: .medium)
    button.titleLabel?.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: font)
    button.titleLabel?.adjustsFontForContentSizeCategory = true
    button.titleLabel?.adjustsFontSizeToFitWidth = true
    button.titleLabel?.minimumScaleFactor = 0.7
}
