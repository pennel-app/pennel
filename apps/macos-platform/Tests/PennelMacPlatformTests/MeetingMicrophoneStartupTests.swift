import AVFoundation
import XCTest
@testable import PennelMacPlatform

final class MeetingMicrophoneStartupTests: XCTestCase {
    private enum UnexpectedFailure: Error, Equatable { case unavailable }

    func testTapFormatIsResolvedAfterVoiceProcessingConfiguration() throws {
        var format = 48_000
        var events: [String] = []
        let selected = try MeetingMicrophoneFormatReadiness.resolve(
            attempts: 1, configure: { events.append("configure"); format = 24_000 },
            read: { events.append("read"); return format }, usable: { $0 > 0 },
            defaultInputAvailable: { true }, checkAccess: {}, pause: { XCTFail("ready route must not wait") }
        )
        XCTAssertEqual(selected, 24_000)
        XCTAssertEqual(events, ["configure", "read"])
    }

    func testFormatReadinessWaitsForExistingDefaultInputAfterRouteReconfiguration() throws {
        var formats = [0, 0, 48_000]
        var waits = 0
        let format = try MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { formats.removeFirst() }, usable: { $0 > 0 },
            defaultInputAvailable: { true }, checkAccess: {}, pause: { waits += 1 }
        )
        XCTAssertEqual(format, 48_000)
        XCTAssertEqual(waits, 2)
    }

    func testFormatReadinessDoesNotWaitForAHealthyFormatOrAMissingDevice() throws {
        let format = try MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { 24_000 }, usable: { $0 > 0 },
            defaultInputAvailable: { XCTFail("healthy input must not need re-resolution"); return false },
            checkAccess: {}, pause: { XCTFail("healthy input must not wait") }
        )
        XCTAssertEqual(format, 24_000)
        XCTAssertThrowsError(try MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { 0 }, usable: { $0 > 0 }, defaultInputAvailable: { false },
            checkAccess: {}, pause: { XCTFail("missing input must not wait") }
        )) { XCTAssertTrue($0 is NoInputDevice) }
    }

    func testPersistentUnreadyRouteIsBoundedAndRetainsPermissionFailures() {
        var reads = 0
        var waits = 0
        XCTAssertThrowsError(try MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { reads += 1; return 0 }, usable: { $0 > 0 }, defaultInputAvailable: { true },
            checkAccess: {}, pause: { waits += 1 }
        )) { XCTAssertEqual($0 as? MicrophoneStartError, .failed) }
        XCTAssertEqual(reads, 3)
        XCTAssertEqual(waits, 2)
        XCTAssertThrowsError(try MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { XCTFail("denied access must not query input"); return 0 }, usable: { $0 > 0 },
            defaultInputAvailable: { true }, checkAccess: { throw MicrophoneStartError.permissionDenied },
            pause: { XCTFail("denied access must not wait") }
        )) { XCTAssertEqual($0 as? MicrophoneStartError, .permissionDenied) }
    }

    func testEchoStartupDoesNotResetOrRetryAHealthyEngine() throws {
        var attempts: [Bool] = []
        var resets = 0
        let echo = try MeetingMicrophoneStartup.start(
            echoCancellation: true,
            attempt: { attempts.append($0) },
            reset: { resets += 1 }
        )

        XCTAssertTrue(echo)
        XCTAssertEqual(attempts, [true])
        XCTAssertEqual(resets, 0)
        var health = MeetingCaptureHealth(echoRequested: true)
        health.microphoneStarted(echoActive: echo)
        XCTAssertTrue(health.microphoneActive)
        XCTAssertNil(health.microphoneError)
        XCTAssertNil(health.echoWarning)
    }

    func testRawStartupDoesNotEnableEchoOrResetAHealthyEngine() throws {
        var attempts: [Bool] = []
        var resets = 0
        let echo = try MeetingMicrophoneStartup.start(
            echoCancellation: false,
            attempt: { attempts.append($0) },
            reset: { resets += 1 }
        )

        XCTAssertFalse(echo)
        XCTAssertEqual(attempts, [false])
        XCTAssertEqual(resets, 0)
    }

    func testFailedEchoStartupResetsBeforeOneRawAttemptAndShowsTheEchoWarning() throws {
        var events: [String] = []
        var failedTapInstalled = false
        let echo = try MeetingMicrophoneStartup.start(
            echoCancellation: true,
            attempt: { echo in
                events.append(echo ? "echo" : "raw")
                if echo {
                    failedTapInstalled = true
                    throw MicrophoneStartError.failed
                }
                XCTAssertFalse(failedTapInstalled, "The failed capture pipeline must be removed before retrying.")
            },
            reset: {
                events.append("reset")
                failedTapInstalled = false
            }
        )

        XCTAssertEqual(events, ["echo", "reset", "raw"])
        XCTAssertFalse(echo)
        var health = MeetingCaptureHealth(echoRequested: true)
        health.microphoneStarting()
        health.microphoneStarted(echoActive: echo)
        XCTAssertTrue(health.microphoneActive)
        XCTAssertNil(health.microphoneError)
        XCTAssertFalse(health.echoActive)
        XCTAssertNotNil(health.echoWarning)
    }

    func testBothFailuresStopAfterOneRetryAndPreserveTheSecondError() {
        for secondFailure in [MicrophoneStartError.failed, .permissionDenied] {
            var attempts: [Bool] = []
            var resets = 0
            XCTAssertThrowsError(try MeetingMicrophoneStartup.start(
                echoCancellation: true,
                attempt: { echo in
                    attempts.append(echo)
                    throw echo ? MicrophoneStartError.failed : secondFailure
                },
                reset: { resets += 1 }
            )) { error in
                XCTAssertEqual(error as? MicrophoneStartError, secondFailure)
            }

            XCTAssertEqual(attempts, [true, false])
            XCTAssertEqual(resets, 1)
        }
    }

    func testRawStartupFailureDoesNotRetry() {
        var attempts: [Bool] = []
        var resets = 0
        XCTAssertThrowsError(try MeetingMicrophoneStartup.start(
            echoCancellation: false,
            attempt: {
                attempts.append($0)
                throw MicrophoneStartError.failed
            },
            reset: { resets += 1 }
        )) { error in
            XCTAssertEqual(error as? MicrophoneStartError, .failed)
        }

        XCTAssertEqual(attempts, [false])
        XCTAssertEqual(resets, 0)
    }

    func testPermissionFailuresDoNotRetryOrLoseTheirSpecificMessage() {
        for failure in [MicrophoneStartError.permissionDenied, .permissionPending] {
            var attempts: [Bool] = []
            var resets = 0
            XCTAssertThrowsError(try MeetingMicrophoneStartup.start(
                echoCancellation: true,
                attempt: {
                    attempts.append($0)
                    throw failure
                },
                reset: { resets += 1 }
            )) { error in
                XCTAssertEqual(error as? MicrophoneStartError, failure)
                XCTAssertEqual(error.localizedDescription, failure.localizedDescription)
            }
            XCTAssertEqual(attempts, [true])
            XCTAssertEqual(resets, 0)
        }
    }

    func testMissingInputDoesNotRetryOrBecomeAGenericStartupError() {
        var attempts: [Bool] = []
        var resets = 0
        XCTAssertThrowsError(try MeetingMicrophoneStartup.start(
            echoCancellation: true,
            attempt: {
                attempts.append($0)
                throw NoInputDevice()
            },
            reset: { resets += 1 }
        )) { error in
            XCTAssertTrue(error is NoInputDevice)
        }

        XCTAssertEqual(attempts, [true])
        XCTAssertEqual(resets, 0)
    }

    func testUnknownFailureDoesNotRetryOrBecomeAMicrophoneError() {
        var attempts: [Bool] = []
        var resets = 0
        XCTAssertThrowsError(try MeetingMicrophoneStartup.start(
            echoCancellation: true,
            attempt: {
                attempts.append($0)
                throw UnexpectedFailure.unavailable
            },
            reset: { resets += 1 }
        )) { error in
            XCTAssertEqual(error as? UnexpectedFailure, .unavailable)
        }

        XCTAssertEqual(attempts, [true])
        XCTAssertEqual(resets, 0)
    }

    func testConfigurationChangesFollowTheReplacementEngine() {
        let center = NotificationCenter()
        let observer = MeetingMicrophoneConfigurationObserver(center: center)
        let original = AVAudioEngine()
        let replacement = AVAudioEngine()
        var changes: [String] = []
        observer.observe(original) { changes.append("original") }
        center.post(name: .AVAudioEngineConfigurationChange, object: original)
        let delivered = expectation(description: "replacement engine change")
        observer.observe(replacement) { changes.append("replacement"); delivered.fulfill() }
        center.post(name: .AVAudioEngineConfigurationChange, object: original)
        center.post(name: .AVAudioEngineConfigurationChange, object: replacement)
        wait(for: [delivered], timeout: 1)

        XCTAssertEqual(changes, ["replacement"], "Queued changes from a replaced engine must be ignored.")
    }

    func testStoppedCaptureNoLongerReceivesConfigurationChanges() {
        let center = NotificationCenter()
        let observer = MeetingMicrophoneConfigurationObserver(center: center)
        let engine = AVAudioEngine()
        var changes = 0
        observer.observe(engine) { changes += 1 }
        center.post(name: .AVAudioEngineConfigurationChange, object: engine)
        observer.stop()
        center.post(name: .AVAudioEngineConfigurationChange, object: engine)
        let drained = expectation(description: "queued notifications drained")
        DispatchQueue.main.async { drained.fulfill() }
        wait(for: [drained], timeout: 1)

        XCTAssertEqual(changes, 0)
    }

    func testConfigurationChangesDuringStartupWaitUntilStartupReturns() {
        let center = NotificationCenter()
        let observer = MeetingMicrophoneConfigurationObserver(center: center)
        let engine = AVAudioEngine()
        var starting = true
        let delivered = expectation(description: "startup configuration change")
        observer.observe(engine) {
            XCTAssertFalse(starting, "Recovery must not reenter microphone startup.")
            delivered.fulfill()
        }
        center.post(name: .AVAudioEngineConfigurationChange, object: engine)
        starting = false
        wait(for: [delivered], timeout: 1)
    }
}
