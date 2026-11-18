import CryptoKit
import Foundation
import Network
import Testing
@testable import PeerNativeQA

@Suite(.serialized)
struct NativeTransportTests {
    private struct Material: Sendable {
        let certificate: Data
        let key: Data
        let account: String

        static func create() throws -> Self {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-peer-native-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                                   attributes: [.posixPermissions: 0o700])
            defer { try? FileManager.default.removeItem(at: root) }
            func run(_ arguments: [String]) throws {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/bin/openssl")
                process.arguments = arguments
                process.standardOutput = FileHandle.nullDevice
                process.standardError = FileHandle.nullDevice
                try process.run()
                process.waitUntilExit()
                guard process.terminationStatus == 0 else { throw QANativeTransportError.invalidIdentity }
            }
            let key = root.appendingPathComponent("qa.key")
            let certificate = root.appendingPathComponent("qa.der")
            let pkcs8 = root.appendingPathComponent("qa-key.der")
            try run(["ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", key.path])
            try run(["req", "-new", "-x509", "-key", key.path, "-sha256", "-days", "2",
                     "-subj", "/CN=pennel-peer-native.qa", "-outform", "DER", "-out", certificate.path])
            try run(["pkcs8", "-topk8", "-nocrypt", "-in", key.path, "-outform", "DER", "-out", pkcs8.path])
            return Self(certificate: try Data(contentsOf: certificate), key: try Data(contentsOf: pkcs8),
                        account: "peer-compare-\(UUID().uuidString)")
        }

        func identity() throws -> QANativeIdentity {
            try QANativeIdentity(certificateDER: certificate, privateKeyDER: key, account: account)
        }
    }

    @Test
    func identityRoundTripAndMismatchedKeyRefusal() async throws {
        try await Task.detached {
            let first = try Material.create()
            let second = try Material.create()
            let identity = try first.identity()
            defer { try? identity.removeFromKeychain() }
            let reopened = try first.identity()
            #expect(reopened.identityID == identity.identityID)
            #expect(identity.identityID == SHA256.hash(data: first.certificate).map { String(format: "%02x", $0) }.joined())
            #expect(throws: QANativeTransportError.invalidIdentity) {
                try QANativeIdentity(certificateDER: first.certificate, privateKeyDER: second.key,
                                     account: "peer-compare-\(UUID().uuidString)")
            }
            #expect(throws: QANativeTransportError.invalidIdentity) {
                try QANativeIdentity(certificateDER: first.certificate, privateKeyDER: first.key, account: "production")
            }
        }.value
    }

    @Test
    func directMutualTLSAndHalfCloseKeepFinalAcknowledgement() async throws {
        try await Task.detached {
            let first = try Material.create().identity()
            let second = try Material.create().identity()
            defer { try? first.removeFromKeychain(); try? second.removeFromKeychain() }
            let a = try QANativeTransport(identity: first)
            let b = try QANativeTransport(identity: second)
            defer { a.close(); b.close() }
            #expect(try a.approvePeer(b.identityID))
            #expect(try b.approvePeer(a.identityID))
            let connected = try a.connect(addressJSON: b.addressJSON(), expectedPeer: b.identityID)
            defer { connected.close() }
            let accepted = try b.accept()
            defer { accepted.close() }
            #expect(connected.peerID == b.identityID)
            #expect(accepted.peerID == a.identityID)
            #expect(try connected.write(Data("hello".utf8)) == 5)
            #expect(try accepted.read(maxBytes: 64) == Data("hello".utf8))
            try connected.finishWrite()
            #expect(try accepted.read(maxBytes: 64).isEmpty)
            #expect(try accepted.write(Data("ack".utf8)) == 3)
            #expect(try connected.read(maxBytes: 64) == Data("ack".utf8))
            try accepted.finishWrite()
            #expect(try connected.read(maxBytes: 64).isEmpty)
            #expect(throws: QANativeTransportError.limitExceeded) { try connected.read(maxBytes: 65_537) }
            #expect(throws: QANativeTransportError.limitExceeded) { try connected.write(Data(repeating: 1, count: 65_537)) }
        }.value
    }

    @Test
    func unknownAndSubstitutedPeersCannotCompleteHandshake() async throws {
        try await Task.detached {
            let first = try Material.create().identity()
            let second = try Material.create().identity()
            defer { try? first.removeFromKeychain(); try? second.removeFromKeychain() }
            let a = try QANativeTransport(identity: first)
            let b = try QANativeTransport(identity: second)
            defer { a.close(); b.close() }
            _ = try a.approvePeer(b.identityID)
            do {
                let channel = try a.connect(addressJSON: b.addressJSON(), expectedPeer: b.identityID)
                channel.close()
                Issue.record("A peer without reciprocal approval completed mutual TLS")
            } catch { }
            _ = try b.approvePeer(a.identityID)
            var address = try JSONSerialization.jsonObject(with: Data(b.addressJSON().utf8)) as? [String: Any] ?? [:]
            address["identity_id"] = String(repeating: "0", count: 64)
            let encoded = try JSONSerialization.data(withJSONObject: address)
            #expect(throws: QANativeTransportError.unauthorized) {
                try a.connect(addressJSON: String(decoding: encoded, as: UTF8.self), expectedPeer: b.identityID)
            }
        }.value
    }

    @Test
    func revocationUnblocksPendingReadAndCloseUnblocksAccept() async throws {
        let firstMaterial = try Material.create()
        let secondMaterial = try Material.create()
        try await Task.detached {
            let first = try firstMaterial.identity()
            let second = try secondMaterial.identity()
            defer { try? first.removeFromKeychain(); try? second.removeFromKeychain() }
            let a = try QANativeTransport(identity: first)
            let b = try QANativeTransport(identity: second)
            defer { a.close(); b.close() }
            _ = try a.approvePeer(b.identityID)
            _ = try b.approvePeer(a.identityID)
            let connected = try a.connect(addressJSON: b.addressJSON(), expectedPeer: b.identityID)
            let accepted = try b.accept()
            defer { connected.close(); accepted.close() }
            let waiting = Task.detached { () -> Bool in
                do { _ = try accepted.read(maxBytes: 1); return false }
                catch { return true }
            }
            try await Task.sleep(for: .milliseconds(30))
            let began = ContinuousClock.now
            #expect(b.revokePeer(a.identityID))
            #expect(await waiting.value)
            #expect(began.duration(to: .now) < .seconds(3))
            let accepting = Task.detached { () -> Bool in
                do { _ = try b.accept(); return false }
                catch QANativeTransportError.closed { return true }
                catch { return false }
            }
            try await Task.sleep(for: .milliseconds(30))
            b.close()
            #expect(await accepting.value)
        }.value
    }

    @Test
    func invalidRelayConfigurationFailsBeforeConnection() async throws {
        try await Task.detached {
            let material = try Material.create()
            let identity = try material.identity()
            defer { try? identity.removeFromKeychain() }
            let malformed = QANativeRelayConfiguration(host: "https://example.invalid", port: 443,
                                                       serverCertificateDERBase64: material.certificate.base64EncodedString(), ticket: "qa")
            #expect(throws: QANativeTransportError.invalidConfiguration) {
                try QANativeTransport(identity: identity, relay: malformed)
            }
        }.value
    }

    @Test
    func revocationRejectsPreviouslyBufferedBytes() async throws {
        try await Task.detached {
            let first = try Material.create().identity()
            let second = try Material.create().identity()
            defer { try? first.removeFromKeychain(); try? second.removeFromKeychain() }
            let a = try QANativeTransport(identity: first)
            let b = try QANativeTransport(identity: second)
            defer { a.close(); b.close() }
            _ = try a.approvePeer(b.identityID)
            _ = try b.approvePeer(a.identityID)
            let connected = try a.connect(addressJSON: b.addressJSON(), expectedPeer: b.identityID)
            let accepted = try b.accept()
            defer { connected.close(); accepted.close() }
            _ = try connected.write(Data([1, 2]))
            #expect(try accepted.read(maxBytes: 1) == Data([1]))
            #expect(b.revokePeer(a.identityID))
            #expect(throws: QANativeTransportError.closed) { try accepted.read(maxBytes: 1) }
        }.value
    }

    @Test
    func listenerRemainsUsableAfterCapacityRejections() async throws {
        try await Task.detached {
            let first = try Material.create().identity()
            let second = try Material.create().identity()
            let third = try Material.create().identity()
            defer { try? first.removeFromKeychain(); try? second.removeFromKeychain(); try? third.removeFromKeychain() }
            let a = try QANativeTransport(identity: first)
            let b = try QANativeTransport(identity: second)
            let c = try QANativeTransport(identity: third)
            defer { a.close(); b.close(); c.close() }
            _ = try a.approvePeer(b.identityID)
            _ = try b.approvePeer(a.identityID)
            _ = try a.approvePeer(c.identityID)
            _ = try c.approvePeer(a.identityID)
            var outgoing: [QANativeByteChannel] = []
            var accepted: [QANativeByteChannel] = []
            defer { outgoing.forEach { $0.close() }; accepted.forEach { $0.close() } }
            for _ in 0..<16 {
                outgoing.append(try a.connect(addressJSON: b.addressJSON(), expectedPeer: b.identityID))
                accepted.append(try b.accept())
            }
            let address = try a.addressJSON()
            let unexpected = await withTaskGroup(of: Bool.self) { group in
                for _ in 0..<16 {
                    group.addTask {
                        do {
                            let extra = try c.connect(addressJSON: address, expectedPeer: a.identityID)
                            extra.close()
                            return true
                        } catch { return false }
                    }
                }
                var admitted = false
                for await result in group { admitted = admitted || result }
                return admitted
            }
            #expect(!unexpected)
            outgoing.forEach { $0.close() }
            accepted.forEach { $0.close() }
            let restored = try c.connect(addressJSON: a.addressJSON(), expectedPeer: a.identityID)
            let inbound = try a.accept()
            defer { restored.close(); inbound.close() }
            _ = try restored.write(Data([1]))
            #expect(try inbound.read(maxBytes: 1) == Data([1]))
        }.value
    }
}
