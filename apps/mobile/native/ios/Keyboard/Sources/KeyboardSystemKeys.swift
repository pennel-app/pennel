import PennelMobileKit
import SwiftUI
import UIKit

@MainActor
final class KeyboardInputModeTarget {
    weak var controller: UIInputViewController?
}

struct KeyboardGlobeKey: UIViewRepresentable {
    let target: KeyboardInputModeTarget
    let label: String

    func makeUIView(context: Context) -> UIButton {
        let button = UIButton(type: .system)
        button.setImage(UIImage(systemName: "globe"), for: .normal)
        button.addTarget(target.controller, action: #selector(UIInputViewController.handleInputModeList(from:with:)), for: .allTouchEvents)
        return button
    }

    func updateUIView(_ button: UIButton, context: Context) {
        button.accessibilityLabel = label
        button.tintColor = UIColor(Brand.foreground.color)
        button.backgroundColor = UIColor(Brand.subtle.color)
        button.layer.cornerRadius = Brand.Radius.keycap
    }
}

struct KeyboardDeleteKey: UIViewRepresentable {
    let label: String
    let action: () -> Void
    var feedback: () -> Void = {}
    var prepareFeedback: () -> Void = {}
    var deleteWord: () -> Void = {}

    func makeUIView(context: Context) -> RepeatingDeleteButton { RepeatingDeleteButton() }

    func updateUIView(_ button: RepeatingDeleteButton, context: Context) {
        button.action = action
        button.feedback = feedback
        button.prepareFeedback = prepareFeedback
        button.deleteWord = deleteWord
        button.accessibilityLabel = label
        button.tintColor = UIColor(Brand.foreground.color)
        button.backgroundColor = UIColor(Brand.subtle.color)
        button.layer.cornerRadius = Brand.Radius.keycap
    }

    static func dismantleUIView(_ button: RepeatingDeleteButton, coordinator: ()) { button.stopRepeating() }
}

final class RepeatingDeleteButton: UIButton {
    var action: (() -> Void)?
    var feedback: (() -> Void)?
    var prepareFeedback: (() -> Void)?
    var deleteWord: (() -> Void)?
    private var repeatTimer: Timer?
    private var deletedWordSteps = 0

    override init(frame: CGRect) {
        super.init(frame: frame)
        setImage(UIImage(systemName: "delete.left"), for: .normal)
        addTarget(self, action: #selector(pressed), for: .touchDown)
        addTarget(self, action: #selector(released), for: [.touchUpInside, .touchUpOutside, .touchCancel, .touchDragExit])
        let pan = UIPanGestureRecognizer(target: self, action: #selector(dragged(_:)))
        pan.cancelsTouchesInView = false
        addGestureRecognizer(pan)
    }

    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil { stopRepeating() }
    }

    override func accessibilityActivate() -> Bool {
        prepareFeedback?()
        action?()
        feedback?()
        return true
    }

    @objc private func pressed() {
        stopRepeating()
        deletedWordSteps = 0
        prepareFeedback?()
        action?()
        feedback?()
        repeatTimer = Timer.scheduledTimer(timeInterval: 0.38, target: self, selector: #selector(startRepeating), userInfo: nil, repeats: false)
    }

    @objc private func startRepeating() {
        action?()
        repeatTimer = Timer.scheduledTimer(timeInterval: 0.075, target: self, selector: #selector(repeatDelete), userInfo: nil, repeats: true)
    }

    @objc private func repeatDelete() { action?() }
    @objc private func released() { stopRepeating() }

    @objc private func dragged(_ recognizer: UIPanGestureRecognizer) {
        if recognizer.state == .began { stopRepeating() }
        guard recognizer.state == .began || recognizer.state == .changed else { return }
        let steps = max(0, KeyboardGestureSteps.offset(translation: Double(-recognizer.translation(in: self).x), step: Double(max(bounds.width, 1))))
        if steps > deletedWordSteps {
            for _ in deletedWordSteps..<min(steps, deletedWordSteps + 16) { deleteWord?() }
            deletedWordSteps = steps
        }
    }

    func stopRepeating() {
        repeatTimer?.invalidate()
        repeatTimer = nil
    }
}

struct KeyboardCharacterKey: UIViewRepresentable {
    let text: String
    let alternatives: [String]
    let insert: (String) -> Void
    let showAlternatives: () -> Void
    var prepareFeedback: () -> Void = {}

    func makeUIView(context: Context) -> CharacterButton { CharacterButton(type: .custom) }

    func updateUIView(_ button: CharacterButton, context: Context) {
        button.character = text
        button.alternatives = alternatives
        button.insert = insert
        button.showAlternatives = showAlternatives
        button.prepareFeedback = prepareFeedback
        button.setTitle(text, for: .normal)
        button.setTitleColor(UIColor(Brand.foreground.color), for: .normal)
        button.backgroundColor = UIColor(Brand.field.color)
        button.layer.cornerRadius = Brand.Radius.keycap
        button.layer.shadowColor = UIColor(Brand.keyShadow.color).cgColor
        button.layer.shadowOpacity = 1
        button.layer.shadowRadius = 0
        button.layer.shadowOffset = CGSize(width: 0, height: 1)
        let font = UIFont(name: "Figtree-Medium", size: Brand.FontSize.title) ?? .systemFont(ofSize: Brand.FontSize.title, weight: .medium)
        button.titleLabel?.font = UIFontMetrics(forTextStyle: .title3).scaledFont(for: font, maximumPointSize: Brand.FontSize.title * 1.5)
        button.titleLabel?.adjustsFontForContentSizeCategory = true
        button.titleLabel?.adjustsFontSizeToFitWidth = true
        button.titleLabel?.minimumScaleFactor = 0.7
        button.accessibilityLabel = text
        button.accessibilityCustomActions = alternatives.map { value in
            UIAccessibilityCustomAction(name: value) { [weak button] _ in
                guard let button else { return false }
                button.insert?(value)
                return true
            }
        }
    }
}

final class CharacterButton: UIButton {
    var character = ""
    var alternatives: [String] = [] { didSet { holdRecognizer?.isEnabled = !alternatives.isEmpty } }
    var insert: ((String) -> Void)?
    var showAlternatives: (() -> Void)?
    var prepareFeedback: (() -> Void)?
    private var holdRecognizer: UILongPressGestureRecognizer?

    override init(frame: CGRect) {
        super.init(frame: frame)
        setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        addTarget(self, action: #selector(tapped), for: .touchUpInside)
        addTarget(self, action: #selector(pressed), for: .touchDown)
        let hold = UILongPressGestureRecognizer(target: self, action: #selector(held(_:)))
        hold.minimumPressDuration = 0.35
        hold.isEnabled = false
        addGestureRecognizer(hold)
        holdRecognizer = hold
    }

    required init?(coder: NSCoder) { nil }

    override var isHighlighted: Bool {
        didSet { alpha = isHighlighted ? 0.7 : 1 }
    }

    override func accessibilityActivate() -> Bool {
        insert?(character)
        return true
    }

    @objc private func tapped() { insert?(character) }
    @objc private func pressed() { prepareFeedback?() }

    @objc private func held(_ recognizer: UILongPressGestureRecognizer) {
        guard recognizer.state == .began, !alternatives.isEmpty else { return }
        showAlternatives?()
    }
}

struct KeyboardDictationGesture: UIViewRepresentable {
    let label: String
    var hint = ""
    var value = ""
    var enabled = true
    var holdEnabled = true
    let tap: () -> Void
    let holdBegan: () -> Void
    let holdEnded: () -> Void
    var prepareFeedback: () -> Void = {}

    func makeUIView(context: Context) -> DictationGestureButton { DictationGestureButton(type: .custom) }

    func updateUIView(_ button: DictationGestureButton, context: Context) {
        button.accessibilityLabel = label
        button.accessibilityHint = hint
        button.accessibilityValue = value
        button.isEnabled = enabled
        button.holdEnabled = holdEnabled
        button.tap = tap
        button.holdBegan = holdBegan
        button.holdEnded = holdEnded
        button.prepareFeedback = prepareFeedback
    }
}

final class DictationGestureButton: UIButton {
    var tap: (() -> Void)?
    var holdBegan: (() -> Void)?
    var holdEnded: (() -> Void)?
    var prepareFeedback: (() -> Void)?
    var holdEnabled = true {
        didSet {
            guard holdRecognizer?.state != .began, holdRecognizer?.state != .changed else { return }
            holdRecognizer?.isEnabled = holdEnabled
        }
    }
    private var holdRecognizer: UILongPressGestureRecognizer?

    override init(frame: CGRect) {
        super.init(frame: frame)
        addTarget(self, action: #selector(tapped), for: .touchUpInside)
        addTarget(self, action: #selector(pressed), for: .touchDown)
        let hold = UILongPressGestureRecognizer(target: self, action: #selector(held(_:)))
        hold.minimumPressDuration = 0.3
        hold.allowableMovement = 60
        addGestureRecognizer(hold)
        holdRecognizer = hold
    }

    required init?(coder: NSCoder) { nil }

    override func accessibilityActivate() -> Bool {
        guard isEnabled else { return false }
        tap?()
        return true
    }

    @objc private func tapped() { tap?() }
    @objc private func pressed() { prepareFeedback?() }

    @objc private func held(_ recognizer: UILongPressGestureRecognizer) {
        switch recognizer.state {
        case .began: holdBegan?()
        case .ended, .cancelled: holdEnded?()
        default: break
        }
    }
}
