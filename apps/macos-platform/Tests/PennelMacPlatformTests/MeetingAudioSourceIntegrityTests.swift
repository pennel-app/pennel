import CoreAudio
import XCTest
@testable import PennelMacPlatform

final class MeetingAudioSourceIntegrityTests: XCTestCase {
    func testAudioClockUsesTheTrackUptimeEpochAndFreezesAtHardwareStop() {
        var clock = MeetingAudioClock()
        XCTAssertNil(clock.elapsedMilliseconds(at: 100))
        clock.start(at: 102)
        XCTAssertEqual(clock.elapsedMilliseconds(at: 103.25), 1_250)
        clock.stop(at: 104)
        XCTAssertEqual(clock.elapsedMilliseconds(at: 200), 2_000)
        clock.stop(at: 205)
        XCTAssertEqual(clock.elapsedMilliseconds(at: 300), 2_000)
        clock.start(at: 400)
        XCTAssertNil(clock.elapsedMilliseconds(at: 399))
        XCTAssertNil(clock.elapsedMilliseconds(at: .infinity))
        XCTAssertEqual(clock.elapsedMilliseconds(at: 400.5), 500)
    }

    func testEmptyProcessResolutionPreservesGlobalCaptureButCannotSupplyAppNames() {
        for processes in [[], [AudioObjectID(kAudioObjectUnknown)]] {
            var source = MeetingAudioSourceIntegrity(appRequested: true)
            var events: [String] = []
            let captured = source.prepare(processes: processes, report: { events.append("scope:\($0)") }) { selected in
                events.append("capture:\(selected.appScoped)")
                return "audio continues"
            }
            XCTAssertEqual(events, ["scope:false", "capture:false"])
            XCTAssertEqual(captured, "audio continues")
            XCTAssertFalse(source.appScopedForNaming)
            XCTAssertEqual(source.scope, "global")
            XCTAssertNotNil(source.warning)
        }
    }

    func testTargetedToGlobalInvalidatesBeforeFallbackAudioAndCannotRecoverNames() {
        var source = MeetingAudioSourceIntegrity(appRequested: true)
        var events: [String] = []
        _ = source.prepare(processes: [42], report: { events.append("scope:\($0)") }) { selected in
            XCTAssertEqual(selected.processes, [42])
            return 1
        }
        XCTAssertTrue(source.appScopedForNaming)
        _ = source.prepare(processes: nil, report: { events.append("scope:\($0)") }) { selected in
            events.append("fallback:\(selected.appScoped)")
            return 2
        }
        XCTAssertEqual(events, ["scope:true", "scope:false", "fallback:false"])
        _ = source.prepare(processes: [43], report: { events.append("scope:\($0)") }) { _ in 3 }
        XCTAssertFalse(source.appScopedForNaming, "later app audio must not repair a widened recording")
        XCTAssertEqual(events.last, "scope:false")
    }

    func testHealthyTargetedRebuildsRetainScopeAndDoNotWarn() {
        var source = MeetingAudioSourceIntegrity(appRequested: true)
        var reports: [Bool] = []
        for processes: [AudioObjectID] in [[42], [42, 43]] {
            _ = source.prepare(processes: processes, report: { reports.append($0) }) { selected in
                XCTAssertTrue(selected.appScoped)
                return true
            }
        }
        XCTAssertEqual(reports, [true, true])
        XCTAssertTrue(source.appScopedForNaming)
        XCTAssertEqual(source.scope, "app")
        XCTAssertNil(source.warning)
    }

    func testManualGlobalCaptureRemainsGlobalAndNeverQualifiesFromALaterReport() {
        var source = MeetingAudioSourceIntegrity(appRequested: false)
        var reports: [Bool] = []
        let captured = source.prepare(processes: nil, report: { reports.append($0) }) { selected in
            XCTAssertFalse(selected.appScoped)
            return "global audio"
        }
        XCTAssertEqual(captured, "global audio")
        XCTAssertEqual(source.scope, "global")
        XCTAssertNil(source.warning)
        _ = source.prepare(processes: [42], report: { reports.append($0) }) { selected in
            XCTAssertFalse(selected.appScoped, "manual capture must remain global")
            return true
        }
        XCTAssertEqual(reports, [false, false])
        XCTAssertFalse(source.appScopedForNaming)
    }

    func testCaptureFailureInvalidatesAndCannotBeRequalifiedByARestart() {
        var source = MeetingAudioSourceIntegrity(appRequested: true)
        var reports: [Bool] = []
        XCTAssertThrowsError(try source.prepare(processes: [42], report: { reports.append($0) }) { _ -> Void in
            throw CocoaError(.fileReadUnknown)
        })
        XCTAssertEqual(reports, [true, false])
        XCTAssertEqual(source.scope, "unavailable")
        XCTAssertFalse(source.appScopedForNaming)
        source.failed(report: { reports.append($0) })
        XCTAssertFalse(source.appScopedForNaming)
    }

    func testScopeAwareAndLegacyBridgeStartsRejectMissingPathsWithoutHardwareOrPermissions() {
        for scoped in [false, true] {
            var error: UnsafeMutablePointer<CChar>?
            let handle: UnsafeMutableRawPointer?
            if scoped {
                handle = meetingStartPathsWithScope(nil, nil, nil, nil, false, { _, _, _, _, _ in }, nil,
                                                   { _, _ in XCTFail("invalid paths must not start capture") }, nil, &error)
            } else {
                handle = meetingStartPaths(nil, nil, nil, nil, false, { _, _, _, _, _ in }, nil, &error)
            }
            XCTAssertNil(handle)
            XCTAssertEqual(error.map { String(cString: $0) }, "No recording paths provided")
            stringFree(error)
        }
    }
}
