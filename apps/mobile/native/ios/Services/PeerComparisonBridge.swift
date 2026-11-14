#if PENNEL_PEER_TRANSPORT_QA
import Foundation
import LocalAuthentication
import Security

/// Only the isolated qualification runner creates these adapters.
final class PeerComparisonNativeBridge: PeerCompareNativeTransport, @unchecked Sendable {
    let native: QANativeTransport

    init(_ native: QANativeTransport) { self.native = native }
    func identityId() -> String { native.identityID }
    func addressJson() throws -> String { try mapped { try native.addressJSON() } }
    func connect(addressJson: String, expectedPeer: String) throws -> any PeerCompareNativeChannel {
        try mapped { PeerComparisonChannelBridge(try native.connect(addressJSON: addressJson, expectedPeer: expectedPeer)) }
    }
    func accept() throws -> any PeerCompareNativeChannel {
        try mapped { PeerComparisonChannelBridge(try native.accept()) }
    }
    func approvePeer(id: String) throws -> Bool { try mapped { try native.approvePeer(id) } }
    func revokePeer(id: String) throws -> Bool { native.revokePeer(id) }
    func close() { native.close() }
}

private final class PeerComparisonChannelBridge: PeerCompareNativeChannel, @unchecked Sendable {
    let native: QANativeByteChannel
    init(_ native: QANativeByteChannel) { self.native = native }
    func peerId() -> String { native.peerID }
    func read(maxBytes: UInt32) throws -> Data { try mapped { try native.read(maxBytes: maxBytes) } }
    func write(bytes: Data) throws -> UInt32 { try mapped { try native.write(bytes) } }
    func finishWrite() throws { try mapped { try native.finishWrite() } }
    func close() { native.close() }
}

private func mapped<T>(_ operation: () throws -> T) throws -> T {
    do { return try operation() }
    catch let error as PeerCompareError { throw error }
    catch let error as QANativeTransportError {
        switch error {
        case .unauthorized: throw PeerCompareError.Unauthorized
        case .closed: throw PeerCompareError.Closed
        case .limitExceeded: throw PeerCompareError.LimitExceeded
        case .invalidConfiguration: throw PeerCompareError.InvalidConfig
        case .invalidIdentity: throw PeerCompareError.Storage
        case .identityStorage: throw PeerCompareError.Storage
        case .timeout, .transport: throw PeerCompareError.Transport
        }
    }
    catch { throw PeerCompareError.Transport }
}

/// Separate namespace: neither normal Preview nor production credentials are inspected.
final class PeerComparisonIdentityKeychain: PeerCompareIdentityStore, @unchecked Sendable {
    private let service = "app.pennel.peer-transport.qa.identities"
    private let lock = NSLock()

    private func query(_ account: String) throws -> [String: Any] {
        guard account.hasPrefix("peer-probe-"), account.utf8.count <= 100,
              account.utf8.allSatisfy({ $0 == 45 || (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) }) else {
            throw PeerCompareError.InvalidConfig
        }
        var result: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                   kSecAttrService as String: service, kSecAttrAccount as String: account]
        #if os(macOS)
        result[kSecUseDataProtectionKeychain as String] = true
        #endif
        return result
    }

    func load(account: String) throws -> String? {
        lock.lock(); defer { lock.unlock() }
        var request = try query(account)
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        let context = LAContext()
        context.interactionNotAllowed = true
        request[kSecUseAuthenticationContext as String] = context
        var item: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = item as? Data, data.count <= 32 * 1024,
              let value = String(data: data, encoding: .utf8) else { throw PeerCompareError.Storage }
        return value
    }

    func save(account: String, value: String) throws {
        lock.lock(); defer { lock.unlock() }
        guard let data = value.data(using: .utf8), !data.isEmpty, data.count <= 32 * 1024 else { throw PeerCompareError.Storage }
        let request = try query(account)
        let attributes: [String: Any] = [kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        // Core creates only after a confirmed miss. Never replace an existing or inaccessible identity.
        let status = SecItemAdd(request.merging(attributes) { $1 } as CFDictionary, nil)
        guard status == errSecSuccess else { throw PeerCompareError.Storage }
    }

    func remove(account: String) throws {
        let status = SecItemDelete(try query(account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw PeerCompareError.Storage }
    }
}

enum PeerComparisonFactory {
    static func identity(store: PeerComparisonIdentityKeychain, account: String) throws -> PeerCompareIdentity {
        if let saved = try store.load(account: account) { return try decodeIdentity(saved) }
        let identity = try generatePeerCompareIdentity()
        let encoded = try JSONSerialization.data(withJSONObject: [
            "certificate_der_b64": identity.certificateDerB64,
            "private_key_der_b64": identity.privateKeyDerB64,
            "identity_id": identity.identityId,
        ], options: [.sortedKeys])
        guard let value = String(data: encoded, encoding: .utf8) else { throw PeerCompareError.Storage }
        try store.save(account: account, value: value)
        guard try store.load(account: account) == value else { throw PeerCompareError.Storage }
        return identity
    }

    private static func decodeIdentity(_ value: String) throws -> PeerCompareIdentity {
        guard let object = try JSONSerialization.jsonObject(with: Data(value.utf8)) as? [String: String],
              let certificate = object["certificate_der_b64"], let key = object["private_key_der_b64"],
              let id = object["identity_id"] else { throw PeerCompareError.Storage }
        return PeerCompareIdentity(certificateDerB64: certificate, privateKeyDerB64: key, identityId: id)
    }

    static func native(identity: PeerCompareIdentity, account: String, bindHost: String = "127.0.0.1",
                       advertisedHost: String = "127.0.0.1", relayJSON: String? = nil) throws -> PeerComparisonProbe {
        guard let certificate = Data(base64Encoded: identity.certificateDerB64),
              let key = Data(base64Encoded: identity.privateKeyDerB64) else { throw PeerCompareError.Storage }
        let nativeIdentity = try QANativeIdentity(certificateDER: certificate, privateKeyDER: key, account: account)
        guard nativeIdentity.identityID == identity.identityId else { throw PeerCompareError.Storage }
        let relay = try relayJSON.map { try JSONDecoder().decode(QANativeRelayConfiguration.self, from: Data($0.utf8)) }
        let transport = try QANativeTransport(identity: nativeIdentity, bindHost: bindHost, advertisedHost: advertisedHost, relay: relay)
        return try PeerComparisonProbe.newNative(transport: PeerComparisonNativeBridge(transport))
    }
}
#endif
