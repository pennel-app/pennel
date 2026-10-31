import Foundation
import PennelMobileKit

/// The core keeps file locks and SQLite open, so it lives in the app's own container. A suspended app holding a
/// lock in the shared App Group container is killed by iOS (`0xdead10cc`); the group only carries handoff files.
enum AppPaths {
    static let appGroup = HandoffStore.appGroup
    enum ConfigurationError: Error, Equatable { case invalidQAProfile }

    static let qaProfile: String? = {
        do { return try validatedQAProfile(bundleIdentifier: Bundle.main.bundleIdentifier, info: Bundle.main.infoDictionary ?? [:]) }
        catch { fatalError("Invalid Preview QA profile configuration.") }
    }()

    static var isQAProfile: Bool { qaProfile != nil }

    static func validatedQAProfile(bundleIdentifier: String?, info: [String: Any]) throws -> String? {
        guard let marker = info["PennelQAProfile"] else {
            if (info["PennelKeychainService"] as? String)?.hasPrefix("app.pennel.ios.preview.qa.") == true { throw ConfigurationError.invalidQAProfile }
            return nil
        }
        guard bundleIdentifier == "app.pennel.ios.preview", let profile = marker as? String,
              !profile.isEmpty, profile.utf8.count <= 64,
              profile.utf8.allSatisfy({ $0 == 45 || (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) }),
              info["PennelKeychainService"] as? String == defaultsSuite(profile: profile) else {
            throw ConfigurationError.invalidQAProfile
        }
        return profile
    }

    static func scopedDirectory(base: URL, name: String, profile: String?) -> URL {
        let root = profile.map { base.appendingPathComponent("QA", isDirectory: true).appendingPathComponent($0, isDirectory: true) } ?? base
        return root.appendingPathComponent(name, isDirectory: true)
    }

    static func defaultsSuite(profile: String?) -> String? { profile.map { "app.pennel.ios.preview.qa.\($0)" } }

    static var defaults: UserDefaults {
        guard let suite = defaultsSuite(profile: qaProfile) else { return .standard }
        guard let value = UserDefaults(suiteName: suite) else { fatalError("Cannot open Preview QA preferences.") }
        return value
    }

    static var keyboardDefaults: UserDefaults? { isQAProfile ? defaults : KeyboardLayout.groupDefaults }

    static var dataDirectory: URL {
        scopedDirectory(base: FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0], name: "Pennel", profile: qaProfile)
    }

    /// Visible in the Files app under "Op mijn iPhone › Pennel".
    static var meetingsDirectory: URL {
        scopedDirectory(base: FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0], name: "Meetings", profile: qaProfile)
    }

    /// Without an App Group container (unsigned builds) the session still runs; only the keyboard cannot reach it.
    static var handoffDirectory: URL {
        if let profile = qaProfile {
            return scopedDirectory(base: FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0], name: "handoff", profile: profile)
        }
        return HandoffStore.groupDirectory
            ?? FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("handoff", isDirectory: true)
    }
}
