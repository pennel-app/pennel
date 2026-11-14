#if PENNEL_PEER_TRANSPORT_QA
import CryptoKit
import Foundation
import Network
import Security

/// This transport is compiled only into explicitly selected qualification products.
public enum QANativeTransportError: Error, Equatable, Sendable {
    case invalidConfiguration, invalidIdentity, unauthorized, closed, timeout, transport, limitExceeded
    case identityStorage(Int32)
}

public struct QANativeRelayConfiguration: Codable, Sendable {
    public let host: String
    public let port: UInt16
    public let serverCertificateDERBase64: String
    public let ticket: String

    enum CodingKeys: String, CodingKey {
        case host, port, ticket
        case serverCertificateDERBase64 = "server_cert_der_b64"
    }

    public init(host: String, port: UInt16, serverCertificateDERBase64: String, ticket: String) {
        self.host = host
        self.port = port
        self.serverCertificateDERBase64 = serverCertificateDERBase64
        self.ticket = ticket
    }

    fileprivate func validatedCertificate() throws -> Data {
        guard !host.isEmpty, host.utf8.count <= 253, !host.contains(where: { $0.isWhitespace }),
              !host.contains(where: { "/\\\0?#".contains($0) }), port > 0,
              !ticket.isEmpty, ticket.utf8.count <= 12_000,
              let certificate = Data(base64Encoded: serverCertificateDERBase64),
              certificate.count <= 16_384,
              SecCertificateCreateWithData(nil, certificate as CFData) != nil else {
            throw QANativeTransportError.invalidConfiguration
        }
        return certificate
    }
}

private enum QAConstants {
    static let maximumRead = 64 * 1024
    static let maximumControl = 16 * 1024
    static let maximumConnections = 16
    static let handshakeSeconds: TimeInterval = 10
    static let stepSeconds: TimeInterval = 30
    static let alpn = "pennel/peer-transport-qa/1"

    static func pin(_ certificate: Data) -> String {
        SHA256.hash(data: certificate).map { String(format: "%02x", $0) }.joined()
    }

    static func validPin(_ pin: String) -> Bool {
        pin.utf8.count == 64 && pin.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }

    static func offMainThread() throws {
        guard !Thread.isMainThread else { throw QANativeTransportError.invalidConfiguration }
    }
}

/// No user or Developer ID certificate is read or used as a transport identity.
public final class QANativeIdentity: @unchecked Sendable {
    public let identityID: String
    public let certificateDER: Data
    fileprivate let identity: SecIdentity
    private let label: String
    private let tag: Data

    public init(certificateDER: Data, privateKeyDER: Data, account: String) throws {
        guard (account.hasPrefix("peer-compare-") || account.hasPrefix("peer-probe-")), account.utf8.count <= 100,
              account.utf8.allSatisfy({ (45...46).contains($0) || (48...57).contains($0)
                  || (65...90).contains($0) || (97...122).contains($0) }),
              certificateDER.count <= 16_384, privateKeyDER.count <= 16_384,
              let certificate = SecCertificateCreateWithData(nil, certificateDER as CFData) else {
            throw QANativeTransportError.invalidIdentity
        }
        let pin = QAConstants.pin(certificateDER)
        let label = "app.pennel.peer-compare.qa.\(account).\(pin)"
        let tag = Data(label.utf8)
        let cryptoKey: P256.Signing.PrivateKey
        do { cryptoKey = try P256.Signing.PrivateKey(derRepresentation: privateKeyDER) }
        catch { throw QANativeTransportError.invalidIdentity }
        let attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeyClass as String: kSecAttrKeyClassPrivate,
            kSecAttrKeySizeInBits as String: 256,
        ]
        guard let key = SecKeyCreateWithData(cryptoKey.x963Representation as CFData, attributes as CFDictionary, nil),
              let publicKey = SecKeyCopyPublicKey(key),
              let publicBytes = SecKeyCopyExternalRepresentation(publicKey, nil) as Data?,
              let certificateKey = SecCertificateCopyKey(certificate),
              let certificateBytes = SecKeyCopyExternalRepresentation(certificateKey, nil) as Data?,
              publicBytes == certificateBytes else {
            throw QANativeTransportError.invalidIdentity
        }

        // Reusing an exact QA identity is safe; a failed lookup never replaces a key.
        let keyQuery: [String: Any] = [kSecClass as String: kSecClassKey,
                                      kSecAttrApplicationTag as String: tag,
                                      kSecUseDataProtectionKeychain as String: true,
                                      kSecReturnRef as String: true]
        var found: CFTypeRef?
        let lookup = SecItemCopyMatching(keyQuery as CFDictionary, &found)
        if lookup == errSecItemNotFound {
            let add: [String: Any] = [kSecClass as String: kSecClassKey,
                                      kSecValueRef as String: key,
                                      kSecAttrApplicationTag as String: tag,
                                      kSecAttrLabel as String: label,
                                      kSecUseDataProtectionKeychain as String: true,
                                      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
            let added = SecItemAdd(add as CFDictionary, nil)
            guard added == errSecSuccess else { throw QANativeTransportError.identityStorage(added) }
        } else if lookup != errSecSuccess {
            throw QANativeTransportError.identityStorage(lookup)
        }
        var existing: CFTypeRef?
        let readback = SecItemCopyMatching(keyQuery as CFDictionary, &existing)
        guard readback == errSecSuccess else { throw QANativeTransportError.identityStorage(readback) }
        guard
              let existing, CFGetTypeID(existing) == SecKeyGetTypeID() else {
            throw QANativeTransportError.invalidIdentity
        }
        let existingKey = unsafeDowncast(existing, to: SecKey.self)
        guard let existingPublic = SecKeyCopyPublicKey(existingKey),
              let existingBytes = SecKeyCopyExternalRepresentation(existingPublic, nil) as Data?,
              existingBytes == publicBytes else { throw QANativeTransportError.invalidIdentity }

        let certificateAdd: [String: Any] = [kSecClass as String: kSecClassCertificate,
                                           kSecValueRef as String: certificate,
                                           kSecUseDataProtectionKeychain as String: true,
                                           kSecAttrLabel as String: label]
        let certificateStatus = SecItemAdd(certificateAdd as CFDictionary, nil)
        guard certificateStatus == errSecSuccess || certificateStatus == errSecDuplicateItem else {
            throw QANativeTransportError.identityStorage(certificateStatus)
        }
        var result: CFTypeRef?
        let identityQuery: [String: Any] = [kSecClass as String: kSecClassIdentity,
                                          kSecAttrLabel as String: label,
                                          kSecUseDataProtectionKeychain as String: true,
                                          kSecReturnRef as String: true,
                                          kSecMatchLimit as String: kSecMatchLimitOne]
        let identityStatus = SecItemCopyMatching(identityQuery as CFDictionary, &result)
        guard identityStatus == errSecSuccess else { throw QANativeTransportError.identityStorage(identityStatus) }
        guard
              let result, CFGetTypeID(result) == SecIdentityGetTypeID() else {
            throw QANativeTransportError.invalidIdentity
        }
        let identity = unsafeDowncast(result, to: SecIdentity.self)
        var identityCertificate: SecCertificate?
        guard SecIdentityCopyCertificate(identity, &identityCertificate) == errSecSuccess,
              let identityCertificate,
              QAConstants.pin(SecCertificateCopyData(identityCertificate) as Data) == pin else {
            throw QANativeTransportError.invalidIdentity
        }
        self.identityID = pin
        self.certificateDER = certificateDER
        self.identity = identity
        self.label = label
        self.tag = tag
    }

    /// Called only by the owner of this freshly scoped qualification account.
    public func removeFromKeychain() throws {
        let certificateQuery: [String: Any] = [kSecClass as String: kSecClassCertificate,
                                              kSecUseDataProtectionKeychain as String: true,
                                              kSecAttrLabel as String: label]
        let keyQuery: [String: Any] = [kSecClass as String: kSecClassKey,
                                      kSecUseDataProtectionKeychain as String: true,
                                      kSecAttrApplicationTag as String: tag]
        for query in [certificateQuery, keyQuery] {
            let status = SecItemDelete(query as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else {
                throw QANativeTransportError.invalidIdentity
            }
        }
    }
}

private final class QAGate<Value: Sendable>: @unchecked Sendable {
    private let condition = NSCondition()
    private var result: Result<Value, QANativeTransportError>?

    func resolve(_ value: Result<Value, QANativeTransportError>) {
        condition.lock()
        if result == nil { result = value }
        condition.broadcast()
        condition.unlock()
    }

    func wait(seconds: TimeInterval) throws -> Value {
        guard seconds > 0 else { throw QANativeTransportError.timeout }
        let deadline = ContinuousClock.now.advanced(by: .seconds(seconds))
        condition.lock()
        defer { condition.unlock() }
        while result == nil {
            guard ContinuousClock.now < deadline else { throw QANativeTransportError.timeout }
            // NSCondition uses wall time; short waits preserve the monotonic watchdog if the
            // system clock changes while a peer stalls.
            _ = condition.wait(until: Date(timeIntervalSinceNow: 0.1))
        }
        guard let result else { throw QANativeTransportError.transport }
        return try result.get()
    }
}

private final class QAVerifiedPeer: @unchecked Sendable {
    private let lock = NSLock()
    private var value: String?
    func set(_ pin: String) { lock.withLock { value = pin } }
    func get() -> String? { lock.withLock { value } }
}

private func qaTLSOptions(identity: QANativeIdentity?, verify: @escaping @Sendable (String) -> Bool,
                          queue: DispatchQueue, verified: QAVerifiedPeer) throws -> NWProtocolTLS.Options {
    let options = NWProtocolTLS.Options()
    let security = options.securityProtocolOptions
    sec_protocol_options_set_min_tls_protocol_version(security, .TLSv13)
    sec_protocol_options_set_max_tls_protocol_version(security, .TLSv13)
    sec_protocol_options_set_tls_resumption_enabled(security, false)
    sec_protocol_options_set_tls_tickets_enabled(security, false)
    sec_protocol_options_set_tls_false_start_enabled(security, false)
    if let identity {
        guard let local = sec_identity_create(identity.identity) else { throw QANativeTransportError.invalidIdentity }
        sec_protocol_options_set_local_identity(security, local)
        sec_protocol_options_set_peer_authentication_required(security, true)
        sec_protocol_options_add_tls_application_protocol(security, QAConstants.alpn)
    } else {
        sec_protocol_options_set_tls_server_name(security, "pennel-peer-relay.qa")
    }
    sec_protocol_options_set_verify_block(security, { _, suppliedTrust, complete in
        let trust = sec_trust_copy_ref(suppliedTrust).takeRetainedValue()
        guard let certificates = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
              let leaf = certificates.first else { complete(false); return }
        let pin = QAConstants.pin(SecCertificateCopyData(leaf) as Data)
        guard verify(pin),
              SecTrustSetAnchorCertificates(trust, [leaf] as CFArray) == errSecSuccess,
              SecTrustSetAnchorCertificatesOnly(trust, true) == errSecSuccess,
              SecTrustSetPolicies(trust, SecPolicyCreateBasicX509()) == errSecSuccess,
              SecTrustEvaluateWithError(trust, nil) else { complete(false); return }
        verified.set(pin)
        complete(true)
    }, queue)
    return options
}

private func qaNegotiatedProtocol(_ metadata: sec_protocol_metadata_t) -> String? {
    if #available(macOS 15.5, iOS 18.5, *) {
        guard let value = sec_protocol_metadata_copy_negotiated_protocol(metadata) else { return nil }
        defer { free(UnsafeMutableRawPointer(mutating: value)) }
        return String(cString: value)
    }
    return nil
}

/// Blocking methods are called only on Rust's blocking pool or another owned worker.
public final class QANativeByteChannel: @unchecked Sendable {
    private let connection: NWConnection
    private let verified: QAVerifiedPeer
    private let ready = QAGate<Bool>()
    private let state = NSLock()
    private let readLock = NSLock()
    private let writeLock = NSLock()
    private var ended = false
    private var pending: [UUID: @Sendable () -> Void] = [:]
    private var closing: (@Sendable (Bool) -> Void)?
    private var readBuffer = Data()
    private var readEnded = false
    private var writeEnded = false
    private var successfulWriteShutdown = false

    fileprivate init(connection: NWConnection, verified: QAVerifiedPeer,
                     queue: DispatchQueue, closing: (@Sendable (Bool) -> Void)? = nil) {
        self.connection = connection
        self.verified = verified
        self.closing = closing
        connection.stateUpdateHandler = { [weak self] value in
            guard let self else { return }
            switch value {
            case .ready:
                guard let tls = connection.metadata(definition: NWProtocolTLS.definition) as? NWProtocolTLS.Metadata,
                      qaNegotiatedProtocol(tls.securityProtocolMetadata) == QAConstants.alpn else {
                    self.fail(.unauthorized); return
                }
                if self.verified.get() == nil,
                   let metadata = connection.metadata(definition: NWProtocolTLS.definition) as? NWProtocolTLS.Metadata {
                    _ = sec_protocol_metadata_access_peer_certificate_chain(metadata.securityProtocolMetadata) { certificate in
                        if self.verified.get() == nil {
                            let leaf = sec_certificate_copy_ref(certificate).takeRetainedValue()
                            self.verified.set(QAConstants.pin(SecCertificateCopyData(leaf) as Data))
                        }
                    }
                }
                guard self.verified.get() != nil else { self.fail(.unauthorized); return }
                self.ready.resolve(.success(true))
            case .failed: self.fail(.transport)
            case .cancelled: self.fail(.closed)
            default: break
            }
        }
        connection.start(queue: queue)
    }

    public var peerID: String { verified.get() ?? "" }

    fileprivate func waitReady() throws {
        do { _ = try ready.wait(seconds: QAConstants.handshakeSeconds) }
        catch { close(); throw error }
    }

    public func read(maxBytes: UInt32) throws -> Data {
        try read(maxBytes: maxBytes, timeoutSeconds: QAConstants.stepSeconds)
    }

    private func read(maxBytes: UInt32, timeoutSeconds: TimeInterval) throws -> Data {
        try QAConstants.offMainThread()
        guard maxBytes > 0, maxBytes <= QAConstants.maximumRead else { throw QANativeTransportError.limitExceeded }
        readLock.lock()
        defer { readLock.unlock() }
        guard !state.withLock({ ended }) else { throw QANativeTransportError.closed }
        if readEnded { return Data() }
        do {
            if readBuffer.isEmpty {
                let deadline = ContinuousClock.now.advanced(by: .seconds(timeoutSeconds))
                let header = try receiveExactly(4, deadline: deadline)
                let size = header.reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
                guard size <= QAConstants.maximumRead else { throw QANativeTransportError.limitExceeded }
                if size == 0 { readEnded = true; return Data() }
                readBuffer = try receiveExactly(Int(size), deadline: deadline)
            }
            let amount = min(Int(maxBytes), readBuffer.count)
            let result = Data(readBuffer.prefix(amount))
            readBuffer.removeFirst(amount)
            return result
        } catch { close(); throw error }
    }

    private func receiveExactly(_ count: Int, deadline: ContinuousClock.Instant) throws -> Data {
        var result = Data()
        result.reserveCapacity(count)
        while result.count < count {
            let remaining = ContinuousClock.now.duration(to: deadline)
            guard remaining > .zero else { throw QANativeTransportError.timeout }
            let seconds = Double(remaining.components.seconds) + Double(remaining.components.attoseconds) / 1e18
            let data = try receiveOnce(maximum: count - result.count, timeout: seconds)
            guard !data.isEmpty else { throw QANativeTransportError.transport }
            result.append(data)
        }
        return result
    }

    private func receiveOnce(maximum: Int, timeout: TimeInterval) throws -> Data {
        let gate = QAGate<Data>()
        let token = try register { gate.resolve(.failure(.closed)) }
        defer { unregister(token) }
        connection.receive(minimumIncompleteLength: 1, maximumLength: maximum) { data, _, complete, error in
            if error != nil { gate.resolve(.failure(.transport)) }
            else if let data, !data.isEmpty { gate.resolve(.success(data)) }
            else if complete { gate.resolve(.success(Data())) }
            else { gate.resolve(.failure(.transport)) }
        }
        return try gate.wait(seconds: timeout)
    }

    @discardableResult
    public func write(_ data: Data) throws -> UInt32 {
        try write(data, timeoutSeconds: QAConstants.stepSeconds)
    }

    private func write(_ data: Data, timeoutSeconds: TimeInterval) throws -> UInt32 {
        try QAConstants.offMainThread()
        guard !data.isEmpty, data.count <= QAConstants.maximumRead else { throw QANativeTransportError.limitExceeded }
        writeLock.lock()
        defer { writeLock.unlock() }
        guard !writeEnded else { throw QANativeTransportError.closed }
        var framed = Data()
        var count = UInt32(data.count).bigEndian
        withUnsafeBytes(of: &count) { framed.append(contentsOf: $0) }
        framed.append(data)
        do { try sendFrame(framed, timeoutSeconds: timeoutSeconds); return UInt32(data.count) }
        catch { close(); throw error }
    }

    private func sendFrame(_ data: Data, timeoutSeconds: TimeInterval = QAConstants.stepSeconds) throws {
        let gate = QAGate<Bool>()
        let token = try register { gate.resolve(.failure(.closed)) }
        defer { unregister(token) }
        connection.send(content: data, completion: .contentProcessed { error in
            gate.resolve(error == nil ? .success(true) : .failure(.transport))
        })
        _ = try gate.wait(seconds: timeoutSeconds)
    }

    public func finishWrite() throws {
        try QAConstants.offMainThread()
        writeLock.lock()
        defer { writeLock.unlock() }
        if writeEnded { return }
        // Network.framework does not expose a reliable TLS write-half shutdown while keeping
        // reverse acknowledgement reads alive. Four-byte native transport frames provide that
        // byte-channel contract; zero length ends only this direction. Payloads stay <=64 KiB.
        do {
            try sendFrame(Data(repeating: 0, count: 4))
            writeEnded = true
            state.withLock { successfulWriteShutdown = true }
        }
        catch { close(); throw error }
    }

    public func close() {
        fail(.closed, graceful: state.withLock { successfulWriteShutdown })
    }

    fileprivate func abort() { fail(.closed) }

    fileprivate func confirmPeerAdmission() throws {
        // Client TLS readiness can precede receipt of a server's client-certificate alert.
        // Exchange this bounded marker only after the local pin/approval gate, so neither
        // factory returns a usable channel until the remote gate also completed.
        let marker = Data("PENNEL-NATIVE-PEER-READY/1\0".utf8)
        let deadline = ContinuousClock.now.advanced(by: .seconds(QAConstants.handshakeSeconds))
        _ = try write(marker, timeoutSeconds: QAConstants.handshakeSeconds)
        let remaining = ContinuousClock.now.duration(to: deadline)
        guard remaining > .zero else { close(); throw QANativeTransportError.timeout }
        let seconds = Double(remaining.components.seconds) + Double(remaining.components.attoseconds) / 1e18
        guard try read(maxBytes: UInt32(QAConstants.maximumRead), timeoutSeconds: seconds) == marker else {
            close()
            throw QANativeTransportError.unauthorized
        }
    }

    private func register(_ callback: @escaping @Sendable () -> Void) throws -> UUID {
        try state.withLock {
            guard !ended else { throw QANativeTransportError.closed }
            let token = UUID()
            pending[token] = callback
            return token
        }
    }

    private func unregister(_ token: UUID) { _ = state.withLock { pending.removeValue(forKey: token) } }

    private func fail(_ error: QANativeTransportError, graceful: Bool = false) {
        let callbacks: ([@Sendable () -> Void], (@Sendable (Bool) -> Void)?)? = state.withLock {
            guard !ended else { return nil }
            ended = true
            let callbacks = Array(pending.values)
            pending.removeAll()
            let closing = closing
            self.closing = nil
            return (callbacks, closing)
        }
        guard let callbacks else { return }
        ready.resolve(.failure(error))
        if graceful { connection.cancel() }
        else { connection.forceCancel() }
        callbacks.0.forEach { $0() }
        callbacks.1?(graceful)
    }

    deinit { connection.cancel() }
}

private struct QANativeAddress: Codable {
    let identityID: String
    let host: String
    let port: UInt16
    let route: String

    enum CodingKeys: String, CodingKey {
        case identityID = "identity_id"
        case host, port, route
    }
}

private struct QARelayReadyReply: Decodable {
    let ready: Bool
    let error: String?
}

private enum QARelaySetupError: Error { case busy }

private struct QARelayOutgoing {
    let peer: String
    let result: QAGate<QANativeByteChannel>
}

public final class QANativeTransport: @unchecked Sendable {
    public let identityID: String
    private let identity: QANativeIdentity
    private let listener: NWListener
    private let queue = DispatchQueue(label: "app.pennel.peer-compare.native", qos: .utility)
    private let condition = NSCondition()
    private let advertisedHost: String
    private let relay: QANativeRelayConfiguration?
    private var allowed = Set<String>()
    private var channels: [UUID: QANativeByteChannel] = [:]
    private var incoming: [QANativeByteChannel] = []
    private var rawConnections: [ObjectIdentifier: NWConnection] = [:]
    private var pumps: [UUID: QARelayPump] = [:]
    private var auxiliaryListeners: [UUID: NWListener] = [:]
    private var relayPreparing = false
    private var relayGeneration: UInt64 = 0
    private var relayOuter: NWConnection?
    private var relayOutgoing: QARelayOutgoing?
    private var relaySignalSent = false
    private var relayIncoming = false
    private var closed = false

    public init(identity: QANativeIdentity, bindHost: String = "127.0.0.1",
                advertisedHost: String = "127.0.0.1", relay: QANativeRelayConfiguration? = nil) throws {
        try QAConstants.offMainThread()
        guard #available(macOS 15.5, iOS 18.5, *) else { throw QANativeTransportError.invalidConfiguration }
        guard IPv4Address(advertisedHost) != nil || IPv6Address(advertisedHost) != nil else {
            throw QANativeTransportError.invalidConfiguration
        }
        if let relay { _ = try relay.validatedCertificate() }
        self.identity = identity
        self.identityID = identity.identityID
        self.advertisedHost = advertisedHost
        self.relay = relay
        let verification = QAVerifiedPeer()
        // The verification callback consults a shared access box initialized before bind.
        let access = QAAccessBox()
        let tls = try qaTLSOptions(identity: identity, verify: { access.contains($0) }, queue: queue, verified: verification)
        let parameters = NWParameters(tls: tls, tcp: NWProtocolTCP.Options())
        parameters.requiredLocalEndpoint = .hostPort(host: NWEndpoint.Host(relay == nil ? bindHost : "127.0.0.1"), port: .any)
        self.listener = try NWListener(using: parameters, on: .any)
        self.listener.newConnectionLimit = QAConstants.maximumConnections
        self.access = access
        let started = QAGate<Bool>()
        listener.stateUpdateHandler = { state in
            switch state {
            case .ready: started.resolve(.success(true))
            case .failed: started.resolve(.failure(.transport))
            case .cancelled: started.resolve(.failure(.closed))
            default: break
            }
        }
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { connection.cancel(); return }
            self.received(connection)
        }
        listener.start(queue: queue)
        do { _ = try started.wait(seconds: QAConstants.handshakeSeconds) }
        catch { listener.cancel(); throw error }
    }

    private let access: QAAccessBox

    public func addressJSON() throws -> String {
        condition.lock()
        let isClosed = closed
        condition.unlock()
        guard !isClosed, let port = listener.port?.rawValue else { throw QANativeTransportError.closed }
        let address = QANativeAddress(identityID: identityID,
                                      host: relay == nil ? advertisedHost : "127.0.0.1",
                                      port: relay == nil ? port : 2,
                                      route: relay == nil ? "direct" : "forced_relay")
        guard let json = String(data: try JSONEncoder().encode(address), encoding: .utf8) else {
            throw QANativeTransportError.invalidConfiguration
        }
        return json
    }

    public func approvePeer(_ id: String) throws -> Bool {
        guard QAConstants.validPin(id), id != identityID else { throw QANativeTransportError.invalidConfiguration }
        condition.lock()
        defer { condition.unlock() }
        guard !closed else { throw QANativeTransportError.closed }
        guard allowed.contains(id) || allowed.count < QAConstants.maximumConnections else {
            throw QANativeTransportError.limitExceeded
        }
        let inserted = allowed.insert(id).inserted
        access.set(allowed)
        return inserted
    }

    public func revokePeer(_ id: String) -> Bool {
        condition.lock()
        let removed = allowed.remove(id) != nil
        access.set(allowed)
        let affected = channels.values.filter { $0.peerID == id }
        incoming.removeAll { $0.peerID == id }
        let relayPumps = Array(pumps.values)
        condition.broadcast()
        condition.unlock()
        affected.forEach { $0.abort() }
        if removed {
            relayPumps.forEach { $0.close() }
            if relay != nil { resetRelay() }
        }
        return removed
    }

    public func connect(addressJSON: String, expectedPeer: String) throws -> QANativeByteChannel {
        try QAConstants.offMainThread()
        guard addressJSON.utf8.count <= QAConstants.maximumControl, access.contains(expectedPeer),
              let bytes = addressJSON.data(using: .utf8),
              let fields = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any],
              Set(fields.keys) == ["identity_id", "host", "port", "route"],
              let address = try? JSONDecoder().decode(QANativeAddress.self, from: bytes),
              address.identityID == expectedPeer,
              IPv4Address(address.host) != nil || IPv6Address(address.host) != nil,
              address.port > 0 else {
            throw QANativeTransportError.unauthorized
        }
        if relay != nil {
            guard address.route == "forced_relay" else { throw QANativeTransportError.invalidConfiguration }
            return try connectRelayed(expectedPeer: expectedPeer)
        }
        guard address.route == "direct", let endpointPort = NWEndpoint.Port(rawValue: address.port) else {
            throw QANativeTransportError.invalidConfiguration
        }
        let verified = QAVerifiedPeer()
        let tls = try qaTLSOptions(identity: identity, verify: { [access] pin in
            pin == expectedPeer && access.contains(pin)
        }, queue: queue, verified: verified)
        let connection = NWConnection(host: NWEndpoint.Host(address.host), port: endpointPort,
                                      using: NWParameters(tls: tls, tcp: NWProtocolTCP.Options()))
        return try makeChannel(connection, verified: verified)
    }

    public func accept() throws -> QANativeByteChannel {
        try QAConstants.offMainThread()
        let deadline = Date(timeIntervalSinceNow: QAConstants.stepSeconds)
        while true {
            if relay != nil { prepareResponderRelay() }
            condition.lock()
            if closed { condition.unlock(); throw QANativeTransportError.closed }
            if !incoming.isEmpty {
                let channel = incoming.removeFirst()
                condition.unlock()
                if access.contains(channel.peerID) { return channel }
                channel.close()
                continue
            }
            guard Date() < deadline else { condition.unlock(); throw QANativeTransportError.timeout }
            _ = condition.wait(until: min(deadline, Date(timeIntervalSinceNow: 0.1)))
            condition.unlock()
        }
    }

    public func close() {
        condition.lock()
        if closed { condition.unlock(); return }
        closed = true
        relayGeneration &+= 1
        let outgoing = relayOutgoing
        relayOutgoing = nil
        access.set([])
        let current = Array(channels.values)
        let raw = Array(rawConnections.values)
        let currentPumps = Array(pumps.values)
        let listeners = Array(auxiliaryListeners.values)
        incoming.removeAll()
        condition.broadcast()
        condition.unlock()
        outgoing?.result.resolve(.failure(.closed))
        listener.cancel()
        current.forEach { $0.abort() }
        raw.forEach { $0.forceCancel() }
        currentPumps.forEach { $0.close() }
        listeners.forEach { $0.cancel() }
    }

    private func received(_ connection: NWConnection) {
        // Each accepted connection has its own verified-peer box. The listener's verification
        // callback authorizes the leaf; this box is filled from negotiated metadata after ready.
        let verified = QAVerifiedPeer()
        let token = UUID()
        condition.lock()
        guard !closed, channels.count < QAConstants.maximumConnections else {
            let replenish = !closed
            condition.unlock()
            connection.forceCancel()
            // NWListener decremented this counter before delivering the rejected connection.
            // No channel owns the slot, so restore it here while the listener is active.
            if replenish { listener.newConnectionLimit += 1 }
            return
        }
        let channel = QANativeByteChannel(connection: connection, verified: verified, queue: queue,
                                           closing: { [weak self] _ in self?.removeChannel(token, replenishListener: true) })
        channels[token] = channel
        condition.unlock()
        // The shared TLS verify callback records a pin in the accepted connection's TLS metadata;
        // obtain the chain independently for this connection before admitting the channel.
        DispatchQueue.global(qos: .utility).async { [weak self, weak channel] in
            guard let self, let channel else { return }
            do {
                try channel.waitReady()
                guard self.access.contains(channel.peerID) else { channel.close(); return }
                try channel.confirmPeerAdmission()
                self.condition.lock()
                if self.closed || !self.allowed.contains(channel.peerID) {
                    self.condition.unlock(); channel.close(); return
                }
                self.incoming.append(channel)
                self.condition.broadcast()
                self.condition.unlock()
            } catch { channel.close() }
        }
    }

    private func makeChannel(_ connection: NWConnection, verified: QAVerifiedPeer,
                             extraClose: (@Sendable (Bool) -> Void)? = nil) throws -> QANativeByteChannel {
        let token = UUID()
        condition.lock()
        guard !closed, channels.count < QAConstants.maximumConnections else {
            condition.unlock(); connection.cancel(); throw QANativeTransportError.closed
        }
        let channel = QANativeByteChannel(connection: connection, verified: verified, queue: queue,
                                           closing: { [weak self] graceful in
            extraClose?(graceful)
            self?.removeChannel(token, replenishListener: false)
        })
        channels[token] = channel
        condition.unlock()
        try channel.waitReady()
        guard access.contains(channel.peerID) else { channel.close(); throw QANativeTransportError.unauthorized }
        try channel.confirmPeerAdmission()
        guard access.contains(channel.peerID) else { channel.close(); throw QANativeTransportError.unauthorized }
        return channel
    }

    private func removeChannel(_ token: UUID, replenishListener: Bool) {
        condition.lock()
        if let removed = channels.removeValue(forKey: token) {
            incoming.removeAll { $0 === removed }
        }
        condition.broadcast()
        condition.unlock()
        guard replenishListener else { return }
        queue.async { [weak self] in
            guard let self else { return }
            self.condition.lock()
            let active = !self.closed
            self.condition.unlock()
            if active { self.listener.newConnectionLimit += 1 }
        }
    }

    private func rawReady(_ connection: NWConnection) throws {
        let token = ObjectIdentifier(connection)
        condition.lock()
        guard !closed, rawConnections.count < QAConstants.maximumConnections else {
            condition.unlock(); connection.cancel(); throw QANativeTransportError.closed
        }
        rawConnections[token] = connection
        condition.unlock()
        let gate = QAGate<Bool>()
        connection.stateUpdateHandler = { state in
            switch state {
            case .ready: gate.resolve(.success(true))
            case .failed: gate.resolve(.failure(.transport))
            case .cancelled: gate.resolve(.failure(.closed))
            default: break
            }
        }
        connection.start(queue: queue)
        do { _ = try gate.wait(seconds: QAConstants.handshakeSeconds) }
        catch { removeRaw(connection); throw error }
    }

    private func relayConnection() throws -> NWConnection {
        let deadline = ContinuousClock.now.advanced(by: .seconds(QAConstants.handshakeSeconds))
        while true {
            do { return try relayConnectionOnce() }
            catch QARelaySetupError.busy {
                // A prior successful pair may still be negotiating its graceful FIN. Retry
                // only this same ticket/relay admission for a bounded drain interval.
                guard ContinuousClock.now < deadline else { throw QANativeTransportError.timeout }
                condition.lock()
                guard !closed else { condition.unlock(); throw QANativeTransportError.closed }
                _ = condition.wait(until: Date(timeIntervalSinceNow: 0.05))
                condition.unlock()
            }
        }
    }

    private func relayConnectionOnce() throws -> NWConnection {
        guard let relay else { throw QANativeTransportError.invalidConfiguration }
        let certificate = try relay.validatedCertificate()
        let pin = QAConstants.pin(certificate)
        let verified = QAVerifiedPeer()
        let tls = try qaTLSOptions(identity: nil, verify: { $0 == pin }, queue: queue, verified: verified)
        guard let port = NWEndpoint.Port(rawValue: relay.port) else { throw QANativeTransportError.invalidConfiguration }
        let connection = NWConnection(host: NWEndpoint.Host(relay.host), port: port,
                                      using: NWParameters(tls: tls, tcp: NWProtocolTCP.Options()))
        try rawReady(connection)
        do {
            var control = try JSONEncoder().encode(["ticket": relay.ticket])
            control.append(0x0a)
            guard control.count <= QAConstants.maximumControl else { throw QANativeTransportError.limitExceeded }
            try rawWrite(connection, control)
            var reply = Data()
            let deadline = ContinuousClock.now.advanced(by: .seconds(QAConstants.stepSeconds))
            while !reply.contains(0x0a) {
                guard reply.count < QAConstants.maximumControl else { throw QANativeTransportError.limitExceeded }
                let remaining = ContinuousClock.now.duration(to: deadline)
                guard remaining > .zero else { throw QANativeTransportError.timeout }
                let seconds = Double(remaining.components.seconds) + Double(remaining.components.attoseconds) / 1e18
                let chunk = try rawRead(connection, maximum: 1, timeoutSeconds: seconds)
                guard !chunk.isEmpty else { throw QANativeTransportError.transport }
                reply.append(chunk)
            }
            let value = try JSONDecoder().decode(QARelayReadyReply.self, from: reply)
            if !value.ready, value.error == "busy" { throw QARelaySetupError.busy }
            guard value.ready, value.error == nil else { throw QANativeTransportError.transport }
            return connection
        } catch { removeRaw(connection); throw error }
    }

    private func rawWrite(_ connection: NWConnection, _ data: Data) throws {
        let gate = QAGate<Bool>()
        connection.send(content: data, completion: .contentProcessed { error in
            gate.resolve(error == nil ? .success(true) : .failure(.transport))
        })
        _ = try gate.wait(seconds: QAConstants.stepSeconds)
    }

    private func rawRead(_ connection: NWConnection, maximum: Int,
                         timeoutSeconds: TimeInterval = QAConstants.stepSeconds) throws -> Data {
        let gate = QAGate<Data>()
        connection.receive(minimumIncompleteLength: 1, maximumLength: maximum) { data, _, complete, error in
            if error != nil { gate.resolve(.failure(.transport)) }
            else if let data { gate.resolve(.success(data)) }
            else if complete { gate.resolve(.success(Data())) }
            else { gate.resolve(.failure(.transport)) }
        }
        return try gate.wait(seconds: timeoutSeconds)
    }

    private func removeRaw(_ connection: NWConnection, force: Bool = true) {
        if force { connection.forceCancel() }
        else { connection.cancel() }
        condition.lock()
        rawConnections.removeValue(forKey: ObjectIdentifier(connection))
        condition.unlock()
    }

    private func prepareResponderRelay() {
        condition.lock()
        guard !closed, !relayPreparing, allowed.count == 1 else { condition.unlock(); return }
        relayPreparing = true
        relayGeneration &+= 1
        let generation = relayGeneration
        condition.unlock()
        DispatchQueue.global(qos: .utility).async { [weak self] in
            guard let self else { return }
            do {
                let outer = try self.relayConnection()
                self.condition.lock()
                guard !self.closed, self.relayGeneration == generation else {
                    self.condition.unlock(); self.removeRaw(outer); throw QANativeTransportError.closed
                }
                self.relayOuter = outer
                let open = self.relayOutgoing != nil
                if open { self.relaySignalSent = true }
                self.condition.unlock()
                if open { try self.rawWrite(outer, Data([1])) }

                // Passive offer loops may exist on both devices. Do not choose a TLS server
                // role until a one-shot OPEN arrives; ACK commits the opposite client role.
                // This is one channel per pair, not a stream multiplexer. Simultaneous OPENs
                // fail as an explicit collision and retain all resumable file state.
                let control = try self.rawRead(outer, maximum: 1)
                guard control.count == 1 else { throw QANativeTransportError.transport }
                if control[0] == 1 {
                    self.condition.lock()
                    let accepted = !self.closed && self.relayGeneration == generation
                        && !self.relaySignalSent && self.relayOutgoing == nil
                    if accepted { self.relayIncoming = true }
                    self.condition.unlock()
                    guard accepted else { throw QANativeTransportError.invalidConfiguration }
                    try self.rawWrite(outer, Data([2]))
                    guard let port = self.listener.port else { throw QANativeTransportError.transport }
                    let local = NWConnection(host: "127.0.0.1", port: port, using: .tcp)
                    try self.rawReady(local)
                    self.installPump(local, outer, generation: generation)
                } else if control[0] == 2 {
                    self.condition.lock()
                    let outgoing = self.relayOutgoing
                    let accepted = !self.closed && self.relayGeneration == generation && self.relaySignalSent
                    self.condition.unlock()
                    guard accepted, let outgoing else { throw QANativeTransportError.invalidConfiguration }
                    let channel = try self.openRelayClient(outer: outer, expectedPeer: outgoing.peer, generation: generation)
                    self.condition.lock()
                    let current = !self.closed && self.relayGeneration == generation
                    if current { self.relayOutgoing = nil }
                    self.condition.unlock()
                    guard current else { channel.close(); throw QANativeTransportError.closed }
                    outgoing.result.resolve(.success(channel))
                } else {
                    throw QANativeTransportError.invalidConfiguration
                }
            } catch {
                self.resetRelay(generation: generation,
                                error: error as? QANativeTransportError ?? .transport)
            }
        }
    }

    private func connectRelayed(expectedPeer: String) throws -> QANativeByteChannel {
        let result = QAGate<QANativeByteChannel>()
        let deadline = ContinuousClock.now.advanced(by: .seconds(QAConstants.handshakeSeconds))
        condition.lock()
        // A completed request may return its durable receipt just before the serving offer
        // loop retires the previous channel. Serialize that short drain without cancelling it.
        while !closed, relayOutgoing == nil, relaySignalSent || relayIncoming {
            guard ContinuousClock.now < deadline else {
                condition.unlock(); throw QANativeTransportError.invalidConfiguration
            }
            _ = condition.wait(until: Date(timeIntervalSinceNow: 0.01))
        }
        guard !closed, allowed == [expectedPeer], relayOutgoing == nil,
              !relaySignalSent, !relayIncoming else {
            condition.unlock(); throw QANativeTransportError.invalidConfiguration
        }
        relayOutgoing = QARelayOutgoing(peer: expectedPeer, result: result)
        let outer = relayOuter
        if outer != nil { relaySignalSent = true }
        let generation = relayGeneration
        condition.unlock()
        do {
            if let outer { try rawWrite(outer, Data([1])) }
            else { prepareResponderRelay() }
            return try result.wait(seconds: QAConstants.stepSeconds)
        } catch {
            // A timeout owns only its current attempt; a retired callback cannot close a
            // replacement pair prepared by a still-running offer loop.
            condition.lock()
            let current = relayOutgoing?.result === result ? relayGeneration : nil
            condition.unlock()
            if let current { resetRelay(generation: current) }
            else if outer != nil { resetRelay(generation: generation) }
            throw error
        }
    }

    private func openRelayClient(outer: NWConnection, expectedPeer: String,
                                 generation: UInt64) throws -> QANativeByteChannel {
        let token = UUID()
        var forwarding: NWListener?
        do {
            let parameters = NWParameters.tcp
            parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
            let localListener = try NWListener(using: parameters, on: .any)
            forwarding = localListener
            localListener.newConnectionLimit = 1
            let listening = QAGate<Bool>()
            localListener.stateUpdateHandler = { state in
                switch state {
                case .ready: listening.resolve(.success(true))
                case .failed: listening.resolve(.failure(.transport))
                case .cancelled: listening.resolve(.failure(.closed))
                default: break
                }
            }
            localListener.newConnectionHandler = { [weak self] connection in
                guard let self else { connection.cancel(); return }
                localListener.cancel()
                self.condition.lock()
                self.auxiliaryListeners.removeValue(forKey: token)
                self.condition.unlock()
                DispatchQueue.global(qos: .utility).async {
                    do {
                        try self.rawReady(connection)
                        self.installPump(connection, outer, generation: generation)
                    } catch {
                        self.removeRaw(connection)
                        self.resetRelay(generation: generation)
                    }
                }
            }
            condition.lock()
            guard !closed, relayGeneration == generation else {
                condition.unlock(); throw QANativeTransportError.closed
            }
            auxiliaryListeners[token] = localListener
            condition.unlock()
            localListener.start(queue: queue)
            _ = try listening.wait(seconds: QAConstants.handshakeSeconds)
            guard let port = localListener.port else { throw QANativeTransportError.transport }
            let verified = QAVerifiedPeer()
            let tls = try qaTLSOptions(identity: identity, verify: { [access] in
                $0 == expectedPeer && access.contains($0)
            }, queue: queue, verified: verified)
            let inner = NWConnection(host: "127.0.0.1", port: port,
                                     using: NWParameters(tls: tls, tcp: NWProtocolTCP.Options()))
            return try makeChannel(inner, verified: verified, extraClose: { [weak self] graceful in
                // A successful stream close must let the loopback pump forward every queued
                // ciphertext byte before retiring the outer session. Errors abort immediately.
                if !graceful { self?.resetRelay(generation: generation) }
            })
        } catch {
            forwarding?.cancel()
            condition.lock()
            auxiliaryListeners.removeValue(forKey: token)
            condition.unlock()
            throw error
        }
    }

    private func installPump(_ first: NWConnection, _ second: NWConnection, generation: UInt64) {
        let token = UUID()
        let pump = QARelayPump(first: first, second: second, queue: queue, closed: { [weak self] graceful in
            guard let self else { return }
            self.removeRaw(first, force: !graceful)
            self.removeRaw(second, force: !graceful)
            self.condition.lock()
            self.pumps.removeValue(forKey: token)
            self.condition.unlock()
            self.resetRelay(generation: generation, graceful: graceful)
        })
        condition.lock()
        if closed || relayGeneration != generation { condition.unlock(); pump.close(); return }
        pumps[token] = pump
        condition.unlock()
        pump.start()
    }

    private func resetRelay(generation: UInt64? = nil, error: QANativeTransportError = .closed,
                            graceful: Bool = false) {
        condition.lock()
        guard generation == nil || relayGeneration == generation else { condition.unlock(); return }
        relayGeneration &+= 1
        let currentPumps = Array(pumps.values)
        let raw = Array(rawConnections.values)
        let listeners = Array(auxiliaryListeners.values)
        let outgoing = relayOutgoing
        relayOutgoing = nil
        relayOuter = nil
        relaySignalSent = false
        relayIncoming = false
        relayPreparing = false
        condition.broadcast()
        condition.unlock()
        outgoing?.result.resolve(.failure(error))
        currentPumps.forEach { $0.close(graceful: graceful) }
        raw.forEach { removeRaw($0, force: !graceful) }
        listeners.forEach { $0.cancel() }
    }

    deinit { close() }
}

private final class QAAccessBox: @unchecked Sendable {
    private let lock = NSLock()
    private var allowed = Set<String>()
    func contains(_ pin: String) -> Bool { lock.withLock { allowed.contains(pin) } }
    func set(_ pins: Set<String>) { lock.withLock { allowed = pins } }
}

/// Exactly one bounded receive and send is outstanding in each direction.
private final class QARelayPump: @unchecked Sendable {
    private let first: NWConnection
    private let second: NWConnection
    private let queue: DispatchQueue
    private let lock = NSLock()
    private var ended = false
    private var closedCallback: (@Sendable (Bool) -> Void)?
    private var timer: DispatchSourceTimer?
    private var operationBegan: [Int: Date] = [:]

    init(first: NWConnection, second: NWConnection, queue: DispatchQueue,
         closed: @escaping @Sendable (Bool) -> Void) {
        self.first = first
        self.second = second
        self.queue = queue
        self.closedCallback = closed
    }

    func start() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        self.timer = timer
        timer.schedule(deadline: .now() + 1, repeating: 1)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            let expired = self.lock.withLock {
                self.operationBegan.values.contains { Date().timeIntervalSince($0) >= QAConstants.stepSeconds }
            }
            if expired { self.close() }
        }
        timer.resume()
        transfer(first, second, direction: 0)
        transfer(second, first, direction: 1)
    }

    private func transfer(_ source: NWConnection, _ target: NWConnection, direction: Int) {
        guard !lock.withLock({ ended }) else { return }
        lock.withLock { operationBegan[direction] = Date() }
        source.receive(minimumIncompleteLength: 1, maximumLength: QAConstants.maximumRead) { [weak self] data, _, complete, error in
            guard let self, !self.lock.withLock({ self.ended }) else { return }
            if error != nil { self.close(); return }
            if let data, !data.isEmpty {
                self.lock.withLock { self.operationBegan[direction] = Date() }
                target.send(content: data, completion: .contentProcessed { [weak self] error in
                    guard let self else { return }
                    if error != nil { self.close() }
                    else if complete { self.close(graceful: true) }
                    else { self.transfer(source, target, direction: direction) }
                })
            } else if complete { self.close(graceful: true) }
            else { self.close() }
        }
    }

    func close(graceful: Bool = false) {
        let callback: (@Sendable (Bool) -> Void)? = lock.withLock {
            guard !ended else { return nil }
            ended = true
            let callback = closedCallback
            closedCallback = nil
            return callback
        }
        guard let callback else { return }
        timer?.cancel()
        if graceful { first.cancel(); second.cancel() }
        else { first.forceCancel(); second.forceCancel() }
        callback(graceful)
    }
}
#endif
