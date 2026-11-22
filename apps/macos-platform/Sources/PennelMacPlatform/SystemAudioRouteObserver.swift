import CoreAudio
import Foundation

/// Outlives an individual tap, so a failed Bluetooth rebuild can recover on the next route change.
final class SystemAudioRouteObserver: @unchecked Sendable {
    typealias Subscribe = (AudioObjectID, AudioObjectPropertySelector, @escaping @Sendable () -> Void) -> (() -> Void)?
    private let currentOutput: () -> AudioObjectID?
    private let subscribe: Subscribe
    private var routeCancellation: (() -> Void)?
    private var deviceCancellations: [() -> Void] = []
    private var generation = CaptureGeneration()

    init(currentOutput: @escaping () -> AudioObjectID? = SystemAudioRouteObserver.outputDevice,
         subscribe: @escaping Subscribe = SystemAudioRouteObserver.systemSubscription) {
        self.currentOutput = currentOutput
        self.subscribe = subscribe
    }

    func observe(_ handler: @escaping @Sendable () -> Void) {
        stop()
        let token = generation.begin()
        routeCancellation = subscribe(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice) { [weak self] in
            guard let self, self.generation.accepts(token) else { return }
            self.followDevice(token: token, handler: handler)
            handler()
        }
        followDevice(token: token, handler: handler)
    }

    private func followDevice(token: UInt64, handler: @escaping @Sendable () -> Void) {
        deviceCancellations.forEach { $0() }
        deviceCancellations.removeAll()
        guard let device = currentOutput(), device != kAudioObjectUnknown else { return }
        for selector in [kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyDeviceIsAlive] {
            if let cancel = subscribe(device, selector, { [weak self] in
                guard let self, self.generation.accepts(token) else { return }
                handler()
            }) { deviceCancellations.append(cancel) }
        }
    }

    func stop() {
        generation.stop()
        routeCancellation?()
        routeCancellation = nil
        deviceCancellations.forEach { $0() }
        deviceCancellations.removeAll()
    }

    deinit { stop() }

    private static func outputDevice() -> AudioObjectID? {
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultOutputDevice,
                                                mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var value = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        return AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &value) == noErr ? value : nil
    }

    static func systemSubscription(object: AudioObjectID, selector: AudioObjectPropertySelector,
                                           handler: @escaping @Sendable () -> Void) -> (() -> Void)? {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal,
                                                mElement: kAudioObjectPropertyElementMain)
        let block: AudioObjectPropertyListenerBlock = { _, _ in handler() }
        guard AudioObjectAddPropertyListenerBlock(object, &address, .main, block) == noErr else { return nil }
        return { AudioObjectRemovePropertyListenerBlock(object, &address, .main, block) }
    }
}
