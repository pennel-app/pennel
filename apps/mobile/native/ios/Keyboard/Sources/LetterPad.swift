import PennelMobileKit
import SwiftUI
import UIKit

@MainActor
final class KeyboardLetterInputTarget {
    weak var grid: KeyboardLetterGridView?
    func character(at point: CGPoint, from source: UIView) -> String? {
        guard let grid else { return nil }
        return grid.character(at: grid.convert(point, from: source))
    }
    func highlight(at point: CGPoint, from source: UIView) {
        guard let grid else { return }
        grid.highlightExternal(at: grid.convert(point, from: source))
    }
    func clearHighlight() { grid?.highlightExternal(at: nil) }
}

struct LetterPad: View {
    let layout: KeyboardLayout
    let typing: KeyboardTypingModel
    let insert: (String) -> Void
    let delete: () -> Void
    let feedback: (FeedbackEvent) -> Void
    var prepareFeedback: () -> Void = {}
    var deleteWord: () -> Void = {}
    var swipe: (KeyboardSwipeGesture) -> Void = { _ in }
    var inputTarget: KeyboardLetterInputTarget? = nil
    var allowSwipe = true
    var recordTouch: (KeyboardSwipeGesture) -> Void = { _ in }
    var stateOverride: Binding<KeyboardTypingState>? = nil
    @ScaledMetric(relativeTo: .body) private var scaledKeyHeight = Brand.Spacing.s10 + Brand.Spacing.s1
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    private var keyHeight: CGFloat {
        if verticalSizeClass == .compact { return max(Brand.Spacing.s8, min(scaledKeyHeight, Brand.Spacing.s10)) }
        return max(Brand.Spacing.s10 + Brand.Spacing.s1, min(scaledKeyHeight, Brand.Spacing.s10 + Brand.Spacing.s8))
    }
    var body: some View {
        KeyboardLetterGrid(layout: layout, typing: typing, keyHeight: keyHeight, insert: insert,
            delete: delete, deleteWord: deleteWord, swipe: swipe, feedback: feedback,
            prepareFeedback: prepareFeedback, inputTarget: inputTarget, allowSwipe: allowSwipe,
            recordTouch: recordTouch, stateOverride: stateOverride)
            .frame(height: keyHeight * 3 + Brand.Spacing.s1 * 2)
    }
}

private struct KeyboardLetterGrid: UIViewRepresentable {
    let layout: KeyboardLayout
    let typing: KeyboardTypingModel
    let keyHeight: CGFloat
    let insert: (String) -> Void
    let delete: () -> Void
    let deleteWord: () -> Void
    let swipe: (KeyboardSwipeGesture) -> Void
    let feedback: (FeedbackEvent) -> Void
    let prepareFeedback: () -> Void
    let inputTarget: KeyboardLetterInputTarget?
    let allowSwipe: Bool
    let recordTouch: (KeyboardSwipeGesture) -> Void
    let stateOverride: Binding<KeyboardTypingState>?
    final class Coordinator { var usesLocalState = false }
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeUIView(context: Context) -> KeyboardLetterGridView { KeyboardLetterGridView() }
    func updateUIView(_ view: KeyboardLetterGridView, context: Context) {
        let usesLocalState = stateOverride != nil
        if usesLocalState != context.coordinator.usesLocalState {
            view.cancelInteractions(); context.coordinator.usesLocalState = usesLocalState
        }
        let state = stateOverride?.wrappedValue ?? typing.state
        inputTarget?.grid = view
        view.configure(rows: state.rows(layout: layout), shifted: state.shift != .off,
            shiftLocked: state.shift == .locked, layer: state.layer, keyHeight: keyHeight,
            swipeEnabled: typing.swipeEnabled && allowSwipe, copy: typing.copy,
            alternatives: { (stateOverride?.wrappedValue ?? typing.state).alternatives(for: $0) }, insert: insert, delete: delete,
            deleteWord: deleteWord, swipe: swipe,
            shift: {
                if let stateOverride {
                    var value = stateOverride.wrappedValue; value.pressShift(at: Date()); stateOverride.wrappedValue = value
                } else { typing.state.pressShift(at: Date()) }
                feedback(.keyPress)
            },
            changeLayer: {
                if let stateOverride {
                    var value = stateOverride.wrappedValue; value.layer = value.layer == .numbers ? .symbols : .numbers; stateOverride.wrappedValue = value
                } else { typing.state.layer = typing.state.layer == .numbers ? .symbols : .numbers }
                feedback(.keyPress)
            }, feedback: feedback, prepareFeedback: prepareFeedback, recordTouch: recordTouch)
    }
    static func dismantleUIView(_ view: KeyboardLetterGridView, coordinator: Coordinator) { view.cancelInteractions() }
}
