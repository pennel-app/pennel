import Foundation
import PennelMobileKit
import Testing
import UIKit

@MainActor
struct KeyboardInteractionTests {
    @Test func taps_fill_key_gaps_and_commit_once_after_preparation() {
        let harness = InteractionHarness()
        let touch = NSObject()
        harness.grid.beginContact(touch, at: CGPoint(x: 29, y: 20))
        #expect(harness.preparations == 1)
        #expect(harness.inserted.isEmpty)
        harness.grid.endContact(touch, at: CGPoint(x: 29, y: 20))
        #expect(harness.inserted == ["q"])
        #expect(harness.recorded.count == 1)
        #expect(harness.recorded.first?.keys.count == 26)
        #expect(harness.recorded.first?.points.first?.x == 29.0 / 300.0)
        harness.grid.endContact(touch, at: CGPoint(x: 29, y: 20))
        #expect(harness.inserted == ["q"])
        harness.grid.cancelInteractions()
    }

    @Test func cancelled_and_released_outside_contacts_never_commit() {
        let harness = InteractionHarness()
        let first = NSObject(), second = NSObject()
        harness.grid.beginContact(first, at: CGPoint(x: 15, y: 20))
        harness.grid.cancelContact(first)
        harness.grid.beginContact(second, at: CGPoint(x: 15, y: 20))
        harness.grid.endContact(second, at: CGPoint(x: -20, y: 20))
        #expect(harness.inserted.isEmpty)
        #expect(harness.feedback.isEmpty)
        #expect(harness.recorded.isEmpty)
        harness.grid.cancelInteractions()
    }

    @Test func concurrent_thumb_contacts_keep_independent_commit_destinations() {
        let harness = InteractionHarness()
        let first = NSObject(), second = NSObject()
        harness.grid.beginContact(first, at: CGPoint(x: 15, y: 20))
        harness.grid.beginContact(second, at: CGPoint(x: 285, y: 20))
        harness.grid.endContact(second, at: CGPoint(x: 285, y: 20))
        harness.grid.endContact(first, at: CGPoint(x: 15, y: 20))
        #expect(harness.inserted == ["p", "q"])
        #expect(harness.swipes.isEmpty)
        harness.grid.cancelInteractions()
    }

    @Test func a_swipe_delivers_actual_geometry_and_no_partial_letters() {
        let harness = InteractionHarness(swipeEnabled: true)
        let touch = NSObject()
        harness.grid.beginContact(touch, at: CGPoint(x: 15, y: 20))
        harness.grid.moveContact(touch, to: CGPoint(x: 50, y: 20))
        harness.grid.moveContact(touch, to: CGPoint(x: 100, y: 70))
        harness.grid.endContact(touch, at: CGPoint(x: 140, y: 70))
        #expect(harness.inserted.isEmpty)
        #expect(harness.swipes.count == 1)
        #expect(harness.swipes[0].keys.contains { $0.key == "q" && $0.x == 0.05 })
        #expect(harness.swipes[0].points.first?.x == 0.05)
        #expect(harness.swipes[0].points.last?.x == 140.0 / 300.0)
        #expect(harness.feedback.isEmpty)
        harness.grid.cancelInteractions()
    }

    @Test func held_delete_repeats_without_additional_pulses_and_stops_on_cancel() {
        let harness = InteractionHarness()
        let touch = NSObject()
        harness.grid.beginContact(touch, at: CGPoint(x: 285, y: 120))
        #expect(harness.deleted == 1)
        harness.grid.startRepeating()
        harness.grid.repeatDelete()
        #expect(harness.deleted == 3)
        #expect(harness.feedback == [.keyPress])
        harness.grid.cancelContact(touch)
        harness.grid.repeatDelete()
        #expect(harness.deleted == 3)
        harness.grid.cancelInteractions()
    }

    @Test func accents_stay_within_the_grid_and_commit_only_at_release() {
        let harness = InteractionHarness()
        let touch = NSObject()
        harness.grid.beginContact(touch, at: CGPoint(x: 75, y: 20))
        harness.grid.showHeldAccent()
        #expect(harness.inserted.isEmpty)
        #expect(harness.feedback == [.selection])
        harness.grid.moveContact(touch, to: CGPoint(x: 120, y: 20))
        harness.grid.endContact(touch, at: CGPoint(x: 120, y: 20))
        #expect(harness.inserted == ["è"])
        #expect(harness.grid.bounds.height == 140)
        harness.grid.cancelInteractions()
    }

    @Test func both_layouts_export_measured_key_centers_at_different_sizes() throws {
        for layout in [KeyboardLayout.qwerty, .azerty] {
            for width in [CGFloat(300), CGFloat(650)] {
                for height in [CGFloat(40), CGFloat(72)] {
                    let harness = InteractionHarness(layout: layout, width: width, keyHeight: height)
                    let touch = NSObject()
                    let point = CGPoint(x: width * 0.05, y: height / 2)
                    harness.grid.beginContact(touch, at: point)
                    harness.grid.endContact(touch, at: point)
                    let keys = try #require(harness.recorded.first?.keys)
                    #expect(keys.count == 26)
                    #expect(Set(keys.map(\.key)).count == 26)
                    #expect(keys.allSatisfy { (0...1).contains($0.x) && (0...1).contains($0.y) })
                    let secondRowLetter = layout.rows[1][0]
                    let actual = try #require(keys.first { $0.key == secondRowLetter })
                    let expectedX = Double((12 + (width - 24) / CGFloat(layout.rows[1].count) / 2) / width)
                    let expectedY = Double((height + 4 + height / 2) / (height * 3 + 8))
                    #expect(abs(actual.x - expectedX) < 0.000_001)
                    #expect(abs(actual.y - expectedY) < 0.000_001)
                    harness.grid.cancelInteractions()
                }
            }
        }
    }
}

@MainActor
private final class InteractionHarness {
    let grid: KeyboardLetterGridView
    var preparations = 0
    var inserted: [String] = []
    var feedback: [FeedbackEvent] = []
    var recorded: [KeyboardSwipeGesture] = []
    var swipes: [KeyboardSwipeGesture] = []
    var deleted = 0

    init(layout: KeyboardLayout = .qwerty, swipeEnabled: Bool = false, width: CGFloat = 300, keyHeight: CGFloat = 44) {
        grid = KeyboardLetterGridView(frame: CGRect(x: 0, y: 0, width: width, height: keyHeight * 3 + 8))
        let state = KeyboardTypingState()
        grid.configure(rows: state.rows(layout: layout), shifted: false, shiftLocked: false,
            layer: .letters, keyHeight: keyHeight, swipeEnabled: swipeEnabled, copy: KeyboardCopy(language: .english),
            alternatives: { state.alternatives(for: $0) }, insert: { [weak self] in self?.inserted.append($0) },
            delete: { [weak self] in self?.deleted += 1 }, deleteWord: {},
            swipe: { [weak self] in self?.swipes.append($0) }, shift: {}, changeLayer: {},
            feedback: { [weak self] in self?.feedback.append($0) },
            prepareFeedback: { [weak self] in self?.preparations += 1 },
            recordTouch: { [weak self] in self?.recorded.append($0) })
        grid.layoutIfNeeded()
    }
}
