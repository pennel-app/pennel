import Foundation
import Testing
@testable internal import PennelMobile

@MainActor
struct EngineSmokeTests {
    @Test func qaProfileRequiresThePreviewIdentityAndItsOwnCredentialService() throws {
        let info: [String: Any] = ["PennelQAProfile": "pairing-2026", "PennelKeychainService": "app.pennel.ios.preview.qa.pairing-2026"]
        #expect(try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios.preview", info: info) == "pairing-2026")
        #expect(try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios", info: [:]) == nil)
        #expect(try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios.preview", info: ["PennelKeychainService": "app.pennel.ios.preview"]) == nil)
        for bundle in ["app.pennel.ios", "app.pennel.ios.preview.other", ""] {
            #expect(throws: AppPaths.ConfigurationError.invalidQAProfile) {
                try AppPaths.validatedQAProfile(bundleIdentifier: bundle, info: info)
            }
        }
        for service in ["app.pennel.ios", "app.pennel.ios.preview", "app.pennel.ios.preview.qa.other", ""] {
            #expect(throws: AppPaths.ConfigurationError.invalidQAProfile) {
                try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios.preview", info: ["PennelQAProfile": "pairing-2026", "PennelKeychainService": service])
            }
        }
        #expect(throws: AppPaths.ConfigurationError.invalidQAProfile) {
            try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios.preview", info: ["PennelKeychainService": "app.pennel.ios.preview.qa.orphaned"])
        }
    }

    @Test func qaProfileRejectsEmptyTraversalUnicodeAndUnboundedNames() throws {
        for profile in ["", ".", "..", "../real", "qa/other", "qa\\other", "qa name", "qa_name", "é", String(repeating: "a", count: 65)] {
            #expect(throws: AppPaths.ConfigurationError.invalidQAProfile) {
                try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios.preview", info: ["PennelQAProfile": profile, "PennelKeychainService": "app.pennel.ios.preview.qa.\(profile)"])
            }
        }
        #expect(throws: AppPaths.ConfigurationError.invalidQAProfile) {
            try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios.preview", info: ["PennelQAProfile": true])
        }
        let longest = String(repeating: "Z", count: 64)
        #expect(try AppPaths.validatedQAProfile(bundleIdentifier: "app.pennel.ios.preview", info: ["PennelQAProfile": longest, "PennelKeychainService": "app.pennel.ios.preview.qa.\(longest)"]) == longest)
    }

    @Test func qaPathsAndPreferenceSuitesCannotAliasTheNormalPreviewProfile() {
        let support = URL(fileURLWithPath: "/fixture/Library/Application Support", isDirectory: true)
        let documents = URL(fileURLWithPath: "/fixture/Documents", isDirectory: true)
        let caches = URL(fileURLWithPath: "/fixture/Library/Caches", isDirectory: true)
        #expect(AppPaths.scopedDirectory(base: support, name: "Pennel", profile: nil).path == "/fixture/Library/Application Support/Pennel")
        #expect(AppPaths.scopedDirectory(base: documents, name: "Meetings", profile: nil).path == "/fixture/Documents/Meetings")
        #expect(AppPaths.scopedDirectory(base: support, name: "Pennel", profile: "pairing").path == "/fixture/Library/Application Support/QA/pairing/Pennel")
        #expect(AppPaths.scopedDirectory(base: documents, name: "Meetings", profile: "pairing").path == "/fixture/Documents/QA/pairing/Meetings")
        #expect(AppPaths.scopedDirectory(base: caches, name: "handoff", profile: "pairing").path == "/fixture/Library/Caches/QA/pairing/handoff")
        #expect(AppPaths.defaultsSuite(profile: nil) == nil)
        #expect(AppPaths.defaultsSuite(profile: "pairing") == "app.pennel.ios.preview.qa.pairing")
        #expect(AppPaths.defaultsSuite(profile: "pairing") != AppPaths.defaultsSuite(profile: "other"))
    }

    @Test func engine_opens_in_app_container_not_app_group() throws {
        let model = AppModel.shared
        let dataDir = AppPaths.dataDirectory.standardizedFileURL.path

        #expect(dataDir.hasPrefix(URL(fileURLWithPath: NSHomeDirectory()).standardizedFileURL.path))
        #expect(dataDir.hasSuffix("/Library/Application Support/Pennel"))
        #expect(FileManager.default.fileExists(atPath: dataDir))
        #expect(!model.engine.settings().providers.isEmpty)

        // The simulator can return nil for an unprovisioned group; then compare against the identifier itself.
        if let group = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: AppPaths.appGroup) {
            #expect(!dataDir.hasPrefix(group.standardizedFileURL.path))
        } else {
            #expect(!dataDir.contains(AppPaths.appGroup))
            #expect(!dataDir.contains("/Shared/AppGroup/"))
        }
    }

    @Test func meetings_dir_defaults_to_documents_meetings() {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let expected = documents.appendingPathComponent("Meetings").standardizedFileURL.path

        #expect(AppModel.shared.engine.settings().meetingsDir == expected)
        #expect(AppModel.shared.settings.meetingsDir == expected)
        #expect(FileManager.default.fileExists(atPath: expected))
    }
}
