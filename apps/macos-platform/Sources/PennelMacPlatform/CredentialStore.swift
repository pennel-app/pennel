import Foundation
import LocalAuthentication
import Security

enum CredentialStorage { case protected, legacy }

struct CredentialBytes {
    let status: OSStatus
    let data: Data?
}

struct CredentialAccounts {
    let status: OSStatus
    let accounts: [String]
}

protocol CredentialStoreBackend {
    func read(_ storage: CredentialStorage, service: String, account: String, interactive: Bool) -> CredentialBytes
    func presence(_ storage: CredentialStorage, service: String, account: String) -> OSStatus
    func accounts(_ storage: CredentialStorage, service: String, interactive: Bool) -> CredentialAccounts
    func add(service: String, account: String, data: Data) -> OSStatus
    func update(service: String, account: String, data: Data) -> OSStatus
    func delete(service: String, account: String) -> OSStatus
}

enum CredentialQueries {
    static func match(_ storage: CredentialStorage, service: String, account: String? = nil, interactive: Bool = false) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecUseDataProtectionKeychain as String: storage == .protected,
            kSecAttrSynchronizable as String: false,
        ]
        if let account { query[kSecAttrAccount as String] = account }
        if !interactive {
            let context = LAContext()
            context.interactionNotAllowed = true
            query[kSecUseAuthenticationContext as String] = context
        }
        return query
    }

    static func creation(service: String, account: String, data: Data) -> [String: Any] {
        var query = match(.protected, service: service, account: account)
        query[kSecValueData as String] = data
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return query
    }
}

enum CredentialAuthorization {
    private static let personalTeam = "B538Q2K76C"
    private struct SigningIdentity {
        let status: OSStatus
        let team: String?
        let identifier: String?
        let entitlements: [String: Any]
    }

    // Signing identity is immutable for this process. This never reads credentials or asks for authentication.
    private static let identity: SigningIdentity = {
        var code: SecCode?
        var staticCode: SecStaticCode?
        var requirement: SecRequirement?
        var information: CFDictionary?
        let rule = "anchor apple generic and certificate leaf[subject.OU] = \"\(personalTeam)\""
        let failed = SigningIdentity(status: errSecMissingEntitlement, team: nil, identifier: nil, entitlements: [:])
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
              SecRequirementCreateWithString(rule as CFString, [], &requirement) == errSecSuccess,
              let requirement, SecCodeCheckValidity(code, [], requirement) == errSecSuccess,
              SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode,
              SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
              let info = information as? [String: Any] else { return failed }
        return SigningIdentity(status: errSecSuccess,
                               team: info[kSecCodeInfoTeamIdentifier as String] as? String,
                               identifier: info[kSecCodeInfoIdentifier as String] as? String,
                               entitlements: info[kSecCodeInfoEntitlementsDict as String] as? [String: Any] ?? [:])
    }()

    static func expectedIdentifier(service: String) -> String? {
        let suffix = ".credential-state.v1"
        let service = service.hasSuffix(suffix) ? String(service.dropLast(suffix.count)) : service
        switch service {
        case "dev.pennel.Pennel": return service
        case "dev.pennel.Pennel.preview", "dev.pennel.Pennel.capture.preview": return "dev.pennel.Pennel.preview"
        default: return nil
        }
    }

    static func validate(service: String, signingTeam: String?, signingIdentifier: String?, entitlements: [String: Any]) -> OSStatus {
        guard let expected = expectedIdentifier(service: service), signingTeam == personalTeam,
              signingIdentifier == expected,
              entitlements["com.apple.developer.team-identifier"] as? String == personalTeam,
              let appID = entitlements["com.apple.application-identifier"] as? String,
              appID.hasSuffix("." + expected),
              entitlements["keychain-access-groups"] as? [String] == [appID] else { return errSecMissingEntitlement }
        let prefix = appID.dropLast(expected.count + 1)
        guard !prefix.isEmpty, !prefix.contains("."), !prefix.contains("*") else { return errSecMissingEntitlement }
        return errSecSuccess
    }

    static func current(service: String) -> OSStatus {
        guard identity.status == errSecSuccess else { return identity.status }
        return validate(service: service, signingTeam: identity.team, signingIdentifier: identity.identifier, entitlements: identity.entitlements)
    }
}

struct SystemCredentialStoreBackend: CredentialStoreBackend {
    private func query(_ storage: CredentialStorage, service: String, account: String? = nil, interactive: Bool = false) -> (OSStatus, [String: Any]) {
        if storage == .protected {
            let authorized = CredentialAuthorization.current(service: service)
            guard authorized == errSecSuccess else { return (authorized, [:]) }
        }
        var query = CredentialQueries.match(storage, service: service, account: account, interactive: interactive)
        if storage == .legacy {
            var keychain: SecKeychain?
            let status = SecKeychainCopyDomainDefault(.user, &keychain)
            guard status == errSecSuccess, let keychain else {
                return (status == errSecSuccess ? errSecNotAvailable : status, [:])
            }
            query[kSecMatchSearchList as String] = [keychain]
        }
        return (errSecSuccess, query)
    }

    func read(_ storage: CredentialStorage, service: String, account: String, interactive: Bool) -> CredentialBytes {
        let (status, baseAttributes) = query(storage, service: service, account: account, interactive: interactive)
        guard status == errSecSuccess else { return CredentialBytes(status: status, data: nil) }
        var attributes = baseAttributes
        attributes[kSecReturnData as String] = true
        attributes[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let copied = SecItemCopyMatching(attributes as CFDictionary, &result)
        return CredentialBytes(status: copied, data: result as? Data)
    }

    func presence(_ storage: CredentialStorage, service: String, account: String) -> OSStatus {
        let (status, baseAttributes) = query(storage, service: service, account: account)
        guard status == errSecSuccess else { return status }
        var attributes = baseAttributes
        attributes[kSecReturnAttributes as String] = true
        attributes[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        return SecItemCopyMatching(attributes as CFDictionary, &result)
    }

    func accounts(_ storage: CredentialStorage, service: String, interactive: Bool) -> CredentialAccounts {
        let (status, baseAttributes) = query(storage, service: service, interactive: interactive)
        guard status == errSecSuccess else { return CredentialAccounts(status: status, accounts: []) }
        var attributes = baseAttributes
        attributes[kSecReturnAttributes as String] = true
        attributes[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let copied = SecItemCopyMatching(attributes as CFDictionary, &result)
        guard copied == errSecSuccess else { return CredentialAccounts(status: copied, accounts: []) }
        guard let items = result as? [[String: Any]] else { return CredentialAccounts(status: errSecDecode, accounts: []) }
        return CredentialAccounts(status: copied, accounts: items.compactMap { $0[kSecAttrAccount as String] as? String })
    }

    func add(service: String, account: String, data: Data) -> OSStatus {
        let authorized = CredentialAuthorization.current(service: service)
        guard authorized == errSecSuccess else { return authorized }
        return SecItemAdd(CredentialQueries.creation(service: service, account: account, data: data) as CFDictionary, nil)
    }

    func update(service: String, account: String, data: Data) -> OSStatus {
        let authorized = CredentialAuthorization.current(service: service)
        guard authorized == errSecSuccess else { return authorized }
        return SecItemUpdate(CredentialQueries.match(.protected, service: service, account: account) as CFDictionary,
                             [kSecValueData as String: data] as CFDictionary)
    }

    func delete(service: String, account: String) -> OSStatus {
        let authorized = CredentialAuthorization.current(service: service)
        guard authorized == errSecSuccess else { return authorized }
        return SecItemDelete(CredentialQueries.match(.protected, service: service, account: account) as CFDictionary)
    }
}

struct CredentialStoreRequest: Decodable {
    let operation: String
    let service: String
    let account: String?
    let value: String?
    let providers: [String]

    init(operation: String, service: String, account: String? = nil, value: String? = nil, providers: [String] = []) {
        self.operation = operation
        self.service = service
        self.account = account
        self.value = value
        self.providers = providers
    }

    enum CodingKeys: String, CodingKey { case operation, service, account, value, providers }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        operation = try values.decode(String.self, forKey: .operation)
        service = try values.decode(String.self, forKey: .service)
        account = try values.decodeIfPresent(String.self, forKey: .account)
        value = try values.decodeIfPresent(String.self, forKey: .value)
        providers = try values.decodeIfPresent([String].self, forKey: .providers) ?? []
    }
}

struct CredentialStoreReply: Encodable {
    var status: String = "ready"
    var pending: Int = 0
    var value: String?
    var present: Bool?
    var os_status: OSStatus?

    static func failure(_ status: OSStatus, pending: Int = 0) -> Self {
        let name: String
        switch status {
        case errSecInteractionNotAllowed: name = "locked"
        case errSecAuthFailed, errSecUserCanceled: name = "denied"
        default: name = "unavailable"
        }
        return Self(status: name, pending: pending, os_status: status)
    }
}

private struct CredentialStoreFailure: Error { let status: OSStatus }

final class MacCredentialStore {
    // Only noninteractive work and commits share this lock. Legacy consent must not stall an in-flight token rotation.
    private static let mutationLock = NSLock()
    private static var migrationActive = false
    private let backend: CredentialStoreBackend
    private let canonical = "dev.pennel.Pennel"
    private let musli = "dev.musli.Musli"

    init(backend: CredentialStoreBackend = SystemCredentialStoreBackend()) { self.backend = backend }

    private static func serialized<T>(_ work: () throws -> T) rethrows -> T {
        mutationLock.lock()
        defer { mutationLock.unlock() }
        return try work()
    }

    func perform(_ request: CredentialStoreRequest) -> CredentialStoreReply {
        guard [canonical, canonical + ".preview", canonical + ".capture.preview"].contains(request.service),
              request.providers.allSatisfy({ !$0.isEmpty }) else { return .failure(errSecParam) }
        if request.operation == "migrate" { return performMigration(request) }
        return Self.serialized {
            responding {
                switch request.operation {
                case "status", "cloud_status":
                    let pending = try pendingAccounts(request, cloudOnly: request.operation == "cloud_status")
                    return CredentialStoreReply(status: pending.isEmpty ? "ready" : "migration_required", pending: pending.count)
                case "get", "has", "set":
                    guard let account = request.account, !account.isEmpty else { return .failure(errSecParam) }
                    if request.operation == "set" {
                        try set(request.service, account: account, value: request.value)
                        return CredentialStoreReply()
                    }
                    return try lookup(request, account: account, data: request.operation == "get")
                default: return .failure(errSecParam)
                }
            }
        }
    }

    private func responding(_ work: () throws -> CredentialStoreReply) -> CredentialStoreReply {
        do {
            return try work()
        } catch let error as CredentialStoreFailure {
            return .failure(error.status)
        } catch {
            return .failure(errSecDecode)
        }
    }

    private func performMigration(_ request: CredentialStoreRequest) -> CredentialStoreReply {
        let admitted = Self.serialized {
            if Self.migrationActive { return false }
            Self.migrationActive = true
            return true
        }
        guard admitted else { return CredentialStoreReply(status: "migration_required") }
        defer { Self.serialized { Self.migrationActive = false } }
        return responding {
            let accounts = try Self.serialized { try candidates(request, interactive: false) }
            for account in accounts { try migrate(request, account: account) }
            return try Self.serialized {
                let pending = try pendingAccounts(request)
                return CredentialStoreReply(status: pending.isEmpty ? "ready" : "migration_required", pending: pending.count)
            }
        }
    }

    private func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else { throw CredentialStoreFailure(status: status) }
    }

    private func marker(_ service: String, account: String) throws -> String? {
        let result = backend.read(.protected, service: service + ".credential-state.v1", account: account, interactive: false)
        if result.status == errSecItemNotFound { return nil }
        try check(result.status)
        guard let data = result.data, let marker = String(data: data, encoding: .utf8), ["active", "deleted"].contains(marker) else {
            throw CredentialStoreFailure(status: errSecDecode)
        }
        return marker
    }

    private func exists(_ storage: CredentialStorage, service: String, account: String) throws -> Bool {
        let status = backend.presence(storage, service: service, account: account)
        if status == errSecItemNotFound { return false }
        try check(status)
        return true
    }

    private func legacyExists(_ request: CredentialStoreRequest, account: String) throws -> Bool {
        if try exists(.legacy, service: request.service, account: account) { return true }
        return request.service == canonical && request.providers.contains(account)
            ? try exists(.legacy, service: musli, account: account) : false
    }

    private func lookup(_ request: CredentialStoreRequest, account: String, data: Bool) throws -> CredentialStoreReply {
        let state = try marker(request.service, account: account)
        if state == "deleted" { return CredentialStoreReply(present: false) }
        if data {
            let target = backend.read(.protected, service: request.service, account: account, interactive: false)
            if target.status == errSecSuccess {
                guard let bytes = target.data, !bytes.isEmpty, let value = String(data: bytes, encoding: .utf8) else {
                    throw CredentialStoreFailure(status: errSecDecode)
                }
                return CredentialStoreReply(value: value, present: true)
            }
            if target.status != errSecItemNotFound { try check(target.status) }
        } else if try exists(.protected, service: request.service, account: account) {
            return CredentialStoreReply(present: true)
        }
        if state == "active" { return CredentialStoreReply(present: false) }
        let pending = try legacyExists(request, account: account)
        return CredentialStoreReply(status: pending ? "migration_required" : "ready", pending: pending ? 1 : 0, present: pending)
    }

    private func candidates(_ request: CredentialStoreRequest, interactive: Bool, includeProviders: Bool = true) throws -> [String] {
        let result = backend.accounts(.legacy, service: request.service, interactive: interactive)
        if result.status != errSecItemNotFound { try check(result.status) }
        var accounts = Set(result.accounts.filter { !$0.isEmpty })
        if request.service == canonical && includeProviders { accounts.formUnion(request.providers) }
        return accounts.sorted()
    }

    private func pendingAccounts(_ request: CredentialStoreRequest, cloudOnly: Bool = false) throws -> [String] {
        // Probe protected storage even on a new installation, so a missing entitlement isn't reported as ready.
        let target = backend.accounts(.protected, service: request.service, interactive: false)
        if target.status != errSecItemNotFound { try check(target.status) }
        let accounts = try candidates(request, interactive: false, includeProviders: !cloudOnly)
            .filter { !cloudOnly || $0.hasPrefix("pennel-cloud-") }
        return try accounts.filter { account in
            try lookup(request, account: account, data: false).status == "migration_required"
        }
    }

    private func write(_ service: String, account: String, data: Data) throws {
        let updated = backend.update(service: service, account: account, data: data)
        if updated == errSecItemNotFound {
            let added = backend.add(service: service, account: account, data: data)
            if added == errSecDuplicateItem { try check(backend.update(service: service, account: account, data: data)) }
            else { try check(added) }
        } else { try check(updated) }
        let saved = backend.read(.protected, service: service, account: account, interactive: false)
        try check(saved.status)
        guard saved.data == data else { throw CredentialStoreFailure(status: errSecDecode) }
    }

    private func set(_ service: String, account: String, value: String?) throws {
        let stateService = service + ".credential-state.v1"
        guard let value, !value.isEmpty else {
            // Record deletion first. A crash or failed physical deletion must never resurrect retained legacy data.
            try write(stateService, account: account, data: Data("deleted".utf8))
            let deleted = backend.delete(service: service, account: account)
            if deleted != errSecItemNotFound { try check(deleted) }
            return
        }
        try write(service, account: account, data: Data(value.utf8))
        try write(stateService, account: account, data: Data("active".utf8))
    }

    private func resolved(_ request: CredentialStoreRequest, account: String) throws -> Bool {
        let state = try marker(request.service, account: account)
        if state == "deleted" || state == "active" { return true }
        return try exists(.protected, service: request.service, account: account)
    }

    private func migrate(_ request: CredentialStoreRequest, account: String) throws {
        if try Self.serialized({ try resolved(request, account: account) }) { return }
        // Only this explicit legacy read can request consent. DPK reads and writes remain available during the prompt.
        var old = backend.read(.legacy, service: request.service, account: account, interactive: true)
        if try Self.serialized({ try resolved(request, account: account) }) { return }
        if old.status == errSecItemNotFound, request.service == canonical, request.providers.contains(account) {
            old = backend.read(.legacy, service: musli, account: account, interactive: true)
        }
        try Self.serialized {
            // A manual save or deletion may have won while the OS was waiting for consent.
            if try resolved(request, account: account) { return }
            if old.status == errSecItemNotFound { return }
            try check(old.status)
            guard let data = old.data, !data.isEmpty, String(data: data, encoding: .utf8) != nil else {
                throw CredentialStoreFailure(status: errSecDecode)
            }
            let added = backend.add(service: request.service, account: account, data: data)
            if added != errSecDuplicateItem { try check(added) }
            let saved = backend.read(.protected, service: request.service, account: account, interactive: false)
            try check(saved.status)
            guard let savedData = saved.data, !savedData.isEmpty, String(data: savedData, encoding: .utf8) != nil,
                  added == errSecDuplicateItem || savedData == data else { throw CredentialStoreFailure(status: errSecDecode) }
            try write(request.service + ".credential-state.v1", account: account, data: Data("active".utf8))
        }
    }
}

@_cdecl("pennel_mac_credential_store")
public func credentialStore(_ rawRequest: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    let reply: CredentialStoreReply
    if let raw = decodeString(rawRequest), let data = raw.data(using: .utf8),
       let request = try? JSONDecoder().decode(CredentialStoreRequest.self, from: data) {
        reply = MacCredentialStore().perform(request)
    } else { reply = .failure(errSecParam) }
    guard let data = try? JSONEncoder().encode(reply), let encoded = String(data: data, encoding: .utf8) else {
        return bridgeString("{\"status\":\"unavailable\",\"pending\":0}")
    }
    return bridgeString(encoded)
}
