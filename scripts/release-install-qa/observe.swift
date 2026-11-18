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

func descendants(_ roots: [AXUIElement]) -> [(AXUIElement, String, [String])] {
    var output: [(AXUIElement, String, [String])] = []
    var queue = roots
    var seen = Set<AXUIElement>()
    while let element = queue.popLast(), seen.count < 1400 {
        guard seen.insert(element).inserted else { continue }
        let role = attribute(element, kAXRoleAttribute as CFString) as? String ?? ""
        let secure = (attribute(element, kAXSubroleAttribute as CFString) as? String) == "AXSecureTextField"
        var names = [kAXTitleAttribute, kAXDescriptionAttribute, kAXPlaceholderValueAttribute]
        if !secure { names.append(kAXValueAttribute) }
        let text = names.compactMap {
            attribute(element, $0 as CFString) as? String
        }
        output.append((element, role, text))
        queue += attribute(element, kAXChildrenAttribute as CFString) as? [AXUIElement] ?? []
    }
    return output
}

func controls(_ app: AXUIElement) -> [(AXUIElement, String, [String])] {
    descendants(attribute(app, kAXWindowsAttribute as CFString) as? [AXUIElement] ?? [])
}

func openWorkspace(_ root: AXUIElement) -> Bool {
    guard let extras = attribute(root, "AXExtrasMenuBar" as CFString),
          CFGetTypeID(extras) == AXUIElementGetTypeID() else { return false }
    let menuBar = unsafeBitCast(extras, to: AXUIElement.self)
    let items = attribute(menuBar, kAXChildrenAttribute as CFString) as? [AXUIElement] ?? []
    guard items.count == 1, let item = items.first,
          AXUIElementPerformAction(item, kAXPressAction as CFString) == .success else { return false }
    Thread.sleep(forTimeInterval: 0.4)
    guard let open = descendants([item]).first(where: { $0.1 == "AXMenuItem" && $0.2.contains("Open Pennel") }) else {
        return false
    }
    return AXUIElementPerformAction(open.0, kAXPressAction as CFString) == .success
}

func press(_ root: AXUIElement, _ label: String, prefix: Bool = false) -> Bool {
    let target = controls(root).first { element, role, names in
        guard role != "AXStaticText", names.contains(where: { $0 == label || (prefix && $0.hasPrefix(label + " ")) }) else {
            return false
        }
        var actions: CFArray?
        return AXUIElementCopyActionNames(element, &actions) == .success && (actions as? [String])?.contains("AXPress") == true
    }
    guard let target, AXUIElementPerformAction(target.0, kAXPressAction as CFString) == .success else { return false }
    Thread.sleep(forTimeInterval: 0.7)
    return true
}

func providerScreen(_ root: AXUIElement, _ app: NSRunningApplication, writeToken: Bool, migrate: Bool) -> [String: Any] {
    var stages: [String] = []
    for label in ["Settings", "Advanced", "Connections", "Groq"] {
        if press(root, label, prefix: label == "Groq") { stages.append(label) }
    }
    var evidence: [String: Any] = ["ordinary_provider_ui_navigation": stages]
    if migrate {
        evidence["app_owned_legacy_migration_requested"] = press(root, "Update saved access")
    }
    let contents = controls(root)
    let field = contents.first { node, _, labels in
        (attribute(node, kAXSubroleAttribute as CFString) as? String) == "AXSecureTextField"
            && labels.contains(where: { $0 == "API key" || $0 == "Paste your key" || $0.contains("(saved)") })
    }
    evidence["provider_key_field_visible"] = field != nil
    guard let field else { return evidence }
    if writeToken {
        let focused = AXUIElementSetAttributeValue(field.0, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success
        let token = "invalid-pennel-qa-" + UUID().uuidString
        let filled = focused && AXUIElementSetAttributeValue(field.0, kAXValueAttribute as CFString, token as CFString) == .success
        evidence["disposable_value_entered"] = filled
        if filled {
            for down in [true, false] {
                CGEvent(keyboardEventSource: nil, virtualKey: 36, keyDown: down)?.postToPid(app.processIdentifier)
            }
            Thread.sleep(forTimeInterval: 1.2)
        }
    }
    let after = controls(root)
    evidence["provider_shows_saved_access"] = after.contains { node, role, labels in
        (role == "AXButton" && labels.contains("Clear")) || (
            (attribute(node, kAXSubroleAttribute as CFString) as? String) == "AXSecureTextField"
                && labels.contains(where: { $0.contains("(saved)") }))
    }
    return evidence
}

func write(_ values: [String: Any]) throws {
    let data = try JSONSerialization.data(withJSONObject: values, options: [.sortedKeys])
    print(String(decoding: data, as: UTF8.self))
}

if action == "first-launch" {
    var status: [String: Any] = ["observer_permitted": AXIsProcessTrusted(), "ordinary_confirmation": false]
    if AXIsProcessTrusted() {
        for identifier in ["com.apple.CoreServicesUIAgent", "com.apple.SecurityAgent", "com.apple.authorizationhost"] {
            for running in NSRunningApplication.runningApplications(withBundleIdentifier: identifier) {
                let root = AXUIElementCreateApplication(running.processIdentifier)
                AXUIElementSetMessagingTimeout(root, 0.5)
                let windows = attribute(root, kAXWindowsAttribute as CFString) as? [AXUIElement] ?? []
                for window in windows {
                    let items = descendants([window])
                    let copy = items.flatMap { $0.2 }.joined(separator: " ").lowercased()
                    guard !copy.isEmpty else { continue }
                    status["system_dialog_observed"] = true
                    if copy.contains("keychain") { status["keychain_dialog"] = true }
                    if copy.contains("password") { status["password_dialog"] = true }
                    if copy.contains("microphone") { status["microphone_dialog"] = true }
                    if copy.contains("pennel") { status["pennel_named"] = true }
                    // The ordinary first-open question follows successful notarized Gatekeeper assessment.
                    guard identifier == "com.apple.CoreServicesUIAgent", copy.contains("pennel"),
                          copy.contains("downloaded from the internet"), !copy.contains("malware"),
                          !copy.contains("cannot verify"), !copy.contains("damaged"),
                          let open = items.first(where: { $0.1 == "AXButton" && $0.2.contains("Open") }) else { continue }
                    status["ordinary_confirmation"] = AXUIElementPerformAction(open.0, kAXPressAction as CFString) == .success
                }
            }
        }
    }
    try write(status)
    exit(0)
}
guard let app = installedApp() else {
    try write(["installed_app_running": false])
    exit(2)
}
if action == "quit" || action == "stop-stalled" {
    let requested = action == "quit" ? app.terminate() : app.forceTerminate()
    guard requested else { exit(3) }
    for _ in 0..<80 where installedApp() != nil {
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
    }
    exit(installedApp() == nil ? 0 : 4)
}

var evidence: [String: Any] = [
    "installed_app_running": true, "finished_launching": app.isFinishedLaunching,
    "ax_observer_already_permitted": AXIsProcessTrusted(), "pid": Int(app.processIdentifier)
]
if AXIsProcessTrusted(), app.isFinishedLaunching {
    let root = AXUIElementCreateApplication(app.processIdentifier)
    AXUIElementSetMessagingTimeout(root, 0.5)
    var initial = controls(root)
    if action != "inspect" && !initial.contains(where: { $0.1 == "AXWebArea" }) {
        evidence["opened_workspace_from_tray"] = openWorkspace(root)
        Thread.sleep(forTimeInterval: 1)
        initial = controls(root)
    }
    evidence["app_window_count"] = (attribute(root, kAXWindowsAttribute as CFString) as? [AXUIElement])?.count ?? 0
    evidence["web_content"] = initial.contains { $0.1 == "AXWebArea" }
    let visible = initial.flatMap { $0.2 }
    evidence["first_use_screen"] = visible.contains(where: { $0.contains("Welcome to Pennel") || $0 == "Choose your language" || $0 == "Continue in English" })
    if action.hasPrefix("credential-") {
        evidence.merge(providerScreen(root, app, writeToken: action == "credential-legacy-save", migrate: action == "credential-migrate")) { _, new in new }
    }
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
