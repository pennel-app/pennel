import Foundation
import AVFoundation
import CoreImage
import UIKit
import Testing
@testable internal import PennelMobile

@Suite("Expo sync setup")
struct SyncSetupTests {
    @Test func deviceAdmissionAvailabilityRetainsItsMachineReadableReason() throws {
        let supported = NativeCloudViews.availability(CloudLinkAvailability(available: true, reasonCode: nil))
        let disabled = NativeCloudViews.availability(CloudLinkAvailability(available: false, reasonCode: "pairing_unavailable"))
        #expect(supported["available"] as? Bool == true)
        #expect(supported["reason_code"] is NSNull)
        #expect(disabled["available"] as? Bool == false)
        #expect(disabled["reason_code"] as? String == "pairing_unavailable")
        #expect(JSONSerialization.isValidJSONObject(disabled))
    }

    @Test func browserReturnIsBoundToTheInstalledVariantAndCarriesNoCredentials() throws {
        #expect(PennelNativeRuntime.acceptsSyncReturn(try #require(URL(string: "pennel://sync")), scheme: "pennel"))
        #expect(PennelNativeRuntime.acceptsSyncReturn(try #require(URL(string: "pennel-preview://sync")), scheme: "pennel-preview"))
        for value in ["pennel://sync", "https://sync", "pennel-preview://sync?token=x", "pennel-preview://sync#token=x", "pennel-preview://user@sync", "pennel-preview://sync:443", "pennel-preview://sync/path"] {
            #expect(!PennelNativeRuntime.acceptsSyncReturn(try #require(URL(string: value)), scheme: "pennel-preview"))
        }
        #expect(!PennelNativeRuntime.acceptsSyncReturn(try #require(URL(string: "pennel://sync")), scheme: nil))
    }

    @Test func setupProjectionKeepsLockedAndPausedDistinct() throws {
        let locked = CloudSyncSetup(phase: .locked, syncEnabled: false, generation: 1, canApproveDevices: false, uploaded: 0, downloaded: 0, conflicts: 0, lastSuccessMs: nil)
        let paused = CloudSyncSetup(phase: .paused, syncEnabled: false, generation: 1, canApproveDevices: true, uploaded: 1, downloaded: 2, conflicts: 0, lastSuccessMs: 3)
        #expect(NativeCloudViews.setup(locked)["phase"] as? String == "locked")
        #expect(NativeCloudViews.setup(paused)["phase"] as? String == "paused")
        #expect(NativeCloudViews.setup(paused)["can_approve_devices"] as? Bool == true)
        #expect(NativeCloudViews.setup(paused)["last_success_ms"] as? Int64 == 3)
    }

    @MainActor @Test func connectionPngHasOpaqueQuietZoneAndDecodesInDarkAppearance() throws {
        let payload = #"{"protocol":"pennel-device-link-v1","service":"https://example.invalid","code":"fixture-1234-5678"}"#
        let view = CloudLinkView(phase: .waitingPeer, role: .newDevice, requestId: "fixture", connectionCode: nil, qrPayload: payload, qrSvg: nil, verificationCode: nil, peerName: nil, peerPlatform: nil, accountEmail: nil, expiresAtMs: nil, errorCode: nil, errorMessage: nil)
        var encoded: String?
        UITraitCollection(userInterfaceStyle: .dark).performAsCurrent {
            encoded = NativeCloudViews.link(view)["qr_image"] as? String
        }
        let url = try #require(encoded)
        let prefix = "data:image/png;base64,"
        #expect(url.hasPrefix(prefix))
        let data = try #require(Data(base64Encoded: String(url.dropFirst(prefix.count))))
        let image = try #require(UIImage(data: data)?.cgImage)
        let detector = try #require(CIDetector(ofType: CIDetectorTypeQRCode, context: nil, options: [CIDetectorAccuracy: CIDetectorAccuracyHigh]))
        let decoded = detector.features(in: CIImage(cgImage: image)).compactMap { ($0 as? CIQRCodeFeature)?.messageString }
        #expect(decoded == [payload])
        let context = try #require(CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        let pixels = try #require(context.data?.assumingMemoryBound(to: UInt8.self))
        let border = 4 * 8
        var white = true
        for y in 0..<image.height {
            for x in 0..<image.width where x < border || y < border || x >= image.width - border || y >= image.height - border {
                let offset = y * context.bytesPerRow + x * 4
                if pixels[offset] != 255 || pixels[offset + 1] != 255 || pixels[offset + 2] != 255 || pixels[offset + 3] != 255 { white = false }
            }
        }
        #expect(white)
    }

    @MainActor @Test func recoveryConfirmationTokenIsBoundToOneDisplayAndAuthGeneration() throws {
        let old = NativeRecoveryDisplay(code: "fixture-a", context: "vault-a", identity: 1)
        let replacement = NativeRecoveryDisplay(code: "fixture-b", context: "vault-b", identity: 1)
        #expect(old.matches(token: old.token, identity: 1))
        #expect(!old.matches(token: old.token, identity: 2))
        #expect(!replacement.matches(token: old.token, identity: 1))
        let generation = NativeLoginGeneration()
        let epoch = generation.begin()
        generation.cancel()
        #expect(!generation.isCurrent(epoch))
    }

    @MainActor @Test func practiceCannotAdoptOrStopAnExistingOrdinaryDictation() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let capture = PracticeCapture()
        let recorder = NativeDictationRecorder(engine: engine, capture: capture, audio: PracticeAudioSession(), requestPermission: { true }, reserveHistory: {}, observeSystem: false)
        try await recorder.start()
        let ordinary = try #require(recorder.sessionID)
        do { try await recorder.start(practice: true); Issue.record("Practice adopted another capture.") }
        catch let error as PennelBridgeError { #expect(error.code == "operation_busy") }
        do { try await recorder.stopPractice(sessionID: ordinary); Issue.record("Practice stopped an ordinary capture.") }
        catch let error as PennelBridgeError { #expect(error.code == "dictation_session_changed") }
        #expect(recorder.phase == .listening)
        #expect(recorder.sessionID == ordinary)
        #expect(!recorder.isPractice)
        #expect(capture.running)
        recorder.cancel()
    }

    @MainActor @Test func stalePracticeCommandsCannotTouchTheNextCapture() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let capture = PracticeCapture()
        let recorder = NativeDictationRecorder(engine: engine, capture: capture, audio: PracticeAudioSession(), requestPermission: { true }, reserveHistory: {}, observeSystem: false)
        try await recorder.start(practice: true)
        let old = try #require(recorder.sessionID)
        try recorder.cancelPractice(sessionID: old)
        #expect(recorder.sessionID == nil)
        #expect(recorder.resultSessionID == nil)
        try await recorder.start()
        #expect(recorder.sessionID != old)
        do { try recorder.cancelPractice(sessionID: old); Issue.record("A stale cancel touched the next capture.") }
        catch let error as PennelBridgeError { #expect(error.code == "dictation_session_changed") }
        do { try await recorder.stopPractice(sessionID: old); Issue.record("A stale stop touched the next capture.") }
        catch let error as PennelBridgeError { #expect(error.code == "dictation_session_changed") }
        #expect(recorder.phase == .listening)
        #expect(capture.running)
        recorder.cancel()
    }

    @MainActor @Test func pendingStartCannotBeAdoptedByPractice() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let gate = PracticePermissionGate()
        let capture = PracticeCapture()
        let recorder = NativeDictationRecorder(engine: engine, capture: capture, audio: PracticeAudioSession(), requestPermission: { await gate.request() }, reserveHistory: {}, observeSystem: false)
        let pending = Task { try await recorder.start() }
        await gate.waitUntilEntered()
        do { try await recorder.start(practice: true); Issue.record("Practice adopted a pending start.") }
        catch let error as PennelBridgeError { #expect(error.code == "operation_busy") }
        recorder.cancel()
        gate.resolve()
        do { try await pending.value; Issue.record("Cancelled start resumed capture.") }
        catch PennelError.Cancelled {}
        #expect(!capture.running)
        #expect(recorder.sessionID == nil)
    }
}

private final class PracticeCapture: AudioCapturing, @unchecked Sendable {
    private let lock = NSLock()
    private var active = false
    var running: Bool { lock.withLock { active } }
    func start(onSamples: @escaping @Sendable ([Float]) -> Void) throws { lock.withLock { active = true } }
    func stop() { lock.withLock { active = false } }
}

private struct PracticeAudioSession: AudioSessionControlling {
    func activate() throws {}
    func deactivate() {}
}

@MainActor private final class PracticePermissionGate {
    private var pending: CheckedContinuation<Bool, Never>?
    private var entered = false
    private var observers: [CheckedContinuation<Void, Never>] = []
    func request() async -> Bool {
        entered = true
        observers.forEach { $0.resume() }
        observers = []
        return await withCheckedContinuation { pending = $0 }
    }
    func waitUntilEntered() async {
        if entered { return }
        await withCheckedContinuation { observers.append($0) }
    }
    func resolve() { pending?.resume(returning: true); pending = nil }
}
