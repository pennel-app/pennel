// Status-only probe built from the production Teams reader. It never requests Accessibility
// permission and prints names only when they exactly match an explicitly supplied QA allowlist.
// Build from the repository root:
//   worktree-storage run -- swiftc -swift-version 5 \
//     apps/macos-platform/Sources/PennelMacPlatform/MeetingSpeakerObserver.swift \
//     scripts/probe-teams-speakers.swift -o build/probe-teams-speakers
// Run while a controlled Teams call is visible:
//   build/probe-teams-speakers --samples 20 --interval-ms 250 --qa-name "Pennel QA A"
import AppKit
import ApplicationServices
import Foundation

@main
struct TeamsSpeakerProbe {
    struct Options {
        var samples = 1
        var intervalMs = 250
        var qaNames = Set<String>()

        init?(_ arguments: [String]) {
            var index = 0
            while index < arguments.count {
                guard index + 1 < arguments.count else { return nil }
                let value = arguments[index + 1]
                switch arguments[index] {
                case "--samples":
                    guard let count = Int(value), (1...240).contains(count) else { return nil }
                    samples = count
                case "--interval-ms":
                    guard let duration = Int(value), (250...5_000).contains(duration) else { return nil }
                    intervalMs = duration
                case "--qa-name":
                    guard !value.isEmpty, value.count <= 256 else { return nil }
                    qaNames.insert(value)
                default:
                    return nil
                }
                index += 2
            }
        }
    }

    static func main() {
        guard let options = Options(Array(CommandLine.arguments.dropFirst())) else {
            FileHandle.standardError.write(Data("Usage: probe-teams-speakers [--samples 1...240] [--interval-ms 250...5000] [--qa-name NAME]\n".utf8))
            exit(2)
        }
        let started = ProcessInfo.processInfo.systemUptime
        var previousWindowId: String?
        for index in 0..<options.samples {
            let sampleStarted = ProcessInfo.processInfo.systemUptime
            let snapshot = MeetingSpeakerObserver.snapshot(appId: MeetingSpeakerObserver.teamsBundleId)
            var result: [String: Any] = [
                "sample": index,
                "elapsed_ms": Int((sampleStarted - started) * 1_000),
                "read_duration_ms": Int((ProcessInfo.processInfo.systemUptime - sampleStarted) * 1_000),
                "trusted": AXIsProcessTrusted(),
                "available": snapshot.available,
                "unavailable_reason": snapshot.unavailableReason as Any? ?? NSNull(),
                "participant_count": snapshot.participants.count,
                "speaking_count": snapshot.speaking.count,
                "duplicate_names": snapshot.duplicateNames,
                "has_process_identity": snapshot.processId != nil && snapshot.processStartedAtMs != nil,
                "has_window_identity": snapshot.windowId != nil,
                "same_window_as_previous": previousWindowId.flatMap { previous in snapshot.windowId.map { $0 == previous } } as Any? ?? NSNull(),
            ]
            if !options.qaNames.isEmpty {
                result["qa_participants"] = snapshot.participants.filter(options.qaNames.contains)
                result["qa_speaking"] = snapshot.speaking.filter(options.qaNames.contains)
                result["other_participant_count"] = snapshot.participants.filter { !options.qaNames.contains($0) }.count
            }
            let data = try! JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
            print(String(decoding: data, as: UTF8.self))
            fflush(stdout)
            previousWindowId = snapshot.windowId
            if index + 1 < options.samples { Thread.sleep(forTimeInterval: Double(options.intervalMs) / 1_000) }
        }
    }
}
