import AVFoundation
import CoreAudio
import XCTest
@testable import PennelMacPlatform

final class SystemCaptureTimingTests: XCTestCase {
    func testMeasuredTapOnlyDeviceElementSevenMapsToFirstScopeLocalInputBuffer() throws {
        let tap = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: true))
        let layout = try XCTUnwrap(TapInputLayout.resolve(tap: tap.streamDescription.pointee, deviceRate: 48_000,
                                                        hardwareChannels: 0, streamStarts: [7],
                                                        streamFormats: [tap.streamDescription.pointee], bufferChannels: [2]))
        XCTAssertEqual(layout.bufferIndices, [0])
        XCTAssertEqual(layout.streamIndices, [0])
        XCTAssertEqual(layout.format, tap)
        let buffer = try tone(format: tap, frames: 480, position: 0)
        XCTAssertTrue(layout.validBuffers(UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)))
    }

    func testContiguousFullDuplexDeviceElementsNormalizeWithoutIncludingTheMicrophone() throws {
        let tap = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: true))
        let layout = try XCTUnwrap(TapInputLayout.resolve(tap: tap.streamDescription.pointee, deviceRate: 24_000,
                                                        hardwareChannels: 1, streamStarts: [7, 8],
                                                        streamFormats: [description(mono(48_000)), tap.streamDescription.pointee],
                                                        bufferChannels: [1, 2]))
        XCTAssertEqual(layout.bufferIndices, [1])
        XCTAssertEqual(layout.streamIndices, [1])
        XCTAssertEqual(layout.format.sampleRate, 24_000)
    }

    func testGapsOverlapsUnknownHardwareAndUnmatchedChannelTotalsStayAmbiguous() throws {
        let tap = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: true))
        let formats = [description(mono(48_000)), tap.streamDescription.pointee]
        for starts in [[7, 9], [7, 7]] {
            XCTAssertNil(TapInputLayout.resolve(tap: tap.streamDescription.pointee, deviceRate: 48_000,
                                               hardwareChannels: 1, streamStarts: starts, streamFormats: formats, bufferChannels: [1, 2]))
        }
        XCTAssertNil(TapInputLayout.resolve(tap: tap.streamDescription.pointee, deviceRate: 48_000,
                                           hardwareChannels: nil, streamStarts: [7], streamFormats: [tap.streamDescription.pointee], bufferChannels: [2]))
        XCTAssertNil(TapInputLayout.resolve(tap: tap.streamDescription.pointee, deviceRate: 48_000,
                                           hardwareChannels: 1, streamStarts: [7, 8], streamFormats: formats, bufferChannels: [1, 3]))
        var combined = tap.streamDescription.pointee
        combined.mChannelsPerFrame = 3
        combined.mBytesPerFrame = 12
        combined.mBytesPerPacket = 12
        XCTAssertNil(TapInputLayout.resolve(tap: tap.streamDescription.pointee, deviceRate: 48_000,
                                           hardwareChannels: 1, streamStarts: [7], streamFormats: [combined], bufferChannels: [3]))
    }

    func testAggregateRateOverridesStaleBluetoothTapRateWithoutChangingPitchOrPadding() throws {
        let tap = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: true))
        let resolved = try XCTUnwrap(TapInputLayout.resolve(tap: tap.streamDescription.pointee, deviceRate: 24_000,
                                                          hardwareChannels: 1, streamStarts: [1, 2],
                                                          streamFormats: [description(mono(48_000)), tap.streamDescription.pointee],
                                                          bufferChannels: [1, 2]))
        XCTAssertEqual(resolved.bufferIndices, [1])
        XCTAssertEqual(resolved.streamIndices, [1])
        XCTAssertEqual(resolved.format.sampleRate, 24_000)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let captured = SampleCollector()
        let diagnostics = CaptureDiagnostics(started: 0)
        let writer = try TrackWriter(track: 1, sink: { context, _, data, count, _ in
            guard let context, let data else { return }
            Unmanaged<SampleCollector>.fromOpaque(context).takeUnretainedValue().samples.append(contentsOf: UnsafeBufferPointer(start: data, count: count))
        }, context: Unmanaged.passUnretained(captured).toOpaque(), started: 0,
                                    url: directory.appendingPathComponent("audio/system.m4a"), diagnostics: diagnostics, write: { _ in })
        let converter = MonoResampler(from: resolved.format)
        var monitor = CaptureRateMonitor()
        var position = 0
        for size in Array(repeating: [240, 480, 960, 120], count: 40).flatMap({ $0 }) {
            let input = try tone(format: resolved.format, frames: size, position: position)
            let output = try XCTUnwrap(converter.convert(input))
            let time = Double(position) / 24_000
            XCTAssertFalse(monitor.observe(start: time, inputDuration: Double(size) / 24_000, outputSamples: Int(output.frameLength)))
            writer.append(output, capturedAt: time)
            position += size
        }
        XCTAssertNil(writer.finish())
        XCTAssertEqual(Double(captured.samples.count) / 16_000, Double(position) / 24_000, accuracy: 0.01)
        let samples = Array(captured.samples.dropFirst(160).dropLast(160))
        let crossings = zip(samples, samples.dropFirst()).filter { $0 <= 0 && $1 > 0 }.count
        XCTAssertEqual(Double(crossings) * 16_000 / Double(samples.count), 440, accuracy: 2)
        let track = try XCTUnwrap((diagnostics.snapshot()["tracks"] as? [[String: Any]])?.first)
        XCTAssertNil(track["inserted_silence_samples"], "A correct continuous clock must not create catch-up gaps.")
    }

    func testSupportedRatesAndLayoutsMapStreamsInsteadOfAssumingLastBuffers() throws {
        for rate in [16_000.0, 24_000, 44_100, 48_000] {
            for (channels, interleaved) in [(UInt32(1), false), (2, true), (2, false)] {
                let format = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: channels, interleaved: interleaved))
                let buffers = interleaved ? [Int(channels)] : Array(repeating: 1, count: Int(channels))
                let result = try XCTUnwrap(TapInputLayout.resolve(tap: format.streamDescription.pointee, deviceRate: rate,
                                                                hardwareChannels: 0, streamStarts: [1],
                                                                streamFormats: [format.streamDescription.pointee], bufferChannels: buffers))
                XCTAssertEqual(result.format.sampleRate, rate)
                XCTAssertEqual(result.format.channelCount, channels)
                let resampler = MonoResampler(from: result.format)
                var outputFrames = 0
                for index in 0..<10 {
                    let input = try tone(format: result.format, frames: Int(rate / 5), position: Int(rate / 5) * index)
                    XCTAssertTrue(result.validBuffers(UnsafeMutableAudioBufferListPointer(input.mutableAudioBufferList)))
                    outputFrames += Int(try XCTUnwrap(resampler.convert(input)).frameLength)
                }
                XCTAssertEqual(Double(outputFrames) / 16_000, 2, accuracy: 0.04, "\(rate) Hz, \(channels) channels, interleaved=\(interleaved)")
            }
        }
    }

    func testAmbiguousMappingAndInvalidCallbackBuffersAreRejected() throws {
        let format = mono(24_000)
        XCTAssertNil(TapInputLayout.resolve(tap: format.streamDescription.pointee, deviceRate: 24_000, hardwareChannels: 1,
                                           streamStarts: [1], streamFormats: [format.streamDescription.pointee], bufferChannels: [2]))
        let stereo = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 24_000, channels: 2, interleaved: false))
        let layout = try XCTUnwrap(TapInputLayout.resolve(tap: stereo.streamDescription.pointee, deviceRate: 24_000,
                                                        hardwareChannels: 0, streamStarts: [1], streamFormats: [stereo.streamDescription.pointee], bufferChannels: [1, 1]))
        let buffer = try tone(format: stereo, frames: 480, position: 0)
        let list = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)
        XCTAssertTrue(layout.validBuffers(list))
        list[1].mDataByteSize -= 4
        XCTAssertFalse(layout.validBuffers(list), "Planar channels must contain the same number of frames.")
    }

    func testPersistentHalfRateIsDetectedButRealInterruptionsArePreserved() {
        var broken = CaptureRateMonitor()
        var detected = false
        for index in 0..<150 {
            detected = broken.observe(start: Double(index) * 0.02, inputDuration: 0.01, outputSamples: 160) || detected
        }
        XCTAssertTrue(detected)
        var healthy = CaptureRateMonitor()
        for index in 0..<200 {
            let interruption = index < 100 ? 0.0 : 2.0
            XCTAssertFalse(healthy.observe(start: Double(index) * 0.02 + interruption, inputDuration: 0.02, outputSamples: 320))
        }
    }

    func testLargeAndVariableHalfRateCallbacksCannotResetTheMismatchGuard() {
        for durations in [[0.2], [0.02, 0.2, 0.01, 0.5, 0.1], [2.0]] {
            var monitor = CaptureRateMonitor()
            var start = 0.0
            var detected = false
            for index in 0..<120 {
                let duration = durations[index % durations.count]
                detected = monitor.observe(start: start, inputDuration: duration / 2,
                                           outputSamples: Int(duration * 8_000)) || detected
                start += duration
            }
            XCTAssertTrue(detected, "Stable half-rate capture must not be mistaken for repeated interruptions: \(durations)")
        }
    }

    func testGenerationRejectsStoppedCallbacksAndEarlierNotificationWork() {
        var gate = CaptureGeneration()
        let initial = gate.begin()
        XCTAssertTrue(gate.accepts(initial))
        gate.suspend()
        XCTAssertFalse(gate.accepts(initial))
        let burst = (0..<20).map { _ in gate.begin() }
        XCTAssertTrue(burst.dropLast().allSatisfy { !gate.accepts($0) })
        XCTAssertTrue(gate.accepts(burst.last!))
        gate.stop()
        XCTAssertFalse(gate.accepts(burst.last!))
        let restarted = gate.begin()
        XCTAssertFalse(gate.accepts(initial))
        XCTAssertTrue(gate.accepts(restarted))
    }

    func testRouteObserverSurvivesFailedCaptureAndFollowsSameDeviceRateChanges() {
        let routes = FakeRouteProperties()
        let retries = CaptureCounter()
        let observer = SystemAudioRouteObserver(currentOutput: { routes.output }, subscribe: routes.subscribe)
        observer.observe { retries.increment() }
        // The initial tap failed; its property listeners do not exist. The recording's watcher remains.
        routes.fire(object: 100, selector: kAudioDevicePropertyNominalSampleRate)
        XCTAssertEqual(retries.value, 1)
        let stale = routes.callbacks
        routes.output = 200
        routes.fire(object: AudioObjectID(kAudioObjectSystemObject), selector: kAudioHardwarePropertyDefaultOutputDevice)
        XCTAssertEqual(retries.value, 2)
        routes.fire(object: 100, selector: kAudioDevicePropertyNominalSampleRate)
        XCTAssertEqual(retries.value, 2)
        routes.fire(object: 200, selector: kAudioDevicePropertyNominalSampleRate)
        XCTAssertEqual(retries.value, 3)
        observer.stop()
        stale.forEach { $0() }
        routes.fire(object: 200, selector: kAudioDevicePropertyNominalSampleRate)
        XCTAssertEqual(retries.value, 3)
    }

    func testDiagnosticsRetainLatestBoundedEventsAndSafeCounters() throws {
        let diagnostics = CaptureDiagnostics(started: 0)
        for index in 0..<1_100 { diagnostics.event("gap", track: "system", at: Double(index), fields: ["inserted_silence_samples": 1_600]) }
        diagnostics.count(track: "system", key: "inserted_silence_samples", value: 1_600)
        diagnostics.count(track: "system", key: "inserted_silence_samples", value: 3_200)
        let object = diagnostics.snapshot()
        let events = try XCTUnwrap(object["events"] as? [[String: Any]])
        XCTAssertEqual(events.count, 1_024)
        XCTAssertEqual(events.first?["sequence"] as? UInt64, 77)
        XCTAssertEqual(events.last?["sequence"] as? UInt64, 1_100)
        XCTAssertEqual(object["dropped_events"] as? UInt64, 76)
        XCTAssertEqual((object["tracks"] as? [[String: Any]])?.first?["inserted_silence_samples"] as? UInt64, 4_800)
        XCTAssertNoThrow(try JSONSerialization.data(withJSONObject: object))
    }

    func testFinishedWriterRejectsLateCallbacksAndKeepsRealGap() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let diagnostics = CaptureDiagnostics(started: 0)
        let collector = SampleCollector()
        let writer = try TrackWriter(track: 1, sink: { context, _, data, count, _ in
            guard let context, let data else { return }
            Unmanaged<SampleCollector>.fromOpaque(context).takeUnretainedValue().samples.append(contentsOf: UnsafeBufferPointer(start: data, count: count))
        }, context: Unmanaged.passUnretained(collector).toOpaque(), started: 0,
                                    url: directory.appendingPathComponent("system.m4a"), diagnostics: diagnostics, write: { _ in })
        let buffer = try tone(format: MonoResampler.format, frames: 16_000, position: 0)
        writer.append(buffer, capturedAt: 0)
        writer.append(buffer, capturedAt: 3)
        XCTAssertNil(writer.finish())
        let count = collector.samples.count
        writer.append(buffer, capturedAt: 4)
        XCTAssertNil(writer.finish())
        XCTAssertEqual(collector.samples.count, count)
        XCTAssertEqual(count, 64_000)
        XCTAssertTrue(collector.samples[16_000..<48_000].allSatisfy { $0 == 0 })
    }

    private func description(_ format: AVAudioFormat) -> AudioStreamBasicDescription {
        withExtendedLifetime(format) { format.streamDescription.pointee }
    }

    private func mono(_ rate: Double) -> AVAudioFormat {
        AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 1, interleaved: false)!
    }

    private func tone(format: AVAudioFormat, frames: Int, position: Int) throws -> AVAudioPCMBuffer {
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: UInt32(frames)))
        buffer.frameLength = UInt32(frames)
        let channels = Int(format.channelCount)
        let data = try XCTUnwrap(buffer.floatChannelData)
        for frame in 0..<frames {
            let value = Float(sin(Double(position + frame) * 2 * .pi * 440 / format.sampleRate)) * 0.2
            for channel in 0..<channels {
                if format.isInterleaved { data[0][frame * channels + channel] = value }
                else { data[channel][frame] = value }
            }
        }
        return buffer
    }
}

private final class SampleCollector { var samples: [Float] = [] }

private final class FakeRouteProperties {
    var output: AudioObjectID = 100
    private var entries: [UUID: (AudioObjectID, AudioObjectPropertySelector, @Sendable () -> Void)] = [:]
    var callbacks: [@Sendable () -> Void] { entries.values.map { $0.2 } }
    func subscribe(object: AudioObjectID, selector: AudioObjectPropertySelector, handler: @escaping @Sendable () -> Void) -> (() -> Void)? {
        let id = UUID()
        entries[id] = (object, selector, handler)
        return { [weak self] in self?.entries.removeValue(forKey: id) }
    }
    func fire(object: AudioObjectID, selector: AudioObjectPropertySelector) {
        Array(entries.values).filter { $0.0 == object && $0.1 == selector }.forEach { $0.2() }
    }
}

private final class CaptureCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var value: Int { lock.withLock { count } }
    func increment() { lock.withLock { count += 1 } }
}
