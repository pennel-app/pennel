import XCTest
@testable import PennelMacPlatform

final class ShortcutMatcherTests: XCTestCase {
    private let control: UInt64 = 0x40000
    private let option: UInt64 = 0x80000
    private let fn: UInt64 = 0x800000

    func testSharedRustSequencesMatchTheNativeMatcher() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("assets/contracts/shortcut-sequences.json"))
        let cases = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        let masks: [String: UInt64] = ["ctrl":0x40000, "alt":0x80000, "fn":0x800000, "left_alt":0x20]
        func binding(_ value: [String: Any], action: UInt32 = 0) -> NativeShortcut {
            NativeShortcut(action: action, code: (value["key"] as? NSNumber)?.int64Value, mods: (value["mods"] as? [String] ?? []).map { masks[$0]! })
        }
        for value in cases {
            let hold = (value["hold"] as? [String: Any]).map { binding($0) }
            let listening = (value["listening"] as? [[String: Any]] ?? []).map { binding($0, action: ($0["action"] as! NSNumber).uint32Value) }
            var matcher = ShortcutMatcher(table: ShortcutTable(hold: hold, always: [], listening: listening))
            matcher.setListening(true)
            for event in value["events"] as! [[String: Any]] {
                let kind: ShortcutInput = ["flags":.flags, "down":.down, "up":.up, "mouse":.mouse][event["kind"] as! String]!
                let result = matcher.route(kind, code: (event["code"] as! NSNumber).int64Value, flags: (event["flags"] as! NSNumber).uint64Value, repeating: event["repeating"] as! Bool)
                let signals = result.signals.map { signal -> String in
                    switch signal { case .down: return "down"; case .up: return "up"; case .interrupt: return "interrupt"; case .shift(let down): return "shift:\(down ? 1 : 0)"; case .action(let id): return "action:\(id)" }
                }
                XCTAssertEqual(signals, event["signals"] as! [String], "\(value["name"]!) \(event)")
                XCTAssertEqual(result.withhold, event["withhold"] as! Bool)
            }
        }
    }

    func testKeyedHoldLatchesThroughRepeatsAndModifierFirstRelease() {
        var matcher = ShortcutMatcher(table: ShortcutTable(hold: NativeShortcut(action: 0, code: 49, mods: [control, option]), always: [], listening: []))
        XCTAssertEqual(matcher.route(.flags, code: 59, flags: control).signals, [])
        _ = matcher.route(.flags, code: 58, flags: control | option)
        let first = matcher.route(.down, code: 49, flags: control | option)
        XCTAssertEqual(first.signals, [.down]); XCTAssertTrue(first.withhold)
        let repeated = matcher.route(.down, code: 49, flags: control | option, repeating: true)
        XCTAssertEqual(repeated.signals, []); XCTAssertTrue(repeated.withhold)
        XCTAssertEqual(matcher.route(.down, code: 0, flags: control | option).signals, [])
        XCTAssertEqual(matcher.route(.flags, code: 59, flags: option).signals, [.up])
        XCTAssertTrue(matcher.route(.up, code: 49, flags: option).withhold)
    }

    func testListeningActionsAreWithheldOnlyWhileListeningAndCancelDoesNotInterrupt() {
        var matcher = ShortcutMatcher(table: ShortcutTable(hold: NativeShortcut(action: 0, code: nil, mods: [fn]), always: [], listening: [NativeShortcut(action: 3, code: 53, mods: [])]))
        XCTAssertFalse(matcher.route(.down, code: 53, flags: 0).withhold)
        _ = matcher.route(.up, code: 53, flags: 0)
        matcher.setListening(true)
        _ = matcher.route(.flags, code: 63, flags: fn)
        let cancel = matcher.route(.down, code: 53, flags: fn)
        XCTAssertTrue(cancel.withhold); XCTAssertEqual(cancel.signals, [.action(3)])
        matcher.setListening(false)
        XCTAssertTrue(matcher.route(.up, code: 53, flags: 0).withhold)
        XCTAssertFalse(matcher.route(.down, code: 53, flags: 0).withhold)
    }

    func testSideSpecificBareModifierNeedsACleanReleaseAndMouseCancelsIt() {
        var matcher = ShortcutMatcher(table: ShortcutTable(hold: nil, always: [], listening: [NativeShortcut(action: 4, code: nil, mods: [0x20])]))
        matcher.setListening(true)
        _ = matcher.route(.flags, code: 61, flags: option | 0x40)
        XCTAssertEqual(matcher.route(.flags, code: 61, flags: 0).signals, [])
        _ = matcher.route(.flags, code: 58, flags: option | 0x20)
        XCTAssertEqual(matcher.route(.flags, code: 58, flags: 0).signals, [.action(4)])
        _ = matcher.route(.flags, code: 58, flags: option | 0x20)
        _ = matcher.route(.mouse, code: 0, flags: option | 0x20)
        XCTAssertEqual(matcher.route(.flags, code: 58, flags: 0).signals, [])
    }

    func testCaptureWithholdsEveryKeyboardEventAndKeepsReleaseLatchAfterStop() {
        var matcher = ShortcutMatcher(table: ShortcutTable(hold: NativeShortcut(action: 0, code: nil, mods: [fn]), always: [NativeShortcut(action: 1, code: 15, mods: [0x100000, 0x20000])], listening: []))
        matcher.setCapture(true)
        XCTAssertTrue(matcher.route(.flags, code: 63, flags: fn).withhold)
        XCTAssertEqual(matcher.route(.down, code: 15, flags: 0x120000).signals, [])
        matcher.setCapture(false)
        XCTAssertTrue(matcher.route(.up, code: 15, flags: 0).withhold)
        XCTAssertTrue(matcher.route(.flags, code: 63, flags: 0).withhold)
    }
}
