import AVFoundation
import XCTest
@testable import PennelMacPlatform

final class MeetingMicrophoneStartupTests: XCTestCase {
    private enum UnexpectedFailure: Error, Equatable { case unavailable }

    @MeetingMicrophoneControl
    private func assertThrows<T>(_ test: @autoclosure () async throws -> T,
                                 file: StaticString = #filePath, line: UInt = #line,
                                 _ check: (Error) -> Void) async {
        do {
            _ = try await test()
            XCTFail("Expected startup to fail", file: file, line: line)
        } catch { check(error) }
    }


    @MeetingMicrophoneControl
    func testTapFormatIsResolvedAfterVoiceProcessingConfiguration() async throws {
        var format = 48_000
        var events: [String] = []
        let selected = try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 1, configure: { events.append("configure"); format = 24_000 },
            read: { events.append("read"); return format }, usable: { $0 > 0 },
            defaultInputAvailable: { true }, checkAccess: {}, pause: { XCTFail("ready route must not wait") }
        )
        XCTAssertEqual(selected, 24_000)
        XCTAssertEqual(events, ["configure", "read"])
    }

    @MeetingMicrophoneControl
    func testFormatReadinessWaitsForExistingDefaultInputAfterRouteReconfiguration() async throws {
        var formats = [0, 0, 48_000]
        var waits = 0
        let format = try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { formats.removeFirst() }, usable: { $0 > 0 },
            defaultInputAvailable: { true }, checkAccess: {}, pause: { waits += 1 }
        )
        XCTAssertEqual(format, 48_000)
        XCTAssertEqual(waits, 2)
    }

    @MeetingMicrophoneControl
    func testFormatReadinessDoesNotWaitForAHealthyFormatOrAMissingDevice() async throws {
        let format = try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { 24_000 }, usable: { $0 > 0 },
            defaultInputAvailable: { XCTFail("healthy input must not need re-resolution"); return false },
            checkAccess: {}, pause: { XCTFail("healthy input must not wait") }
        )
        XCTAssertEqual(format, 24_000)
        await assertThrows(try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { 0 }, usable: { $0 > 0 }, defaultInputAvailable: { false },
            checkAccess: {}, pause: { XCTFail("missing input must not wait") }
        )) { XCTAssertTrue($0 is NoInputDevice) }
    }

    @MeetingMicrophoneControl
    func testPersistentUnreadyRouteIsBoundedAndRetainsPermissionFailures() async throws {
        var reads = 0
        var waits = 0
        await assertThrows(try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { reads += 1; return 0 }, usable: { $0 > 0 }, defaultInputAvailable: { true },
            checkAccess: {}, pause: { waits += 1 }
        )) { XCTAssertEqual($0 as? MicrophoneStartError, .failed) }
        XCTAssertEqual(reads, 3)
        XCTAssertEqual(waits, 2)
        await assertThrows(try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 3, read: { XCTFail("denied access must not query input"); return 0 }, usable: { $0 > 0 },
            defaultInputAvailable: { true }, checkAccess: { throw MicrophoneStartError.permissionDenied },
            pause: { XCTFail("denied access must not wait") }
        )) { XCTAssertEqual($0 as? MicrophoneStartError, .permissionDenied) }
    }

    @MeetingMicrophoneControl
    func testEchoStartupDoesNotResetOrRetryAHealthyEngine() async throws {
        var attempts: [Bool] = []
        var resets = 0
        let echo = try await MeetingMicrophoneStartup.start(
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

    @MeetingMicrophoneControl
    func testRawStartupDoesNotEnableEchoOrResetAHealthyEngine() async throws {
        var attempts: [Bool] = []
        var resets = 0
        let echo = try await MeetingMicrophoneStartup.start(
            echoCancellation: false,
            attempt: { attempts.append($0) },
            reset: { resets += 1 }
        )

        XCTAssertFalse(echo)
        XCTAssertEqual(attempts, [false])
        XCTAssertEqual(resets, 0)
    }

    @MeetingMicrophoneControl
    func testFailedEchoStartupResetsBeforeOneRawAttemptAndShowsTheEchoWarning() async throws {
        var events: [String] = []
        var failedTapInstalled = false
        let echo = try await MeetingMicrophoneStartup.start(
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

    @MeetingMicrophoneControl
    func testBothFailuresStopAfterOneRetryAndPreserveTheSecondError() async throws {
        for secondFailure in [MicrophoneStartError.failed, .permissionDenied] {
            var attempts: [Bool] = []
            var resets = 0
            await assertThrows(try await MeetingMicrophoneStartup.start(
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

    @MeetingMicrophoneControl
    func testRawStartupFailureDoesNotRetry() async throws {
        var attempts: [Bool] = []
        var resets = 0
        await assertThrows(try await MeetingMicrophoneStartup.start(
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

    @MeetingMicrophoneControl
    func testPermissionFailuresDoNotRetryOrLoseTheirSpecificMessage() async throws {
        for failure in [MicrophoneStartError.permissionDenied, .permissionPending] {
            var attempts: [Bool] = []
            var resets = 0
            await assertThrows(try await MeetingMicrophoneStartup.start(
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

    @MeetingMicrophoneControl
    func testMissingInputDoesNotRetryOrBecomeAGenericStartupError() async throws {
        var attempts: [Bool] = []
        var resets = 0
        await assertThrows(try await MeetingMicrophoneStartup.start(
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

    @MeetingMicrophoneControl
    func testUnknownFailureDoesNotRetryOrBecomeAMicrophoneError() async throws {
        var attempts: [Bool] = []
        var resets = 0
        await assertThrows(try await MeetingMicrophoneStartup.start(
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

    @MeetingMicrophoneControl
    func testConfigurationChangesFollowTheReplacementEngine() async throws {
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
        await fulfillment(of: [delivered], timeout: 1)

        XCTAssertEqual(changes, ["replacement"], "Queued changes from a replaced engine must be ignored.")
    }

    @MeetingMicrophoneControl
    func testStoppedCaptureNoLongerReceivesConfigurationChanges() async throws {
        let center = NotificationCenter()
        let observer = MeetingMicrophoneConfigurationObserver(center: center)
        let engine = AVAudioEngine()
        var changes = 0
        observer.observe(engine) { changes += 1 }
        center.post(name: .AVAudioEngineConfigurationChange, object: engine)
        observer.stop()
        center.post(name: .AVAudioEngineConfigurationChange, object: engine)
        let drained = expectation(description: "queued notifications drained")
        Task { @MeetingMicrophoneControl in drained.fulfill() }
        await fulfillment(of: [drained], timeout: 1)

        XCTAssertEqual(changes, 0)
    }

    @MeetingMicrophoneControl
    func testConfigurationChangesDuringStartupWaitUntilStartupReturns() async throws {
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
        await fulfillment(of: [delivered], timeout: 1)
    }

    @MeetingMicrophoneControl
    func testReadinessLetsTheConfigurationOwnerAdvanceWithoutAcceptingTheOldClientFormat() async throws {
        let center = NotificationCenter()
        let observer = MeetingMicrophoneConfigurationObserver(center: center)
        let engine = AVAudioEngine()
        var hardwareChannels = 0
        let cachedClientChannels = 2
        var configured = 0
        observer.observe(engine) { hardwareChannels = 1 }

        let format = try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 20,
            configure: {
                configured += 1
                center.post(name: .AVAudioEngineConfigurationChange, object: engine)
            },
            read: { hardwareChannels },
            usable: { $0 > 0 },
            defaultInputAvailable: { true },
            checkAccess: {},
            pause: { try await Task.sleep(for: .milliseconds(2)) }
        )
        XCTAssertEqual(configured, 1)
        XCTAssertEqual(format, 1)
        XCTAssertNotEqual(format, cachedClientChannels)
    }

    @MeetingMicrophoneControl
    func testStopOrSupersessionDuringReadinessNeverStartsOrRetriesTheOldEngine() async throws {
        for supersede in [false, true] {
            var generation = CaptureGeneration()
            let first = generation.begin()
            var reads = 0
            var resets = 0
            var installs = 0
            await assertThrows(try await MeetingMicrophoneStartup.start(echoCancellation: true, attempt: { _ in
                _ = try await MeetingMicrophoneFormatReadiness.resolve(
                    attempts: 3, read: { reads += 1; return reads == 1 ? 0 : 24_000 },
                    usable: { $0 > 0 }, defaultInputAvailable: { true },
                    checkAccess: {
                        guard generation.accepts(first) else { throw CancellationError() }
                    },
                    pause: {
                        if supersede { _ = generation.begin() } else { generation.stop() }
                        await Task.yield()
                    }
                )
                installs += 1
            }, reset: { resets += 1 })) { XCTAssertTrue($0 is CancellationError) }
            XCTAssertEqual(reads, 1)
            XCTAssertEqual(resets, 0)
            XCTAssertEqual(installs, 0)
        }
    }

    @MeetingMicrophoneControl
    func testProcessedEngineMustBeEnabledAndNotBypassedBeforeAcknowledgingIt() async throws {
        for (enabled, bypassed) in [(false, false), (true, true)] {
            var attempts: [Bool] = []
            var resets = 0
            let usedEcho = try await MeetingMicrophoneStartup.start(echoCancellation: true, attempt: { echo in
                attempts.append(echo)
                try MeetingMicrophoneStartup.verifyProcessing(requested: echo, enabled: enabled, bypassed: bypassed)
            }, reset: { resets += 1 })
            XCTAssertEqual(attempts, [true, false])
            XCTAssertEqual(resets, 1)
            XCTAssertFalse(usedEcho)
        }
    }

    func testRawFallbackRetriesProcessingOnlyOnAnIdentifiedNewRoute() {
        let airPods = MeetingMicrophoneRoute(input: 4, output: 6)
        let builtIn = MeetingMicrophoneRoute(input: 7, output: 6)
        var policy = MeetingMicrophoneEchoPolicy(requested: true)
        XCTAssertTrue(policy.shouldTry(on: airPods))
        policy.didStart(processed: false, on: airPods)
        XCTAssertFalse(policy.shouldTry(on: airPods))
        XCTAssertFalse(policy.shouldTry(on: MeetingMicrophoneRoute(input: nil, output: 6)))
        XCTAssertTrue(policy.shouldTry(on: builtIn))
        policy.didStart(processed: true, on: builtIn)
        XCTAssertTrue(policy.shouldTry(on: airPods))
        policy.didStart(processed: false, on: builtIn)
        XCTAssertTrue(policy.shouldTry(on: airPods))
        XCTAssertFalse(MeetingMicrophoneEchoPolicy(requested: false).shouldTry(on: airPods))
    }

    func testReplacedTapCannotDeliverLateAudio() {
        let old = MicrophoneTapAdmission()
        let replacement = MicrophoneTapAdmission()
        var count = 0
        old.deliver { count += 1 }
        old.close()
        old.deliver { XCTFail("old callback reached its writer after replacement") }
        replacement.deliver { count += 1 }
        XCTAssertEqual(count, 2)
    }

    @MeetingMicrophoneControl
    func testStoppedProcessedAttemptCannotReplaceTheEngineForAnOldRetry() async throws {
        var current = true
        var resets = 0
        var attempts: [Bool] = []
        await assertThrows(try await MeetingMicrophoneStartup.start(echoCancellation: true,
            checkCurrent: { if !current { throw CancellationError() } },
            attempt: { echo in
                attempts.append(echo)
                current = false
                throw MicrophoneStartError.failed
            }, reset: { resets += 1 })) { XCTAssertTrue($0 is CancellationError) }
        XCTAssertEqual(attempts, [true])
        XCTAssertEqual(resets, 0)
    }

    @MeetingMicrophoneControl
    func testDelayedSchedulerDoesNotExtendTheFormatReadinessBudget() async {
        var time: TimeInterval = 100
        var reads = 0
        var waits = 0
        await assertThrows(try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 41, now: { time }, read: { reads += 1; return 0 }, usable: { $0 > 0 },
            defaultInputAvailable: { true }, checkAccess: {},
            pause: { waits += 1; time += 3 }
        )) { XCTAssertEqual($0 as? MicrophoneStartError, .failed) }
        XCTAssertEqual(waits, 1)
        XCTAssertEqual(reads, 2)
    }

    func testDefaultObserversIgnoreIncompleteAndRepeatedRouteReports() {
        let current = MeetingMicrophoneRoute(input: 2, output: 3)
        XCTAssertFalse(MeetingMicrophoneRoute(input: 2, output: nil).defaultChangeNeedsRestart(from: current, running: true))
        XCTAssertFalse(MeetingMicrophoneRoute(input: nil, output: 3).defaultChangeNeedsRestart(from: current, running: true))
        XCTAssertFalse(current.defaultChangeNeedsRestart(from: current, running: true))
        XCTAssertTrue(current.defaultChangeNeedsRestart(from: current, running: false))
        XCTAssertTrue(MeetingMicrophoneRoute(input: 2, output: 4).defaultChangeNeedsRestart(from: current, running: true))
    }

}
