import PennelMobileKit
import SwiftUI
import UIKit

final class KeyboardLetterGridView: UIView {
    private enum Kind: Equatable { case character(String), shift, layer, delete }
    private struct Key {
        let kind: Kind
        let row: Int
        let column: Int
        let button: AccessibleKeyboardButton
        var frame: CGRect = .zero
    }
    private enum Mode: Equatable { case tap, swipe, accent, delete }
    private final class Contact {
        let initial: Int
        let start: CGPoint
        let beganAt: TimeInterval
        var point: CGPoint
        var mode: Mode = .tap
        var path: [KeyboardSwipePoint] = []
        var distance: CGFloat = 0
        var wordSteps = 0
        var holdCancelled = false
        init(initial: Int, point: CGPoint, at time: TimeInterval) {
            self.initial = initial; start = point; self.point = point; beganAt = time
        }
    }
    private var keys: [Key] = []
    private var contacts: [ObjectIdentifier: Contact] = [:]
    private var rows: [[String]] = []
    private var shifted = false
    private var shiftLocked = false
    private var keyboardLayer = KeyboardLayer.letters
    private var keyHeight = Brand.Spacing.s10 + Brand.Spacing.s1
    private var swipeEnabled = false
    private var copy = KeyboardCopy(language: .english)
    private var alternatives: (String) -> [String] = { _ in [] }
    private var insert: (String) -> Void = { _ in }
    private var delete: () -> Void = {}
    private var deleteWord: () -> Void = {}
    private var swipe: (KeyboardSwipeGesture) -> Void = { _ in }
    private var shift: () -> Void = {}
    private var changeLayer: () -> Void = {}
    private var feedback: (FeedbackEvent) -> Void = { _ in }
    private var prepareFeedback: () -> Void = {}
    private var recordTouch: (KeyboardSwipeGesture) -> Void = { _ in }
    private var holdTimer: Timer?
    private var repeatTimer: Timer?
    private var repeatedContact: ObjectIdentifier?
    private var accentOwner: ObjectIdentifier?
    private var accentValues: [String] = []
    private var accentButtons: [AccessibleKeyboardButton] = []
    private var accentSelection = 0
    private let accentPanel = UIView()
    private var externalHighlight: Int?
    private var highlightedKeys: Set<Int> = []

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
        backgroundColor = .clear
        accentPanel.layer.cornerRadius = Brand.Radius.keycap
        accentPanel.backgroundColor = UIColor(Brand.subtle.color)
        accentPanel.layer.shadowColor = UIColor(Brand.keyShadow.color).cgColor
        accentPanel.layer.shadowOffset = CGSize(width: 0, height: 1)
        accentPanel.layer.shadowOpacity = 1
        accentPanel.isHidden = true
        addSubview(accentPanel)
        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (view: KeyboardLetterGridView, _: UITraitCollection) in
            view.updateKeyAppearance()
            view.setNeedsLayout()
        }
    }
    required init?(coder: NSCoder) { nil }

    func configure(rows: [[String]], shifted: Bool, shiftLocked: Bool, layer: KeyboardLayer,
                   keyHeight: CGFloat, swipeEnabled: Bool, copy: KeyboardCopy,
                   alternatives: @escaping (String) -> [String], insert: @escaping (String) -> Void,
                   delete: @escaping () -> Void, deleteWord: @escaping () -> Void,
                   swipe: @escaping (KeyboardSwipeGesture) -> Void, shift: @escaping () -> Void,
                   changeLayer: @escaping () -> Void, feedback: @escaping (FeedbackEvent) -> Void,
                   prepareFeedback: @escaping () -> Void, recordTouch: @escaping (KeyboardSwipeGesture) -> Void = { _ in }) {
        let changed = self.rows != rows || self.keyboardLayer != layer
        let appearanceChanged = changed || self.shifted != shifted || self.shiftLocked != shiftLocked
            || self.keyHeight != keyHeight || self.copy.language != copy.language
        self.rows = rows; self.shifted = shifted; self.shiftLocked = shiftLocked; self.keyboardLayer = layer
        self.keyHeight = keyHeight; self.swipeEnabled = swipeEnabled; self.copy = copy
        self.alternatives = alternatives; self.insert = insert; self.delete = delete
        self.deleteWord = deleteWord; self.swipe = swipe; self.shift = shift; self.changeLayer = changeLayer
        self.feedback = feedback; self.prepareFeedback = prepareFeedback
        self.recordTouch = recordTouch
        if changed { rebuildKeys() }
        if appearanceChanged { updateKeyAppearance(); setNeedsLayout() }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let gap = Brand.Spacing.s1
        for row in 0..<min(3, rows.count) {
            let indices = keys.indices.filter { keys[$0].row == row }
            let inset = row == 1 ? Brand.Spacing.s3 : 0
            let available = max(0, bounds.width - inset * 2)
            let letters = rows[row].count
            let side = min(keyHeight, available / 4)
            let width = row == 2 ? max(0, (available - side * 2 - gap * 2) / CGFloat(max(letters, 1))) : available / CGFloat(max(letters, 1))
            for index in indices {
                let column = keys[index].column
                let x: CGFloat
                let cellWidth: CGFloat
                if row == 2, column == -1 { x = inset; cellWidth = side }
                else if row == 2, column == letters { x = bounds.width - inset - side; cellWidth = side }
                else { x = inset + (row == 2 ? side + gap : 0) + CGFloat(column) * width; cellWidth = width }
                let hit = CGRect(x: x, y: CGFloat(row) * (keyHeight + gap), width: cellWidth, height: keyHeight)
                keys[index].frame = hit
                keys[index].button.frame = hit.insetBy(dx: gap / 2, dy: 0)
            }
        }
        bringSubviewToFront(accentPanel)
    }

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard !isHidden, alpha > 0.01, isUserInteractionEnabled, bounds.contains(point) else { return nil }
        return self
    }
    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil { cancelInteractions() }
    }
    func character(at point: CGPoint) -> String? {
        guard bounds.contains(point), let index = nearestKey(at: point), case .character(let value) = keys[index].kind else { return nil }
        return displayed(value)
    }
    func highlightExternal(at point: CGPoint?) {
        externalHighlight = point.flatMap { bounds.contains($0) ? nearestKey(at: $0) : nil }
        updateHighlights()
    }
    func cancelInteractions() {
        holdTimer?.invalidate(); holdTimer = nil
        repeatTimer?.invalidate(); repeatTimer = nil; repeatedContact = nil
        contacts = [:]
        hideAccents()
        externalHighlight = nil
        updateHighlights()
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches { beginContact(touch, at: touch.location(in: self), time: touch.timestamp) }
    }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches { moveContact(touch, to: touch.location(in: self)) }
    }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches { endContact(touch, at: touch.location(in: self)) }
    }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches { cancelContact(touch) }
    }

    func beginContact(_ token: AnyObject, at point: CGPoint, time: TimeInterval = ProcessInfo.processInfo.systemUptime) {
        guard bounds.contains(point), contacts.count < 10, let index = nearestKey(at: point) else { return }
        let id = ObjectIdentifier(token)
        let contact = Contact(initial: index, point: point, at: time)
        contact.path = [normalized(point)]
        contacts[id] = contact
        prepareFeedback()
        if keys[index].kind == .delete {
            contact.mode = .delete
            delete(); feedback(.keyPress)
            repeatedContact = id
            repeatTimer?.invalidate()
            repeatTimer = Timer.scheduledTimer(timeInterval: 0.38, target: self, selector: #selector(startRepeating), userInfo: nil, repeats: false)
        }
        scheduleHold(); updateHighlights()
    }

    func moveContact(_ token: AnyObject, to point: CGPoint) {
        let id = ObjectIdentifier(token)
        guard let contact = contacts[id] else { return }
        contact.distance += hypot(point.x - contact.point.x, point.y - contact.point.y)
        contact.point = point
        switch contact.mode {
        case .delete:
            let steps = max(0, KeyboardGestureSteps.offset(translation: Double(contact.start.x - point.x), step: Double(keyHeight)))
            if steps > contact.wordSteps {
                repeatTimer?.invalidate(); repeatTimer = nil
                for _ in contact.wordSteps..<min(steps, contact.wordSteps + 16) { deleteWord() }
                contact.wordSteps = steps
            }
        case .accent:
            if accentOwner == id { updateAccentSelection(at: point, from: contact.start) }
        case .tap, .swipe:
            if contact.distance > keyHeight / 4 { contact.holdCancelled = true }
            if case .character = keys[contact.initial].kind, keyboardLayer == .letters, swipeEnabled,
               contacts.count == 1, contact.distance > keyHeight * 0.7 { contact.mode = .swipe }
            append(point, to: contact)
        }
        updateHighlights()
    }

    func endContact(_ token: AnyObject, at point: CGPoint) {
        let id = ObjectIdentifier(token)
        guard let contact = contacts.removeValue(forKey: id) else { return }
        switch contact.mode {
        case .delete: stopRepeating(id)
        case .accent:
            if accentOwner == id {
                let value = accentValues.indices.contains(accentSelection) && bounds.contains(point) ? accentValues[accentSelection] : nil
                hideAccents()
                if let value { insert(value) }
            }
        case .swipe:
            if bounds.contains(point) {
                append(point, to: contact)
                swipe(KeyboardSwipeGesture(points: contact.path, keys: swipeKeys()))
            }
        case .tap:
            if bounds.contains(point), let index = nearestKey(at: point) {
                if case .character = keys[contact.initial].kind {
                    if case .character(let value) = keys[index].kind {
                        if keyboardLayer == .letters { recordTouch(KeyboardSwipeGesture(points: [normalized(point)], keys: swipeKeys())) }
                        insert(displayed(value))
                    }
                } else if index == contact.initial { activate(index) }
            }
        }
        scheduleHold(); updateHighlights()
    }

    func cancelContact(_ token: AnyObject) {
        let id = ObjectIdentifier(token)
        contacts.removeValue(forKey: id); stopRepeating(id)
        if accentOwner == id { hideAccents() }
        scheduleHold(); updateHighlights()
    }

    private func rebuildKeys() {
        cancelInteractions()
        keys.forEach { $0.button.removeFromSuperview() }
        keys = []
        for (row, values) in rows.prefix(3).enumerated() {
            if row == 2 { addKey(keyboardLayer == .letters ? .shift : .layer, row: row, column: -1) }
            for (column, value) in values.enumerated() { addKey(.character(value), row: row, column: column) }
            if row == 2 { addKey(.delete, row: row, column: values.count) }
        }
        accessibilityElements = keys.map(\.button)
    }
    private func addKey(_ kind: Kind, row: Int, column: Int) {
        let button = AccessibleKeyboardButton(type: .custom)
        let index = keys.count
        button.activate = { [weak self] in self?.activate(index) }
        button.layer.cornerRadius = Brand.Radius.keycap
        button.layer.shadowColor = UIColor(Brand.keyShadow.color).cgColor
        button.layer.shadowOpacity = 1
        button.layer.shadowRadius = 0
        button.layer.shadowOffset = CGSize(width: 0, height: 1)
        addSubview(button)
        keys.append(Key(kind: kind, row: row, column: column, button: button))
    }
    private func updateKeyAppearance() {
        for key in keys {
            let button = key.button
            button.setTitle(nil, for: .normal); button.setImage(nil, for: .normal)
            button.setTitleColor(UIColor(Brand.foreground.color), for: .normal)
            button.tintColor = UIColor(Brand.foreground.color)
            let font = UIFont(name: "Figtree-Medium", size: Brand.FontSize.title) ?? .systemFont(ofSize: Brand.FontSize.title, weight: .medium)
            button.titleLabel?.font = UIFontMetrics(forTextStyle: .title3).scaledFont(for: font, maximumPointSize: Brand.FontSize.title * 1.5)
            button.titleLabel?.adjustsFontForContentSizeCategory = true
            button.titleLabel?.adjustsFontSizeToFitWidth = true
            button.titleLabel?.minimumScaleFactor = 0.7
            button.accessibilityTraits = [.keyboardKey]
            button.accessibilityCustomActions = nil
            let base = key.kind == .shift || key.kind == .layer || key.kind == .delete ? Brand.subtle : Brand.field
            button.backgroundColor = UIColor(base.color)
            switch key.kind {
            case .character(let value):
                button.setTitle(displayed(value), for: .normal)
                button.accessibilityLabel = displayed(value)
                button.accessibilityCustomActions = alternatives(value).map { value in
                    UIAccessibilityCustomAction(name: value) { [weak self] _ in self?.prepareFeedback(); self?.insert(value); return self != nil }
                }
            case .shift:
                button.setImage(UIImage(systemName: shiftLocked ? "capslock.fill" : shifted ? "shift.fill" : "shift"), for: .normal)
                button.accessibilityLabel = copy.text(shiftLocked ? .capsLock : .shift)
            case .layer:
                button.setTitle(copy.text(keyboardLayer == .numbers ? .symbols : .numbers), for: .normal)
                button.accessibilityLabel = copy.text(keyboardLayer == .numbers ? .showSymbols : .showNumbers)
            case .delete:
                button.setImage(UIImage(systemName: "delete.left"), for: .normal)
                button.accessibilityLabel = copy.text(.delete)
                button.accessibilityCustomActions = [UIAccessibilityCustomAction(name: copy.language == .dutch ? "Verwijder woord" : "Delete word") { [weak self] _ in
                    self?.prepareFeedback(); self?.deleteWord(); self?.feedback(.keyPress); return self != nil
                }]
            }
        }
        highlightedKeys = []
        updateHighlights()
    }
    private func activate(_ index: Int) {
        guard keys.indices.contains(index) else { return }
        prepareFeedback()
        switch keys[index].kind {
        case .character(let value): insert(displayed(value))
        case .shift: shift()
        case .layer: changeLayer()
        case .delete: delete(); feedback(.keyPress)
        }
    }
    private func displayed(_ value: String) -> String { keyboardLayer == .letters && shifted ? value.uppercased() : value }
    private func nearestKey(at point: CGPoint) -> Int? {
        guard !keys.isEmpty else { return nil }
        return keys.indices.min { distance(point, to: keys[$0].frame) < distance(point, to: keys[$1].frame) }
    }
    private func distance(_ point: CGPoint, to frame: CGRect) -> CGFloat {
        let dx = max(frame.minX - point.x, 0, point.x - frame.maxX)
        let dy = max(frame.minY - point.y, 0, point.y - frame.maxY)
        return dx * dx + dy * dy
    }
    private func normalized(_ point: CGPoint) -> KeyboardSwipePoint {
        KeyboardSwipePoint(x: Double(max(0, min(1, point.x / max(1, bounds.width)))),
            y: Double(max(0, min(1, point.y / max(1, bounds.height)))))
    }
    private func append(_ point: CGPoint, to contact: Contact) {
        let value = normalized(point)
        if let last = contact.path.last, hypot(value.x - last.x, value.y - last.y) < 0.003 { return }
        if contact.path.count >= 256 { contact.path = contact.path.enumerated().filter { $0.offset.isMultiple(of: 2) }.map(\.element) }
        contact.path.append(value)
    }
    private func swipeKeys() -> [KeyboardSwipeKey] {
        keys.compactMap { key in
            guard case .character(let value) = key.kind else { return nil }
            let point = normalized(CGPoint(x: key.frame.midX, y: key.frame.midY))
            return KeyboardSwipeKey(key: value, x: point.x, y: point.y)
        }
    }
    private func updateHighlights() {
        var active = Set(contacts.values.compactMap { bounds.contains($0.point) ? nearestKey(at: $0.point) : nil })
        if let externalHighlight { active.insert(externalHighlight) }
        guard active != highlightedKeys else { return }
        let changed = active.symmetricDifference(highlightedKeys)
        for index in changed where keys.indices.contains(index) {
            let key = keys[index]
            let base = key.kind == .shift || key.kind == .layer || key.kind == .delete ? Brand.subtle : Brand.field
            key.button.backgroundColor = UIColor(active.contains(index) ? Brand.selection.color : base.color)
        }
        highlightedKeys = active
    }
    private func scheduleHold() {
        holdTimer?.invalidate(); holdTimer = nil
        guard accentOwner == nil,
              let item = contacts.first(where: { _, contact in
                  guard contact.mode == .tap, !contact.holdCancelled, case .character(let value) = keys[contact.initial].kind else { return false }
                  return !alternatives(value).isEmpty
              }) else { return }
        let delay = max(0.01, 0.35 - (ProcessInfo.processInfo.systemUptime - item.value.beganAt))
        holdTimer = Timer.scheduledTimer(timeInterval: delay, target: self, selector: #selector(showHeldAccent), userInfo: nil, repeats: false)
    }
    @objc func showHeldAccent() {
        guard accentOwner == nil,
              let (id, contact) = contacts.first(where: { _, value in
                  guard value.mode == .tap, !value.holdCancelled, case .character(let character) = keys[value.initial].kind else { return false }
                  return !alternatives(character).isEmpty
              }),
              case .character(let value) = keys[contact.initial].kind, !alternatives(value).isEmpty else { return }
        contact.mode = .accent; accentOwner = id
        accentValues = [displayed(value)] + alternatives(value); accentSelection = 0
        accentButtons.forEach { $0.removeFromSuperview() }; accentButtons = []
        let width = min(bounds.width, CGFloat(accentValues.count) * keyHeight)
        let key = keys[contact.initial].frame
        accentPanel.frame = CGRect(x: max(0, min(bounds.width - width, key.midX - keyHeight / 2)),
            y: max(0, key.minY - keyHeight - Brand.Spacing.s1), width: width, height: keyHeight)
        for (index, value) in accentValues.enumerated() {
            let button = AccessibleKeyboardButton(type: .custom)
            button.setTitle(value, for: .normal)
            button.setTitleColor(UIColor(Brand.foreground.color), for: .normal)
            button.titleLabel?.font = UIFont(name: "Figtree-Medium", size: Brand.FontSize.title)
            button.frame = CGRect(x: CGFloat(index) * width / CGFloat(accentValues.count), y: 0, width: width / CGFloat(accentValues.count), height: keyHeight)
            button.layer.cornerRadius = Brand.Radius.keycap
            button.accessibilityLabel = value; button.accessibilityTraits = [.keyboardKey]
            button.activate = { [weak self] in self?.hideAccents(); self?.insert(value) }
            accentPanel.addSubview(button); accentButtons.append(button)
        }
        accentPanel.isHidden = false; bringSubviewToFront(accentPanel)
        accessibilityElements = accentButtons
        updateAccentAppearance(); feedback(.selection)
    }
    private func updateAccentSelection(at point: CGPoint, from start: CGPoint) {
        guard !accentValues.isEmpty, abs(point.x - start.x) > Brand.Spacing.s1 else { return }
        let fraction = (point.x - accentPanel.frame.minX) / max(1, accentPanel.frame.width)
        let next = max(0, min(accentValues.count - 1, Int(fraction * CGFloat(accentValues.count))))
        if next != accentSelection { accentSelection = next; updateAccentAppearance(); feedback(.selection) }
    }
    private func updateAccentAppearance() {
        for (index, button) in accentButtons.enumerated() { button.backgroundColor = UIColor(index == accentSelection ? Brand.selection.color : Brand.subtle.color) }
    }
    private func hideAccents() {
        accentOwner = nil; accentValues = []; accentSelection = 0
        accentPanel.isHidden = true; accessibilityElements = keys.map(\.button)
    }
    @objc func startRepeating() {
        repeatTimer?.invalidate(); repeatTimer = nil
        guard let repeatedContact, contacts[repeatedContact]?.mode == .delete else { return }
        delete()
        repeatTimer = Timer.scheduledTimer(timeInterval: 0.075, target: self, selector: #selector(repeatDelete), userInfo: nil, repeats: true)
    }
    @objc func repeatDelete() {
        guard let repeatedContact, let contact = contacts[repeatedContact], contact.wordSteps == 0, bounds.contains(contact.point) else { return }
        delete()
    }
    private func stopRepeating(_ id: ObjectIdentifier) {
        if repeatedContact == id { repeatTimer?.invalidate(); repeatTimer = nil; repeatedContact = nil }
    }
}

final class AccessibleKeyboardButton: UIButton {
    var activate: (() -> Void)?
    override func accessibilityActivate() -> Bool { activate?(); return activate != nil }
}
