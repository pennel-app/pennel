import AppKit
import ApplicationServices
import Foundation

let action = CommandLine.arguments[1]
let path = "/Applications/Pennel.app"

func installedApp() -> NSRunningApplication? {
    NSRunningApplication.runningApplications(withBundleIdentifier: "dev.pennel.Pennel").first {
        $0.bundleURL?.standardizedFileURL.path == path && !$0.isTerminated
    }
}

func attribute(_ element: AXUIElement, _ name: CFString) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name, &value) == .success else { return nil }
    return value
}

func controls(_ root: AXUIElement) -> [(AXUIElement, String, [String])] {
    var output: [(AXUIElement, String, [String])] = []
    var queue = attribute(root, kAXWindowsAttribute as CFString) as? [AXUIElement] ?? []
    var seen = Set<AXUIElement>()
    while let element = queue.popLast(), seen.count < 1400 {
        guard seen.insert(element).inserted else { continue }
        let role = attribute(element, kAXRoleAttribute as CFString) as? String ?? ""
        let text = [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].compactMap {
            attribute(element, $0 as CFString) as? String
        }
        output.append((element, role, text))
        queue += attribute(element, kAXChildrenAttribute as CFString) as? [AXUIElement] ?? []
    }
    return output
}

func write(_ values: [String: Any]) throws {
    let data = try JSONSerialization.data(withJSONObject: values, options: [.sortedKeys])
    print(String(decoding: data, as: UTF8.self))
}

guard let app = installedApp() else {
    try write(["installed_app_running": false])
    exit(2)
}
if action == "quit" {
    guard app.terminate() else { exit(3) }
    for _ in 0..<80 where installedApp() != nil {
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
    }
    exit(installedApp() == nil ? 0 : 4)
}

var evidence: [String: Any] = [
    "installed_app_running": true, "finished_launching": app.isFinishedLaunching,
    "ax_observer_already_permitted": AXIsProcessTrusted()
]
if AXIsProcessTrusted() {
    let root = AXUIElementCreateApplication(app.processIdentifier)
    AXUIElementSetMessagingTimeout(root, 1)
    let initial = controls(root)
    evidence["app_window_count"] = (attribute(root, kAXWindowsAttribute as CFString) as? [AXUIElement])?.count ?? 0
    evidence["web_content"] = initial.contains { $0.1 == "AXWebArea" }
    if action == "fixtures" {
        var observed: [String] = initial.flatMap { $0.2 }
        for labels in [["Dictation"], ["History", "All history"], ["Meetings"]] {
            let current = controls(root)
            guard let hit = current.first(where: {
                $0.1 == "AXButton" && $0.2.contains(where: labels.contains)
            }) else { continue }
            if AXUIElementPerformAction(hit.0, kAXPressAction as CFString) == .success {
                Thread.sleep(forTimeInterval: 1)
                observed += controls(root).flatMap { $0.2 }
            }
        }
        for (key, text) in [("history_visible", "Homebrew QA history sample"),
                            ("meeting_visible", "Homebrew QA meeting")] {
            evidence[key] = observed.contains(where: { $0.contains(text) })
        }
    }
}
try write(evidence)
