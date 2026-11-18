import AudioToolbox
import AVFoundation
import Foundation

struct Failure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}
func check(_ status: OSStatus, _ step: String) throws {
    guard status == noErr else { throw Failure("\(step) returned \(status)") }
}
func deviceString(_ device: AudioObjectID, _ selector: AudioObjectPropertySelector) throws -> String {
    var address = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var value: Unmanaged<CFString>?
    var bytes = UInt32(MemoryLayout.size(ofValue: value))
    try check(AudioObjectGetPropertyData(device, &address, 0, nil, &bytes, &value), "device string")
    guard let value else { throw Failure("empty device metadata") }
    return value.takeRetainedValue() as String
}
func nullDevice() throws -> String {
    var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var bytes: UInt32 = 0
    let system = AudioObjectID(kAudioObjectSystemObject)
    try check(AudioObjectGetPropertyDataSize(system, &address, 0, nil, &bytes), "device count")
    var devices = [AudioDeviceID](repeating: 0, count: Int(bytes) / MemoryLayout<AudioDeviceID>.size)
    try check(AudioObjectGetPropertyData(system, &address, 0, nil, &bytes, &devices), "device list")
    let matches = try devices.filter { try deviceString($0, kAudioObjectPropertyName) == "Null Audio Device" }
    guard matches.count == 1 else { throw Failure("one explicit Null Audio Device required") }
    return try deviceString(matches[0], kAudioDevicePropertyDeviceUID)
}
func bind(_ queue: AudioQueueRef, _ uid: String) throws {
    var value = uid as CFString
    try check(AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice, &value, UInt32(MemoryLayout<CFString>.size)), "select null device")
    var current: Unmanaged<CFString>?
    var bytes = UInt32(MemoryLayout.size(ofValue: current))
    try check(AudioQueueGetProperty(queue, kAudioQueueProperty_CurrentDevice, &current, &bytes), "read selected device")
    guard let current, current.takeRetainedValue() as String == uid else { throw Failure("audio queue did not bind to the Null device") }
}

let rate = 44_100.0
let omega = 2 * Double.pi * 617.0 / rate
let bufferFrames = 2048
final class Measurement {
    let lock = NSLock()
    var capturedFrames = 0
    var renderedFrames = 0
    var peakPCM = 0
    var peakTone = 0.0
}
let outputCallback: AudioQueueOutputCallback = { context, queue, buffer in
    let measure = Unmanaged<Measurement>.fromOpaque(context!).takeUnretainedValue()
    let samples = buffer.pointee.mAudioData.assumingMemoryBound(to: Int16.self)
    let frames = Int(buffer.pointee.mAudioDataBytesCapacity) / 4
    measure.lock.lock()
    let start = measure.renderedFrames
    measure.renderedFrames += frames
    measure.lock.unlock()
    for i in 0..<frames {
        let value = Int16(6553 * sin(omega * Double(start + i)))
        samples[2 * i] = value
        samples[2 * i + 1] = value
    }
    buffer.pointee.mAudioDataByteSize = UInt32(frames * 4)
    AudioQueueEnqueueBuffer(queue, buffer, 0, nil)
}
let inputCallback: AudioQueueInputCallback = { context, queue, buffer, _, _, _ in
    let measure = Unmanaged<Measurement>.fromOpaque(context!).takeUnretainedValue()
    let samples = buffer.pointee.mAudioData.assumingMemoryBound(to: Int16.self)
    let frames = Int(buffer.pointee.mAudioDataByteSize) / 4
    var peak = 0, re = 0.0, im = 0.0
    for i in 0..<frames {
        let value = Int(samples[i * 2])
        peak = max(peak, abs(value))
        re += Double(value) * cos(omega * Double(i))
        im += Double(value) * sin(omega * Double(i))
    }
    if frames > 0 {
        measure.lock.lock()
        measure.capturedFrames += frames
        measure.peakPCM = max(measure.peakPCM, peak)
        measure.peakTone = max(measure.peakTone, 2 * hypot(re, im) / Double(frames) / 32768)
        measure.lock.unlock()
    }
    AudioQueueEnqueueBuffer(queue, buffer, 0, nil)
}
func measure() throws {
    let env = ProcessInfo.processInfo.environment
    guard env["RUNNER_ENVIRONMENT"] == "github-hosted", env["GITHUB_REPOSITORY"] == "pennel-app/pennel",
          env["GITHUB_REF"] == "refs/heads/codex/mac-release-install-qa" else { throw Failure("disposable hosted runner required") }
    let uid = try nullDevice()
    var format = AudioStreamBasicDescription(mSampleRate: rate, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
        mBytesPerPacket: 4, mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 2, mBitsPerChannel: 16, mReserved: 0)
    let measure = Measurement(), source = Unmanaged.passUnretained(measure).toOpaque()
    var input: AudioQueueRef?, output: AudioQueueRef?
    print("Host microphone authorization for the diagnostic", AVCaptureDevice.authorizationStatus(for: .audio).rawValue)
    try check(AudioQueueNewInput(&format, inputCallback, source, nil, nil, 0, &input), "create input")
    guard let input else { throw Failure("missing input queue") }
    defer { AudioQueueDispose(input, true) }
    try check(AudioQueueNewOutput(&format, outputCallback, source, nil, nil, 0, &output), "create output")
    guard let output else { throw Failure("missing output queue") }
    defer { AudioQueueDispose(output, true) }
    try bind(input, uid)
    try bind(output, uid)
    for _ in 0..<3 {
        var ib: AudioQueueBufferRef?, ob: AudioQueueBufferRef?
        try check(AudioQueueAllocateBuffer(input, UInt32(bufferFrames * 4), &ib), "allocate input")
        try check(AudioQueueAllocateBuffer(output, UInt32(bufferFrames * 4), &ob), "allocate output")
        if let ib { try check(AudioQueueEnqueueBuffer(input, ib, 0, nil), "enqueue input") }
        if let ob { outputCallback(source, output, ob) }
    }
    try check(AudioQueueStart(input, nil), "start input")
    Thread.sleep(forTimeInterval: 0.3)
    try check(AudioQueueStart(output, nil), "start tone")
    Thread.sleep(forTimeInterval: 1.8)
    try check(AudioQueueStop(output, true), "stop output")
    Thread.sleep(forTimeInterval: 0.2)
    try check(AudioQueueStop(input, true), "stop input")
    measure.lock.lock()
    defer { measure.lock.unlock() }
    print("Known 617 Hz output, both queues bound to Null Audio Device. Rendered frames", measure.renderedFrames,
          "captured frames", measure.capturedFrames, "peak PCM", measure.peakPCM, "peak tone amplitude", measure.peakTone)
    guard measure.renderedFrames > 44100 && measure.capturedFrames > 44100 else { throw Failure("no full signal window") }
}
try measure()
