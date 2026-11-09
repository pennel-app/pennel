import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation

enum CaptureDeviceTransport {
    static func name(_ code: UInt32) -> String {
        switch code {
        case kAudioDeviceTransportTypeBluetooth, kAudioDeviceTransportTypeBluetoothLE: return "bluetooth"
        case kAudioDeviceTransportTypeBuiltIn: return "built_in"
        case kAudioDeviceTransportTypeUSB: return "usb"
        case kAudioDeviceTransportTypeHDMI, kAudioDeviceTransportTypeDisplayPort: return "display"
        case kAudioDeviceTransportTypeVirtual, kAudioDeviceTransportTypeAggregate: return "virtual"
        default: return "other"
        }
    }

    static func input(_ unit: AudioUnit?) -> String {
        guard let unit else { return "unknown" }
        var device = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        guard AudioUnitGetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &device, &size) == noErr else { return "unknown" }
        var code: UInt32 = 0
        size = UInt32(MemoryLayout<UInt32>.size)
        var address = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyTransportType, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &code) == noErr else { return "unknown" }
        return name(code)
    }
}

struct CaptureRateMonitor {
    private var previousStart: TimeInterval?
    private var previousInputDuration: TimeInterval = 0
    private var previousOutputSamples = 0
    private var previousCadence: TimeInterval = 0
    private var windowElapsed: TimeInterval = 0
    private var windowSamples = 0
    private var badWindows = 0

    mutating func observe(start: TimeInterval, inputDuration: TimeInterval, outputSamples: Int) -> Bool {
        guard start.isFinite, inputDuration.isFinite, inputDuration > 0 else { return true }
        defer {
            previousStart = start
            previousInputDuration = inputDuration
            previousOutputSamples = outputSamples
        }
        guard let previousStart else { return false }
        let interval = start - previousStart
        guard interval > 0 else { return true }
        let gapThreshold = max(0.25, 3 * max(previousInputDuration, previousCadence))
        let interruption = interval - previousInputDuration > gapThreshold
        previousCadence = interval
        if interruption {
            windowElapsed = 0
            windowSamples = 0
            badWindows = 0
            return false
        }
        // Measure the PREVIOUS buffer against acquisition-start deltas. Its declared duration
        // is precisely what may be wrong after Bluetooth negotiation, and cannot define the clock.
        windowElapsed += interval
        windowSamples += previousOutputSamples
        guard windowElapsed >= 1 else { return false }
        let ratio = Double(windowSamples) / 16_000 / windowElapsed
        badWindows = abs(ratio - 1) > 0.1 ? badWindows + 1 : 0
        windowElapsed = 0
        windowSamples = 0
        return badWindows >= 2
    }
}

struct CaptureGeneration {
    private(set) var value: UInt64 = 0
    private(set) var accepting = false

    mutating func begin() -> UInt64 { value &+= 1; accepting = true; return value }
    mutating func suspend() { accepting = false }
    mutating func stop() { accepting = false; value &+= 1 }
    func accepts(_ token: UInt64) -> Bool { accepting && token == value }
}

struct TapInputLayout {
    let bufferIndices: [Int]
    let streamIndices: [Int]
    let format: AVAudioFormat

    static func resolve(tap: AudioStreamBasicDescription, deviceRate: Double, hardwareChannels: Int?,
                        streamStarts: [Int], streamFormats: [AudioStreamBasicDescription], bufferChannels: [Int]) -> Self? {
        guard let hardwareChannels, deviceRate.isFinite, deviceRate > 0, hardwareChannels >= 0,
              tap.mChannelsPerFrame > 0, streamStarts.count == streamFormats.count else { return nil }
        let channels = Int(tap.mChannelsPerFrame)
        guard let channelBase = streamStarts.min(), channelBase > 0,
              streamFormats.allSatisfy({ $0.mChannelsPerFrame > 0 }),
              streamFormats.reduce(0, { $0 + Int($1.mChannelsPerFrame) }) == hardwareChannels + channels,
              bufferChannels.reduce(0, +) == hardwareChannels + channels
        else { return nil }
        // StartingChannel names elements of the owning device; it need not start at one in
        // the input scope's AudioBufferList. Normalize one complete contiguous input layout.
        var expectedStart = channelBase
        for index in streamStarts.indices.sorted(by: { streamStarts[$0] < streamStarts[$1] }) {
            guard streamStarts[index] == expectedStart else { return nil }
            expectedStart += Int(streamFormats[index].mChannelsPerFrame)
        }
        let inputStarts = streamStarts.map { $0 - channelBase + 1 }
        let wanted = hardwareChannels..<(hardwareChannels + channels)
        var offset = 0
        var buffers: [Int] = []
        for (index, count) in bufferChannels.enumerated() {
            guard count > 0 else { return nil }
            let range = offset..<(offset + count)
            if range.overlaps(wanted) {
                guard wanted.contains(range.lowerBound), range.upperBound <= wanted.upperBound else { return nil }
                buffers.append(index)
            }
            offset += count
        }
        guard buffers.reduce(0, { $0 + bufferChannels[$1] }) == channels else { return nil }
        let streams = streamStarts.indices.filter { index in
            let start = inputStarts[index] - 1
            return start >= wanted.lowerBound && start + Int(streamFormats[index].mChannelsPerFrame) <= wanted.upperBound
        }
        guard !streams.isEmpty, streams.reduce(0, { $0 + Int(streamFormats[$1].mChannelsPerFrame) }) == channels else { return nil }
        var expectedChannel = wanted.lowerBound
        for index in streams.sorted(by: { inputStarts[$0] < inputStarts[$1] }) {
            let format = streamFormats[index]
            guard inputStarts[index] - 1 == expectedChannel,
                  format.mFormatID == kAudioFormatLinearPCM, format.mBitsPerChannel == 32,
                  format.mFormatFlags & kAudioFormatFlagIsFloat != 0,
                  format.mFormatFlags & kAudioFormatFlagIsBigEndian == 0 else { return nil }
            expectedChannel += Int(format.mChannelsPerFrame)
        }
        var actual = streamFormats[streams[0]]
        guard actual.mFormatID == kAudioFormatLinearPCM, actual.mBitsPerChannel == 32,
              actual.mFormatFlags & kAudioFormatFlagIsFloat != 0 else { return nil }
        actual.mSampleRate = deviceRate
        actual.mChannelsPerFrame = UInt32(channels)
        let interleaved = buffers.count == 1
        if interleaved { actual.mFormatFlags &= ~kAudioFormatFlagIsNonInterleaved }
        else {
            guard buffers.count == channels, buffers.allSatisfy({ bufferChannels[$0] == 1 }) else { return nil }
            actual.mFormatFlags |= kAudioFormatFlagIsNonInterleaved
        }
        actual.mBytesPerFrame = UInt32((interleaved ? channels : 1) * MemoryLayout<Float>.size)
        actual.mBytesPerPacket = actual.mBytesPerFrame
        actual.mFramesPerPacket = 1
        guard let format = AVAudioFormat(streamDescription: &actual) else { return nil }
        return Self(bufferIndices: buffers, streamIndices: streams, format: format)
    }

    func validBuffers(_ list: UnsafeMutableAudioBufferListPointer) -> Bool {
        var frames: Int?
        for index in bufferIndices {
            guard list.indices.contains(index) else { return false }
            let buffer = list[index]
            let channels = format.isInterleaved ? format.channelCount : 1
            let stride = Int(channels) * MemoryLayout<Float>.size
            guard buffer.mNumberChannels == channels, buffer.mData != nil,
                  buffer.mDataByteSize > 0, Int(buffer.mDataByteSize) % stride == 0 else { return false }
            let count = Int(buffer.mDataByteSize) / stride
            if let frames, frames != count { return false }
            frames = count
        }
        return frames != nil
    }
}

final class CaptureDiagnostics: @unchecked Sendable {
    private let lock = NSLock()
    private let started: TimeInterval
    private var sequence: UInt64 = 0
    private var dropped: UInt64 = 0
    private var events: [[String: Any]] = []
    private var tracks: [String: [String: Any]] = [:]

    init(started: TimeInterval) { self.started = started }

    func event(_ kind: String, track: String, at uptime: TimeInterval? = nil, fields: [String: Any] = [:]) {
        lock.withLock { append(kind, track: track, uptime: uptime, fields: fields) }
    }

    private func append(_ kind: String, track: String, uptime: TimeInterval?, fields: [String: Any]) {
        sequence &+= 1
        var event = fields
        event["sequence"] = sequence
        event["elapsed_ms"] = UInt64(max(0, (uptime ?? ProcessInfo.processInfo.systemUptime) - started) * 1_000)
        event["kind"] = kind
        event["track"] = track
        events.append(event)
        if events.count > 1_024 { events.removeFirst(); dropped &+= 1 }
    }

    func format(track: String, format: AVAudioFormat, transport: String, tapRate: Double? = nil, aggregateRate: Double? = nil) {
        var fields: [String: Any] = ["input_sample_rate": format.sampleRate, "input_channels": format.channelCount,
                                    "input_interleaved": format.isInterleaved, "transport": transport]
        if let tapRate { fields["tap_sample_rate"] = tapRate }
        if let aggregateRate { fields["aggregate_sample_rate"] = aggregateRate }
        lock.withLock {
            var value = tracks[track] ?? [:]
            fields.forEach { value[$0.key] = $0.value }
            value["track"] = track
            value["output_sample_rate"] = 16_000
            value["output_channels"] = 1
            tracks[track] = value
            append("format", track: track, uptime: nil, fields: fields)
        }
    }

    func count(track: String, key: String, value: UInt64) {
        lock.withLock {
            var state = tracks[track] ?? ["track": track]
            state[key] = (state[key] as? UInt64 ?? 0) + value
            tracks[track] = state
        }
    }

    func snapshot() -> [String: Any] {
        lock.withLock {
            ["schema_version": 1, "events": events, "tracks": tracks.keys.sorted().compactMap { tracks[$0] }, "dropped_events": dropped]
        }
    }
}
