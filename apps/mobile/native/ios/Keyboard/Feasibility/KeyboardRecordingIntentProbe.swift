#if PENNEL_KEYBOARD_FEASIBILITY
import AppIntents
import Foundation
import PennelMobileKit
import SwiftUI

/// Compiler/process-routing probe only; this action never opens the microphone or claims recording succeeded.
struct KeyboardRecordingIntentProbe: AudioRecordingIntent {
    static let title: LocalizedStringResource = "Pennel keyboard routing probe"
    static let supportedModes: IntentModes = .background
    static let isDiscoverable = false

    @MainActor
    func perform() async throws -> some IntentResult {
        guard let group = HandoffStore.groupDirectory?.deletingLastPathComponent() else { return .result() }
        let directory = group.appendingPathComponent("Library/Caches/PennelKeyboardQA", isDirectory: true)
        let receipt = Receipt(
            bundleIdentifier: Bundle.main.bundleIdentifier ?? "unknown",
            processIdentifier: ProcessInfo.processInfo.processIdentifier,
            isExtension: Bundle.main.object(forInfoDictionaryKey: "NSExtension") != nil,
            recordingStarted: false, at: Date())
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try JSONEncoder().encode(receipt).write(to: directory.appendingPathComponent("keyboard-intent-probe.json"), options: .atomic)
        return .result()
    }

    private struct Receipt: Encodable {
        let bundleIdentifier: String
        let processIdentifier: Int32
        let isExtension: Bool
        let recordingStarted: Bool
        let at: Date
    }
}

struct KeyboardRecordingIntentProbeButton: View {
    var body: some View {
        Button(intent: KeyboardRecordingIntentProbe()) { Text("Test intent routing") }
            .frame(minHeight: Brand.Spacing.s10 + Brand.Spacing.s1)
            .accessibilityIdentifier("pennel.qa.intentRouting")
    }
}
#endif
