@preconcurrency import AVFoundation
import Foundation

enum MicrophoneAccessState: Equatable {
    case granted, notDetermined, denied

    init(_ status: AVAuthorizationStatus) {
        switch status {
        case .authorized: self = .granted
        case .notDetermined: self = .notDetermined
        default: self = .denied
        }
    }
}

/// User-facing microphone start failures. CoreAudio's own text is never shown; only its code is logged.
enum MicrophoneStartError: LocalizedError, Equatable {
    case permissionDenied
    case permissionPending
    case failed

    var errorDescription: String? {
        switch self {
        case .permissionDenied: "Give Pennel microphone access in System Settings → Privacy & Security → Microphone, then try again."
        case .permissionPending: "Allow microphone access in the macOS prompt, then try again."
        case .failed: "The microphone could not start. Choose another input device or try again."
        }
    }

    /// Initialization can fail when permission is withheld or voice-processing formats disagree.
    /// Check access separately so device failures never look like missing permission.
    static func forEngineFailure(access: MicrophoneAccessState) -> MicrophoneStartError {
        access == .granted ? .failed : .permissionDenied
    }
}

enum MeetingMicrophoneStartup {
    static func start(
        echoCancellation: Bool,
        attempt: (Bool) throws -> Void,
        reset: () -> Void
    ) throws -> Bool {
        do {
            try attempt(echoCancellation)
            return echoCancellation
        } catch {
            guard echoCancellation, error as? MicrophoneStartError == .failed else { throw error }
            reset()
            try attempt(false)
            return false
        }
    }
}

enum MeetingMicrophoneFormatReadiness {
    /// Voice-processing teardown can leave the new default-device aggregate temporarily empty.
    /// Wait only while a real default input remains available; permission and missing-device errors
    /// stay specific, and a persistent route failure remains a bounded startup failure.
    static func resolve<Format>(
        attempts: Int,
        configure: () -> Void = {},
        read: () -> Format,
        usable: (Format) -> Bool,
        defaultInputAvailable: () -> Bool,
        checkAccess: () throws -> Void,
        pause: () -> Void
    ) throws -> Format {
        guard attempts > 0 else { throw MicrophoneStartError.failed }
        try checkAccess()
        configure()
        for attempt in 0..<attempts {
            try checkAccess()
            let format = read()
            if usable(format) { return format }
            guard defaultInputAvailable() else { throw NoInputDevice() }
            if attempt + 1 < attempts { pause() }
        }
        throw MicrophoneStartError.failed
    }
}

final class MeetingMicrophoneConfigurationObserver {
    private let center: NotificationCenter
    private var observer: NSObjectProtocol?
    private var generation = 0

    init(center: NotificationCenter = .default) { self.center = center }

    func observe(_ engine: AVAudioEngine, onChange: @escaping () -> Void) {
        stop()
        let expectedGeneration = generation
        observer = center.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: nil) { [weak self] _ in
            // Never block CoreAudio or reenter startup while it changes the engine's devices.
            DispatchQueue.main.async { [weak self] in
                guard self?.generation == expectedGeneration else { return }
                onChange()
            }
        }
    }

    func stop() {
        generation += 1
        if let observer { center.removeObserver(observer) }
        observer = nil
    }

    deinit { stop() }
}

enum MicrophoneAccess {
    private static let log = Diagnostics("microphone")

    static var state: MicrophoneAccessState { MicrophoneAccessState(AVCaptureDevice.authorizationStatus(for: .audio)) }

    /// Called on the main queue, so an undecided permission is requested without waiting for the answer.
    static func ensureGranted() throws {
        switch state {
        case .granted: return
        case .denied: throw MicrophoneStartError.permissionDenied
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .audio) { _ in }
            throw MicrophoneStartError.permissionPending
        }
    }

    static func start(_ engine: AVAudioEngine) throws {
        do {
            try engine.start()
        } catch {
            let failure = error as NSError
            log.error("microphone start failed: \(failure.domain) \(failure.code)")
            throw MicrophoneStartError.forEngineFailure(access: state)
        }
    }
}
