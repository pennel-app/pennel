import Foundation
import LocalAuthentication
import Security
import XCTest
@testable import PennelMacPlatform

private final class FakeCredentialBackend: CredentialStoreBackend, @unchecked Sendable {
    var items: [String: Data] = [:]
    var failures: [String: OSStatus] = [:]
    var events: [String] = []
    var readOverride: ((CredentialStorage, String, String) -> CredentialBytes?)?
    var beforeLegacyRead: (() -> Void)?
    var beforeAdd: ((String, String, Data) -> OSStatus?)?

    func key(_ storage: CredentialStorage, _ service: String, _ account: String) -> String {
        "\(storage == .protected ? "p" : "l")|\(service)|\(account)"
    }
    func seed(_ storage: CredentialStorage, _ service: String, _ account: String, _ value: String) {
        items[key(storage, service, account)] = Data(value.utf8)
    }
    func value(_ storage: CredentialStorage, _ service: String, _ account: String) -> String? {
        items[key(storage, service, account)].flatMap { String(data: $0, encoding: .utf8) }
    }
    func read(_ storage: CredentialStorage, service: String, account: String, interactive: Bool) -> CredentialBytes {
        let event = "read|\(key(storage, service, account))"
        events.append(event + "|\(interactive)")
        if storage == .legacy { beforeLegacyRead?() }
        if let override = readOverride?(storage, service, account) { return override }
        if let failure = failures[event] { return CredentialBytes(status: failure, data: nil) }
        let data = items[key(storage, service, account)]
        return CredentialBytes(status: data == nil ? errSecItemNotFound : errSecSuccess, data: data)
    }
    func presence(_ storage: CredentialStorage, service: String, account: String) -> OSStatus {
        let event = "has|\(key(storage, service, account))"
        events.append(event)
        return failures[event] ?? (items[key(storage, service, account)] == nil ? errSecItemNotFound : errSecSuccess)
    }
    func accounts(_ storage: CredentialStorage, service: String, interactive: Bool) -> CredentialAccounts {
        let prefix = "\(storage == .protected ? "p" : "l")|\(service)|"
        let event = "accounts|\(prefix)"
        events.append(event + "\(interactive)")
        if let failure = failures[event] { return CredentialAccounts(status: failure, accounts: []) }
        let accounts = items.keys.filter { $0.hasPrefix(prefix) }.map { String($0.dropFirst(prefix.count)) }
        return CredentialAccounts(status: accounts.isEmpty ? errSecItemNotFound : errSecSuccess, accounts: accounts)
    }
    func add(service: String, account: String, data: Data) -> OSStatus {
        let event = "add|\(key(.protected, service, account))"
        events.append(event)
        if let override = beforeAdd?(service, account, data) { return override }
        if let failure = failures[event] { return failure }
        let id = key(.protected, service, account)
        if items[id] != nil { return errSecDuplicateItem }
        items[id] = data
        return errSecSuccess
    }
    func update(service: String, account: String, data: Data) -> OSStatus {
        let event = "update|\(key(.protected, service, account))"
        events.append(event)
        if let failure = failures[event] { return failure }
        let id = key(.protected, service, account)
        guard items[id] != nil else { return errSecItemNotFound }
        items[id] = data
        return errSecSuccess
    }
    func delete(service: String, account: String) -> OSStatus {
        let event = "delete|\(key(.protected, service, account))"
        events.append(event)
        if let failure = failures[event] { return failure }
        return items.removeValue(forKey: key(.protected, service, account)) == nil ? errSecItemNotFound : errSecSuccess
    }
}

private final class SendableCredentialStore: @unchecked Sendable {
    let store: MacCredentialStore
    init(_ backend: FakeCredentialBackend) { store = MacCredentialStore(backend: backend) }
}

private final class CredentialReplies: @unchecked Sendable {
    private let lock = NSLock()
    private var replies: [CredentialStoreReply] = []
    func append(_ reply: CredentialStoreReply) {
        lock.lock()
        defer { lock.unlock() }
        replies.append(reply)
    }
    func snapshot() -> [CredentialStoreReply] {
        lock.lock()
        defer { lock.unlock() }
        return replies
    }
}

final class CredentialStoreTests: XCTestCase {
    private let service = "dev.pennel.Pennel"
    private let old = "dev.musli.Musli"
    private var state: String { service + ".credential-state.v1" }
    private func request(_ operation: String, account: String? = nil, value: String? = nil, providers: [String] = ["provider"]) -> CredentialStoreRequest {
        CredentialStoreRequest(operation: operation, service: service, account: account, value: value, providers: providers)
    }

    func testProtectedQueriesArePrivateDeviceOnlyAndNonInteractive() {
        let query = CredentialQueries.creation(service: service, account: "provider", data: Data("synthetic".utf8))
        XCTAssertEqual(query[kSecUseDataProtectionKeychain as String] as? Bool, true)
        XCTAssertEqual(query[kSecAttrSynchronizable as String] as? Bool, false)
        XCTAssertEqual(query[kSecAttrAccessible as String] as? String, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        XCTAssertNil(query[kSecAttrAccessGroup as String], "The signed app's authorized private group is the default")
        XCTAssertTrue((query[kSecUseAuthenticationContext as String] as? LAContext)?.interactionNotAllowed == true)
        let legacy = CredentialQueries.match(.legacy, service: service, interactive: true)
        XCTAssertEqual(legacy[kSecUseDataProtectionKeychain as String] as? Bool, false)
        XCTAssertNil(legacy[kSecUseAuthenticationContext as String])
    }

    private func entitlements(identifier: String = "dev.pennel.Pennel", prefix: String = "PROFILEPFX1") -> [String: Any] {
        ["com.apple.application-identifier": prefix + "." + identifier,
         "com.apple.developer.team-identifier": "B538Q2K76C",
         "keychain-access-groups": [prefix + "." + identifier]]
    }

    func testAuthorizationAcceptsProfileDerivedPrefixWithoutRequiringSandbox() {
        XCTAssertEqual(CredentialAuthorization.validate(service: service, signingTeam: "B538Q2K76C", signingIdentifier: service,
                                                        entitlements: entitlements()), errSecSuccess)
        XCTAssertEqual(CredentialAuthorization.validate(service: state, signingTeam: "B538Q2K76C", signingIdentifier: service,
                                                        entitlements: entitlements()), errSecSuccess)
        let preview = service + ".preview"
        XCTAssertEqual(CredentialAuthorization.validate(service: preview, signingTeam: "B538Q2K76C", signingIdentifier: preview,
                                                        entitlements: entitlements(identifier: preview)), errSecSuccess)
    }

    func testAuthorizationRejectsMissingEntitlementsWrongTeamsAndNonprivateGroups() {
        XCTAssertEqual(CredentialAuthorization.validate(service: service, signingTeam: "B538Q2K76C", signingIdentifier: service,
                                                        entitlements: [:]), errSecMissingEntitlement)
        XCTAssertEqual(CredentialAuthorization.validate(service: service, signingTeam: "OTHERTEAM", signingIdentifier: service,
                                                        entitlements: entitlements()), errSecMissingEntitlement)
        var wrongClaim = entitlements()
        wrongClaim["com.apple.developer.team-identifier"] = "OTHERTEAM"
        var wrongGroup = entitlements()
        wrongGroup["keychain-access-groups"] = ["PROFILEPFX1.dev.pennel.Pennel.preview"]
        var sharedGroup = entitlements()
        sharedGroup["keychain-access-groups"] = ["PROFILEPFX1.shared", "PROFILEPFX1.dev.pennel.Pennel"]
        var wildcard = entitlements()
        wildcard["keychain-access-groups"] = ["PROFILEPFX1.*"]
        for claims in [wrongClaim, wrongGroup, sharedGroup, wildcard, entitlements(prefix: "")] {
            XCTAssertEqual(CredentialAuthorization.validate(service: service, signingTeam: "B538Q2K76C", signingIdentifier: service,
                                                            entitlements: claims), errSecMissingEntitlement)
        }
    }

    func testAuthorizationRejectsCanonicalPreviewIdentityOverlap() {
        let preview = service + ".preview"
        XCTAssertEqual(CredentialAuthorization.validate(service: service, signingTeam: "B538Q2K76C", signingIdentifier: preview,
                                                        entitlements: entitlements(identifier: preview)), errSecMissingEntitlement)
        XCTAssertEqual(CredentialAuthorization.validate(service: preview, signingTeam: "B538Q2K76C", signingIdentifier: service,
                                                        entitlements: entitlements()), errSecMissingEntitlement)
        XCTAssertEqual(CredentialAuthorization.validate(service: service, signingTeam: nil, signingIdentifier: service,
                                                        entitlements: entitlements()), errSecMissingEntitlement)
    }

    func testStatusFindsAllCanonicalAccountsAndApplicableMusliWithoutReadingSecrets() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "pennel-cloud-refresh", "old-refresh")
        backend.seed(.legacy, old, "provider", "old-provider")
        backend.seed(.legacy, old, "unconfigured", "do-not-import")
        let store = MacCredentialStore(backend: backend)
        let status = store.perform(request("status"))
        XCTAssertEqual(status.status, "migration_required")
        XCTAssertEqual(status.pending, 2)
        XCTAssertNil(status.value)
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|") })
        XCTAssertFalse(backend.events.contains { $0.hasSuffix("|true") })
        let pending = store.perform(request("get", account: "provider"))
        XCTAssertEqual(pending.status, "migration_required")
        XCTAssertTrue(pending.present == true)
        XCTAssertNil(pending.value)
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|") })
    }

    func testNewStoreWinsAndPresenceDoesNotReadProtectedCredentialData() {
        let backend = FakeCredentialBackend()
        backend.seed(.protected, service, "provider", "new")
        backend.seed(.legacy, service, "provider", "old")
        let store = MacCredentialStore(backend: backend)
        XCTAssertTrue(store.perform(request("has", account: "provider")).present == true)
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|p|\(service)|") })
        XCTAssertEqual(store.perform(request("get", account: "provider")).value, "new")
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("has|l|") || $0.hasPrefix("read|l|") })
    }

    func testCloudStatusIncludesDynamicReceiptsAndIgnoresUnrelatedProviderMigration() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "old-provider")
        backend.failures["has|l|\(service)|provider"] = errSecInteractionNotAllowed
        let store = MacCredentialStore(backend: backend)
        XCTAssertEqual(store.perform(request("cloud_status")).status, "ready")
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("has|") && $0.hasSuffix("|provider") })
        backend.seed(.legacy, service, "pennel-cloud-link-receipt-dynamic-device", "old-receipt")
        let pending = store.perform(request("cloud_status"))
        XCTAssertEqual(pending.status, "migration_required")
        XCTAssertEqual(pending.pending, 1)
        XCTAssertNil(pending.value)
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|") })
        backend.failures["accounts|p|\(service)|"] = errSecMissingEntitlement
        XCTAssertEqual(store.perform(request("cloud_status")).status, "unavailable")
    }

    func testAuthorizedMigrationCopiesOnceAndRetainsSourcesAcrossStoreRestart() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "canonical-value")
        backend.seed(.legacy, old, "provider", "older-value")
        backend.seed(.legacy, service, "pennel-cloud-refresh", "refresh")
        let first = MacCredentialStore(backend: backend)
        XCTAssertEqual(first.perform(request("migrate")).status, "ready")
        XCTAssertEqual(backend.value(.protected, service, "provider"), "canonical-value")
        XCTAssertEqual(backend.value(.protected, service, "pennel-cloud-refresh"), "refresh")
        XCTAssertEqual(backend.value(.legacy, service, "provider"), "canonical-value")
        XCTAssertEqual(backend.value(.protected, state, "provider"), "active")
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|\(old)|provider") })
        backend.events = []
        let restarted = MacCredentialStore(backend: backend)
        XCTAssertEqual(restarted.perform(request("get", account: "provider")).value, "canonical-value")
        XCTAssertEqual(restarted.perform(request("migrate")).status, "ready")
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|") })
    }

    func testOnlyMissingCanonicalValueAllowsMusliImportAndPreviewNeverReadsCanonicalOrMusli() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, old, "provider", "musli")
        XCTAssertEqual(MacCredentialStore(backend: backend).perform(request("migrate")).status, "ready")
        XCTAssertEqual(backend.value(.protected, service, "provider"), "musli")
        backend.events = []
        backend.seed(.legacy, service + ".preview", "preview-account", "preview")
        let result = MacCredentialStore(backend: backend).perform(CredentialStoreRequest(operation: "migrate", service: service + ".preview", providers: ["provider"]))
        XCTAssertEqual(result.status, "ready")
        XCTAssertEqual(backend.value(.protected, service + ".preview", "preview-account"), "preview")
        XCTAssertFalse(backend.events.contains { $0.contains("|\(old)|") || $0.contains("|\(service)|") })
    }

    func testDeniedOrLockedCanonicalAccessNeverFallsBack() {
        for (failure, classification) in [(errSecAuthFailed, "denied"), (errSecUserCanceled, "denied"), (errSecInteractionNotAllowed, "locked"), (errSecNotAvailable, "unavailable")] {
            let backend = FakeCredentialBackend()
            backend.seed(.legacy, service, "provider", "canonical")
            backend.seed(.legacy, old, "provider", "old")
            backend.failures["read|l|\(service)|provider"] = failure
            let result = MacCredentialStore(backend: backend).perform(request("migrate"))
            XCTAssertEqual(result.status, classification)
            XCTAssertEqual(result.os_status, failure)
            XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|\(old)|") })
            XCTAssertNil(backend.value(.protected, service, "provider"))
        }
    }

    func testDeniedProtectedReadNeverFallsBackAndFailureIsRetryable() {
        let backend = FakeCredentialBackend()
        backend.seed(.protected, service, "provider", "protected")
        backend.seed(.legacy, service, "provider", "legacy")
        backend.failures["read|p|\(service)|provider"] = errSecInteractionNotAllowed
        let store = MacCredentialStore(backend: backend)
        XCTAssertEqual(store.perform(request("get", account: "provider")).status, "locked")
        XCTAssertFalse(backend.events.contains { $0.contains("|l|") })
        backend.failures = [:]
        XCTAssertEqual(store.perform(request("get", account: "provider")).value, "protected")
    }

    func testFailedMigrationWriteRetainsSourceAndDoesNotMarkComplete() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "original")
        backend.failures["add|p|\(service)|provider"] = errSecNotAvailable
        let store = MacCredentialStore(backend: backend)
        XCTAssertEqual(store.perform(request("migrate")).status, "unavailable")
        XCTAssertNil(backend.value(.protected, state, "provider"))
        XCTAssertEqual(backend.value(.legacy, service, "provider"), "original")
        backend.failures = [:]
        XCTAssertEqual(store.perform(request("migrate")).status, "ready")
    }

    func testMigrationRequiresReadbackAndDoesNotMarkMismatchedCopyComplete() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "original")
        backend.readOverride = { storage, service, account in
            storage == .protected && service == self.service && account == "provider"
                ? CredentialBytes(status: errSecSuccess, data: Data("wrong".utf8)) : nil
        }
        XCTAssertEqual(MacCredentialStore(backend: backend).perform(request("migrate")).status, "unavailable")
        XCTAssertNil(backend.value(.protected, state, "provider"))
        XCTAssertEqual(backend.value(.legacy, service, "provider"), "original")
    }

    func testDuplicateDestinationRaceNeverOverwritesWinner() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "original")
        backend.beforeAdd = { service, account, _ in
            guard service == self.service else { return nil }
            backend.seed(.protected, service, account, "concurrent-winner")
            return errSecDuplicateItem
        }
        XCTAssertEqual(MacCredentialStore(backend: backend).perform(request("migrate")).status, "ready")
        XCTAssertEqual(backend.value(.protected, service, "provider"), "concurrent-winner")
        XCTAssertEqual(backend.value(.protected, state, "provider"), "active")
    }

    func testInterruptedMarkerWriteRetriesWithoutReadingOldSecretAgain() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "original")
        backend.failures["add|p|\(state)|provider"] = errSecNotAvailable
        let store = MacCredentialStore(backend: backend)
        XCTAssertEqual(store.perform(request("migrate")).status, "unavailable")
        XCTAssertEqual(backend.value(.protected, service, "provider"), "original")
        backend.failures = [:]
        backend.events = []
        XCTAssertEqual(MacCredentialStore(backend: backend).perform(request("migrate")).status, "ready")
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|") })
    }

    func testDeleteTombstonePreventsResurrectionEvenWhenPhysicalDeletionFails() {
        let backend = FakeCredentialBackend()
        backend.seed(.protected, service, "provider", "protected")
        backend.seed(.legacy, service, "provider", "legacy")
        backend.failures["delete|p|\(service)|provider"] = errSecNotAvailable
        let store = MacCredentialStore(backend: backend)
        XCTAssertEqual(store.perform(request("set", account: "provider")).status, "unavailable")
        XCTAssertEqual(backend.value(.protected, state, "provider"), "deleted")
        let restarted = MacCredentialStore(backend: backend)
        XCTAssertFalse(restarted.perform(request("get", account: "provider")).present == true)
        XCTAssertEqual(restarted.perform(request("migrate")).status, "ready")
        XCTAssertEqual(backend.value(.protected, service, "provider"), "protected")
        backend.failures = [:]
        XCTAssertEqual(restarted.perform(request("set", account: "provider", value: "replacement")).status, "ready")
        XCTAssertEqual(restarted.perform(request("get", account: "provider")).value, "replacement")
    }

    func testDeletionMarkerFailurePreservesExistingCredential() {
        let backend = FakeCredentialBackend()
        backend.seed(.protected, service, "provider", "protected")
        backend.failures["add|p|\(state)|provider"] = errSecNotAvailable
        let store = MacCredentialStore(backend: backend)
        XCTAssertEqual(store.perform(request("set", account: "provider", value: "")).status, "unavailable")
        XCTAssertEqual(store.perform(request("get", account: "provider")).value, "protected")
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("delete|") })
    }

    func testActiveMarkerSuppressesLegacyAfterDestinationWasRemoved() {
        let backend = FakeCredentialBackend()
        backend.seed(.protected, state, "provider", "active")
        backend.seed(.legacy, service, "provider", "old")
        let store = MacCredentialStore(backend: backend)
        XCTAssertFalse(store.perform(request("get", account: "provider")).present == true)
        XCTAssertEqual(store.perform(request("migrate")).status, "ready")
        XCTAssertFalse(backend.events.contains { $0.hasPrefix("read|l|") })
    }

    func testCloudRefreshRotationSurvivesPendingLegacyApproval() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "legacy")
        backend.seed(.protected, service, "pennel-cloud-refresh", "old-refresh-token")
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        backend.beforeLegacyRead = { entered.signal(); _ = release.wait(timeout: .now() + 3) }
        let wrapper = SendableCredentialStore(backend)
        XCTAssertEqual(wrapper.store.perform(request("get", account: "pennel-cloud-refresh")).value, "old-refresh-token")
        let migrated = expectation(description: "migration finished")
        let rotated = expectation(description: "refresh rotation completes while approval is pending")
        let replies = CredentialReplies()
        let migrationRequest = request("migrate")
        let reread = request("get", account: "pennel-cloud-refresh")
        let rotate = request("set", account: "pennel-cloud-refresh", value: "rotated-refresh-token")
        DispatchQueue.global().async {
            _ = wrapper.store.perform(migrationRequest)
            migrated.fulfill()
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 3), .success)
        let eventCount = backend.events.count
        DispatchQueue.global().async {
            replies.append(wrapper.store.perform(reread))
            replies.append(wrapper.store.perform(rotate))
            rotated.fulfill()
        }
        wait(for: [rotated], timeout: 0.2)
        XCTAssertEqual(replies.snapshot().map(\.status), ["ready", "ready"])
        XCTAssertEqual(replies.snapshot().first?.value, "old-refresh-token")
        XCTAssertFalse(backend.events.dropFirst(eventCount).contains { $0.contains("|l|") })
        release.signal()
        wait(for: [migrated], timeout: 5)
        XCTAssertEqual(wrapper.store.perform(request("get", account: "pennel-cloud-refresh")).value, "rotated-refresh-token")
        XCTAssertEqual(wrapper.store.perform(request("get", account: "provider")).value, "legacy")
    }

    func testDuplicateMigrationIsRejectedWhileOrdinaryStatusRemainsAvailable() {
        let backend = FakeCredentialBackend()
        backend.seed(.legacy, service, "provider", "legacy")
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        backend.beforeLegacyRead = { entered.signal(); _ = release.wait(timeout: .now() + 3) }
        let wrapper = SendableCredentialStore(backend)
        let migrated = expectation(description: "migration finished")
        let migrationRequest = request("migrate")
        DispatchQueue.global().async {
            _ = wrapper.store.perform(migrationRequest)
            migrated.fulfill()
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 3), .success)
        let eventCount = backend.events.count
        XCTAssertEqual(wrapper.store.perform(migrationRequest).status, "migration_required")
        XCTAssertEqual(backend.events.count, eventCount)
        XCTAssertEqual(wrapper.store.perform(request("status")).status, "migration_required")
        XCTAssertTrue(wrapper.store.perform(request("has", account: "provider")).present == true)
        XCTAssertEqual(wrapper.store.perform(request("get", account: "provider")).status, "migration_required")
        release.signal()
        wait(for: [migrated], timeout: 5)
        XCTAssertEqual(wrapper.store.perform(request("status")).status, "ready")
    }

    func testManualSaveOrDeleteDuringConsentWinsOverLegacyImport() {
        for replacement: String? in ["manual-new-value", nil] {
            let backend = FakeCredentialBackend()
            backend.seed(.legacy, service, "provider", "legacy")
            let entered = DispatchSemaphore(value: 0)
            let release = DispatchSemaphore(value: 0)
            backend.beforeLegacyRead = { entered.signal(); _ = release.wait(timeout: .now() + 3) }
            let wrapper = SendableCredentialStore(backend)
            let migrated = expectation(description: "migration finished")
            let migrationRequest = request("migrate")
            DispatchQueue.global().async {
                _ = wrapper.store.perform(migrationRequest)
                migrated.fulfill()
            }
            XCTAssertEqual(entered.wait(timeout: .now() + 3), .success)
            XCTAssertEqual(wrapper.store.perform(request("set", account: "provider", value: replacement)).status, "ready")
            release.signal()
            wait(for: [migrated], timeout: 5)
            let result = wrapper.store.perform(request("get", account: "provider"))
            XCTAssertEqual(result.status, "ready")
            XCTAssertEqual(result.value, replacement)
            XCTAssertEqual(result.present, replacement != nil)
            XCTAssertEqual(backend.value(.protected, state, "provider"), replacement == nil ? "deleted" : "active")
            XCTAssertEqual(backend.value(.legacy, service, "provider"), "legacy")
        }
    }

    func testOverlappingNoninteractiveReadsSerializeAndBothSucceed() {
        let backend = FakeCredentialBackend()
        backend.seed(.protected, service, "provider", "protected-value")
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        var blockedFirstRead = false
        backend.readOverride = { storage, service, account in
            if storage == .protected && service == self.service && account == "provider" && !blockedFirstRead {
                blockedFirstRead = true
                entered.signal()
                _ = release.wait(timeout: .now() + 3)
            }
            return nil
        }
        let wrapper = SendableCredentialStore(backend)
        let firstFinished = expectation(description: "first read finished")
        let secondFinished = expectation(description: "second read finished")
        let secondStarted = DispatchSemaphore(value: 0)
        let secondReturned = DispatchSemaphore(value: 0)
        let replies = CredentialReplies()
        let get = request("get", account: "provider")
        DispatchQueue.global().async {
            replies.append(wrapper.store.perform(get))
            firstFinished.fulfill()
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 3), .success)
        DispatchQueue.global().async {
            secondStarted.signal()
            replies.append(wrapper.store.perform(get))
            secondReturned.signal()
            secondFinished.fulfill()
        }
        XCTAssertEqual(secondStarted.wait(timeout: .now() + 3), .success)
        XCTAssertEqual(secondReturned.wait(timeout: .now() + 0.05), .timedOut)
        release.signal()
        wait(for: [firstFinished, secondFinished], timeout: 5)
        XCTAssertEqual(replies.snapshot().map(\.status), ["ready", "ready"])
        XCTAssertEqual(replies.snapshot().map(\.value), ["protected-value", "protected-value"])
    }

    func testNewInstallationChecksProtectedStoreAvailabilityAndRejectsArbitraryServices() {
        let backend = FakeCredentialBackend()
        backend.failures["accounts|p|\(service)|"] = errSecMissingEntitlement
        let store = MacCredentialStore(backend: backend)
        XCTAssertEqual(store.perform(request("status", providers: [])).status, "unavailable")
        backend.events = []
        let invalid = store.perform(CredentialStoreRequest(operation: "get", service: "another-app", account: "provider"))
        XCTAssertEqual(invalid.status, "unavailable")
        XCTAssertEqual(invalid.os_status, errSecParam)
        XCTAssertTrue(backend.events.isEmpty)
    }

    func testOnlyGetResponsesContainValues() throws {
        let backend = FakeCredentialBackend()
        let store = MacCredentialStore(backend: backend)
        XCTAssertNil(store.perform(request("set", account: "provider", value: "synthetic")).value)
        for operation in ["status", "cloud_status", "has", "migrate"] {
            let result = store.perform(request(operation, account: "provider"))
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(result)) as? [String: Any])
            XCTAssertNil(object["value"])
        }
        XCTAssertEqual(store.perform(request("get", account: "provider")).value, "synthetic")
    }
}
