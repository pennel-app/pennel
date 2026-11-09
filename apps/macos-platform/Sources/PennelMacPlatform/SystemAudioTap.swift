@preconcurrency import AVFoundation
import CoreAudio

struct SystemAudioTapError: Error {
    let step: String
    let status: OSStatus
}

/// Nil/empty process resolution is explicitly global, rather than an implicit targeted tap.
struct SystemAudioTapSelection: Equatable {
    let processes: [AudioObjectID]?

    init(processes: [AudioObjectID]?) {
        let resolved = processes?.filter { $0 != kAudioObjectUnknown } ?? []
        self.processes = resolved.isEmpty ? nil : resolved
    }

    var appScoped: Bool { processes != nil }
}

/// Captures everything the Mac plays (except Pennel itself) via a Core Audio process tap,
/// which only needs the "System Audio Recording Only" permission, not Screen Recording.
final class SystemAudioTap: @unchecked Sendable {
    private let log = Diagnostics("system-audio")
    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var procID: AudioDeviceIOProcID?
    private let queue = DispatchQueue(label: "dev.pennel.systemaudio.io", qos: .userInitiated)
    private let generationLock = NSLock()
    private var generation = CaptureGeneration()
    private var listeners: [(AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertyListenerBlock)] = []
    private var description: CATapDescription?
    private var outputID = AudioObjectID(kAudioObjectUnknown)
    private var layout: TapInputLayout?
    private var callbackTime: TimeInterval = 0
    private(set) var tapSampleRate = 0.0
    private(set) var aggregateSampleRate = 0.0
    private(set) var transport = "unknown"
    var onConfigurationChange: (@Sendable () -> Void)?
    var onInvalidBuffer: (@Sendable () -> Void)?

    /// Construct the aggregate before resolving the tap's effective input format. Bluetooth
    /// profiles can leave the tap ASBD at 48 kHz while the aggregate actually delivers 24 kHz.
    func prepare(processes: [AudioObjectID]? = nil) throws -> AVAudioFormat {
        try prepare(selection: SystemAudioTapSelection(processes: processes))
    }

    func prepare(selection: SystemAudioTapSelection) throws -> AVAudioFormat {
        stop()
        do {
            log.notice("tap selection: app_scoped=\(selection.appScoped), process_count=\(selection.processes?.count ?? 0)")
            let description = if let processes = selection.processes {
                CATapDescription(stereoMixdownOfProcesses: processes)
            } else {
                CATapDescription(stereoGlobalTapButExcludeProcesses: ownProcessObject().map { [$0] } ?? [])
            }
            description.uuid = UUID()
            description.muteBehavior = .unmuted
            description.isPrivate = true
            try check("create tap", AudioHardwareCreateProcessTap(description, &tapID))
            self.description = description
            outputID = try property(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice,
                                    initial: AudioObjectID(kAudioObjectUnknown))
            let outputUID = try defaultOutputUID()
            let aggregate: [String: Any] = [
                kAudioAggregateDeviceNameKey: "Pennel System Audio",
                kAudioAggregateDeviceUIDKey: UUID().uuidString,
                kAudioAggregateDeviceMainSubDeviceKey: outputUID,
                kAudioAggregateDeviceIsPrivateKey: true,
                kAudioAggregateDeviceIsStackedKey: false,
                kAudioAggregateDeviceTapAutoStartKey: true,
                kAudioAggregateDeviceSubDeviceListKey: [[kAudioSubDeviceUIDKey: outputUID]],
                kAudioAggregateDeviceTapListKey: [[kAudioSubTapDriftCompensationKey: true,
                                                 kAudioSubTapUIDKey: description.uuid.uuidString]],
            ]
            try check("create aggregate device", AudioHardwareCreateAggregateDevice(aggregate as CFDictionary, &aggregateID))
            let resolved = try inputLayout(diagnose: true)
            layout = resolved
            let code = (try? property(outputID, kAudioDevicePropertyTransportType, initial: UInt32(0))) ?? 0
            transport = CaptureDeviceTransport.name(code)
            return resolved.format
        } catch { stop(); throw error }
    }

    func start(onBuffer: @escaping @Sendable (AVAudioPCMBuffer, TimeInterval) -> Void) throws {
        guard let layout, aggregateID != kAudioObjectUnknown else {
            throw SystemAudioTapError(step: "not prepared", status: -1)
        }
        let token = generationLock.withLock { generation.begin() }
        let selected = AudioBufferList.allocate(maximumBuffers: layout.bufferIndices.count)
        let storage = TapBufferStorage(selected)
        try check("create io proc", AudioDeviceCreateIOProcIDWithBlock(&procID, aggregateID, queue) { [weak self, storage] now, input, acquired, _, _ in
            guard let self, self.generationLock.withLock({ self.generation.accepts(token) }) else { return }
            let all = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
            guard layout.validBuffers(all) else { self.onInvalidBuffer?(); return }
            for (slot, index) in layout.bufferIndices.enumerated() { storage.buffers[slot] = all[index] }
            guard let buffer = AVAudioPCMBuffer(pcmFormat: layout.format, bufferListNoCopy: storage.buffers.unsafePointer, deallocator: nil) else {
                self.onInvalidBuffer?(); return
            }
            let stamp = acquired.pointee
            let uptime: TimeInterval
            if stamp.mFlags.contains(.hostTimeValid) {
                uptime = AVAudioTime.seconds(forHostTime: stamp.mHostTime)
            } else if now.pointee.mFlags.contains(.hostTimeValid) {
                uptime = AVAudioTime.seconds(forHostTime: now.pointee.mHostTime) - Double(buffer.frameLength) / layout.format.sampleRate
            } else {
                uptime = ProcessInfo.processInfo.systemUptime - Double(buffer.frameLength) / layout.format.sampleRate
            }
            self.generationLock.withLock { self.callbackTime = ProcessInfo.processInfo.systemUptime }
            onBuffer(buffer, uptime)
        })
        do {
            // Selecting buffers in the callback does not disable a Bluetooth headset's mic.
            // Tell HAL that this IOProc consumes only the tap and no output streams.
            try setStreamUsage(scope: kAudioObjectPropertyScopeInput, enabled: Set(layout.streamIndices))
            try setStreamUsage(scope: kAudioObjectPropertyScopeOutput, enabled: [])
            observe(aggregateID, kAudioDevicePropertyNominalSampleRate)
            observe(aggregateID, kAudioDevicePropertyStreams, scope: kAudioObjectPropertyScopeInput)
            observe(aggregateID, kAudioDevicePropertyStreamConfiguration, scope: kAudioObjectPropertyScopeInput)
            observe(outputID, kAudioDevicePropertyNominalSampleRate)
            observe(tapID, kAudioTapPropertyFormat)
            observe(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice)
            for stream in try streams(aggregateID, scope: kAudioObjectPropertyScopeInput) {
                observe(stream, kAudioStreamPropertyVirtualFormat)
            }
            try check("start device", AudioDeviceStart(aggregateID, procID))
        } catch { stop(); throw error }
    }

    func configurationUnchangedAndFlowing() -> Bool {
        guard let layout, aggregateID != kAudioObjectUnknown,
              generationLock.withLock({ ProcessInfo.processInfo.systemUptime - callbackTime < 1 }),
              let currentOutput: AudioObjectID = try? property(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice,
                                                             initial: AudioObjectID(kAudioObjectUnknown)), currentOutput == outputID,
              let actual = try? inputLayout() else { return false }
        return actual.bufferIndices == layout.bufferIndices && actual.streamIndices == layout.streamIndices && actual.format == layout.format
    }

    func suspendDelivery() {
        generationLock.withLock { generation.suspend() }
    }

    func stop() {
        generationLock.withLock { generation.stop() }
        for (object, var address, block) in listeners {
            AudioObjectRemovePropertyListenerBlock(object, &address, .main, block)
        }
        listeners.removeAll()
        if aggregateID != kAudioObjectUnknown {
            if let procID {
                AudioDeviceStop(aggregateID, procID)
                AudioDeviceDestroyIOProcID(aggregateID, procID)
            }
            AudioHardwareDestroyAggregateDevice(aggregateID)
        }
        if tapID != kAudioObjectUnknown { AudioHardwareDestroyProcessTap(tapID) }
        procID = nil
        aggregateID = AudioObjectID(kAudioObjectUnknown)
        tapID = AudioObjectID(kAudioObjectUnknown)
        outputID = AudioObjectID(kAudioObjectUnknown)
        description = nil
        layout = nil
    }

    deinit { stop() }

    private func observe(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        let token = generationLock.withLock { generation.value }
        let block: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
            guard let self, self.generationLock.withLock({ self.generation.value == token }) else { return }
            self.onConfigurationChange?()
        }
        if AudioObjectAddPropertyListenerBlock(object, &address, .main, block) == noErr {
            listeners.append((object, address, block))
        }
    }

    private func inputLayout(diagnose: Bool = false) throws -> TapInputLayout {
        let tap = try tapFormat()
        let rate = try property(aggregateID, kAudioDevicePropertyNominalSampleRate, initial: Double(0))
        let inputStreams = try streams(aggregateID, scope: kAudioObjectPropertyScopeInput)
        let starts = try inputStreams.map { Int(try property($0, kAudioStreamPropertyStartingChannel, initial: UInt32(0))) }
        let formats = try inputStreams.map { try property($0, kAudioStreamPropertyVirtualFormat, initial: AudioStreamBasicDescription()) }
        // An output-only backing device can legitimately expose no input streams. A failed
        // hardware query is not proof of zero microphone channels and must not select tap buffers.
        let hardwareStreams = try streams(outputID, scope: kAudioObjectPropertyScopeInput)
        let hardwareChannels = hardwareStreams.isEmpty ? 0
            : try bufferChannels(outputID, scope: kAudioObjectPropertyScopeInput).reduce(0, +)
        let channels = try bufferChannels(aggregateID, scope: kAudioObjectPropertyScopeInput)
        if diagnose {
            func format(_ value: AudioStreamBasicDescription) -> [String: Any] {
                ["rate": value.mSampleRate.isFinite ? value.mSampleRate as Any : "nonfinite",
                 "channels": value.mChannelsPerFrame, "id": value.mFormatID, "flags": value.mFormatFlags,
                 "bits": value.mBitsPerChannel, "bytes_per_frame": value.mBytesPerFrame]
            }
            let snapshot: [String: Any] = [
                "tap": format(tap), "aggregate_rate": rate.isFinite ? rate as Any : "nonfinite",
                "hardware_input_channels": hardwareChannels, "stream_count": inputStreams.count,
                "stream_starts": Array(starts.prefix(32)), "stream_formats": Array(formats.prefix(32)).map(format),
                "buffer_channels": Array(channels.prefix(32)),
            ]
            if let data = try? JSONSerialization.data(withJSONObject: snapshot, options: [.sortedKeys]) {
                log.notice("tap input layout: \(String(decoding: data, as: UTF8.self))")
            }
        }
        guard let resolved = TapInputLayout.resolve(tap: tap, deviceRate: rate, hardwareChannels: hardwareChannels,
                                                   streamStarts: starts, streamFormats: formats, bufferChannels: channels) else {
            throw SystemAudioTapError(step: "resolve input streams", status: -1)
        }
        tapSampleRate = tap.mSampleRate
        aggregateSampleRate = rate
        return resolved
    }

    private func streams(_ device: AudioObjectID, scope: AudioObjectPropertyScope) throws -> [AudioObjectID] {
        var address = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreams, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        var size: UInt32 = 0
        try check("read stream list size", AudioObjectGetPropertyDataSize(device, &address, 0, nil, &size))
        var result = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        if size > 0 { try check("read streams", AudioObjectGetPropertyData(device, &address, 0, nil, &size, &result)) }
        return result
    }

    private func bufferChannels(_ device: AudioObjectID, scope: AudioObjectPropertyScope) throws -> [Int] {
        var address = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreamConfiguration, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        var size: UInt32 = 0
        try check("read stream configuration size", AudioObjectGetPropertyDataSize(device, &address, 0, nil, &size))
        guard size >= MemoryLayout<AudioBufferList>.size else { return [] }
        let raw = UnsafeMutableRawPointer.allocate(byteCount: Int(size), alignment: MemoryLayout<AudioBufferList>.alignment)
        defer { raw.deallocate() }
        try check("read stream configuration", AudioObjectGetPropertyData(device, &address, 0, nil, &size, raw))
        return UnsafeMutableAudioBufferListPointer(raw.assumingMemoryBound(to: AudioBufferList.self)).map { Int($0.mNumberChannels) }
    }

    private func setStreamUsage(scope: AudioObjectPropertyScope, enabled: Set<Int>) throws {
        guard let procID else { throw SystemAudioTapError(step: "missing io proc", status: -1) }
        let count = try streams(aggregateID, scope: scope).count
        guard count > 0 else { return }
        let offset = MemoryLayout<AudioHardwareIOProcStreamUsage>.offset(of: \.mStreamIsOn)!
        let size = max(MemoryLayout<AudioHardwareIOProcStreamUsage>.size, offset + count * MemoryLayout<UInt32>.size)
        let raw = UnsafeMutableRawPointer.allocate(byteCount: size, alignment: MemoryLayout<AudioHardwareIOProcStreamUsage>.alignment)
        defer { raw.deallocate() }
        raw.initializeMemory(as: UInt8.self, repeating: 0, count: size)
        let usage = raw.assumingMemoryBound(to: AudioHardwareIOProcStreamUsage.self)
        usage.pointee.mIOProc = unsafeBitCast(procID, to: UnsafeMutableRawPointer.self)
        usage.pointee.mNumberStreams = UInt32(count)
        let flags = raw.advanced(by: offset).assumingMemoryBound(to: UInt32.self)
        for index in 0..<count { flags[index] = enabled.contains(index) ? 1 : 0 }
        var address = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyIOProcStreamUsage, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        try check("set stream usage", AudioObjectSetPropertyData(aggregateID, &address, 0, nil, UInt32(size), raw))
    }

    private func property<T: BitwiseCopyable>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector,
                             scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal, initial: T) throws -> T {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        var value = initial
        var size = UInt32(MemoryLayout<T>.size)
        try check("read audio property", AudioObjectGetPropertyData(object, &address, 0, nil, &size, &value))
        return value
    }

    private func check(_ step: String, _ status: OSStatus) throws {
        guard status == noErr else { throw SystemAudioTapError(step: step, status: status) }
    }

    private func tapFormat() throws -> AudioStreamBasicDescription {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioTapPropertyFormat,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var format = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        try check("read tap format", AudioObjectGetPropertyData(tapID, &address, 0, nil, &size, &format))
        return format
    }

    private func defaultOutputUID() throws -> String {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var device = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        try check(
            "default output",
            AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device)
        )
        address.mSelector = kAudioDevicePropertyDeviceUID
        var uid: Unmanaged<CFString>?
        size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        try check("output uid", AudioObjectGetPropertyData(device, &address, 0, nil, &size, &uid))
        guard let uid else { throw SystemAudioTapError(step: "output uid", status: -1) }
        return uid.takeRetainedValue() as String
    }

    private func ownProcessObject() -> AudioObjectID? {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var pid = ProcessInfo.processInfo.processIdentifier
        var object = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        let status = AudioObjectGetPropertyData(
            AudioObjectID(kAudioObjectSystemObject), &address,
            UInt32(MemoryLayout<pid_t>.size), &pid, &size, &object
        )
        return status == noErr && object != kAudioObjectUnknown ? object : nil
    }
}

private final class TapBufferStorage: @unchecked Sendable {
    let buffers: UnsafeMutableAudioBufferListPointer
    init(_ buffers: UnsafeMutableAudioBufferListPointer) { self.buffers = buffers }
    deinit { buffers.unsafeMutablePointer.deallocate() }
}
