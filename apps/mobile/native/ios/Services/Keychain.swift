import Foundation
import Security

/// Provider API keys stay on this device: `AfterFirstUnlockThisDeviceOnly` keeps them out of backups and readable
/// while a recording runs with the screen locked.
enum Keychain {
    static var service: String {
        _ = AppPaths.qaProfile
        return Bundle.main.object(forInfoDictionaryKey: "PennelKeychainService") as? String ?? "app.pennel.ios"
    }
    private static let apiKeyPrefix = "api-key."

    static func apiKeyAccount(_ providerId: String) -> String {
        apiKeyPrefix + providerId
    }

    static func apiKeys() -> ApiKeys {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecReturnAttributes as String: true,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let items = result as? [[String: Any]]
        else { return ApiKeys(byProvider: [:]) }
        var keys: [String: String] = [:]
        for item in items {
            guard let account = item[kSecAttrAccount as String] as? String, account.hasPrefix(apiKeyPrefix),
                  let data = item[kSecValueData as String] as? Data, let key = String(data: data, encoding: .utf8)
            else { continue }
            keys[String(account.dropFirst(apiKeyPrefix.count))] = key
        }
        return ApiKeys(byProvider: keys)
    }

    static func hasKey(for providerId: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: apiKeyAccount(providerId),
        ]
        return SecItemCopyMatching(query as CFDictionary, nil) == errSecSuccess
    }

    /// An empty or nil key deletes the entry.
    @discardableResult
    static func set(key: String?, for providerId: String) -> Bool {
        guard !providerId.isEmpty else { return false }
        return write(key?.trimmingCharacters(in: .whitespacesAndNewlines), account: apiKeyAccount(providerId))
    }

    static func secret(account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let data = result as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    /// A nil or empty value deletes the entry.
    @discardableResult
    static func write(_ value: String?, account: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        guard let value, !value.isEmpty else {
            let status = SecItemDelete(query as CFDictionary)
            return status == errSecSuccess || status == errSecItemNotFound
        }
        let attributes: [String: Any] = [
            kSecValueData as String: Data(value.utf8),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            status = SecItemAdd(query.merging(attributes) { $1 } as CFDictionary, nil)
        }
        return status == errSecSuccess
    }
}
