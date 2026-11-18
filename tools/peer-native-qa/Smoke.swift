import Foundation
import PeerNativeQA
import Darwin

private enum SmokeError: Error { case failed(String) }

@main
struct NativeSmoke {
    static func main() async throws {
        try await Task.detached {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-native-smoke-\(UUID().uuidString)")
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
                guard process.terminationStatus == 0 else { throw SmokeError.failed("fixture_generation") }
            }
            func identity(_ name: String) throws -> QANativeIdentity {
                let key = root.appendingPathComponent("\(name).key")
                let certificate = root.appendingPathComponent("\(name).der")
                let pkcs8 = root.appendingPathComponent("\(name)-key.der")
                try run(["ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", key.path])
                try run(["req", "-new", "-x509", "-key", key.path, "-sha256", "-days", "2",
                         "-subj", "/CN=pennel-peer-native.qa", "-outform", "DER", "-out", certificate.path])
                try run(["pkcs8", "-topk8", "-nocrypt", "-in", key.path, "-outform", "DER", "-out", pkcs8.path])
                return try QANativeIdentity(certificateDER: Data(contentsOf: certificate),
                                            privateKeyDER: Data(contentsOf: pkcs8), account: "peer-compare-\(UUID().uuidString)")
            }
            func check(_ value: Bool, _ label: String) throws {
                guard value else { throw SmokeError.failed(label) }
            }
            func stage(_ value: String) { FileHandle.standardError.write(Data("native-smoke: \(value)\n".utf8)) }
            stage("identity")
            let first = try identity("first")
            let second = try identity("second")
            defer { try? first.removeFromKeychain(); try? second.removeFromKeychain() }
            let a = try QANativeTransport(identity: first)
            let b = try QANativeTransport(identity: second)
            defer { a.close(); b.close() }
            _ = try a.approvePeer(b.identityID)
            _ = try b.approvePeer(a.identityID)
            stage("handshake")
            let sent = try a.connect(addressJSON: b.addressJSON(), expectedPeer: b.identityID)
            defer { sent.close() }
            let received = try b.accept()
            stage("content")
            defer { received.close() }
            try check(sent.peerID == b.identityID && received.peerID == a.identityID, "mutual_identity")
            _ = try sent.write(Data("hello".utf8))
            try check(try received.read(maxBytes: 64) == Data("hello".utf8), "content")
            try sent.finishWrite()
            stage("half-close")
            try check(try received.read(maxBytes: 64).isEmpty, "half_close")
            _ = try received.write(Data("ack".utf8))
            try check(try sent.read(maxBytes: 64) == Data("ack".utf8), "final_ack")
            try received.finishWrite()
            try check(try sent.read(maxBytes: 64).isEmpty, "reverse_eof")
            sent.close()
            received.close()
            let next = try a.connect(addressJSON: b.addressJSON(), expectedPeer: b.identityID)
            defer { next.close() }
            let nextReceived = try b.accept()
            stage("revocation")
            defer { nextReceived.close() }
            let waiting = Task.detached { () -> Bool in
                do { _ = try nextReceived.read(maxBytes: 1); return false }
                catch { return true }
            }
            try await Task.sleep(for: .milliseconds(30))
            let began = ContinuousClock.now
            try check(b.revokePeer(a.identityID), "revoke_result")
            try check(await waiting.value, "revoke_read")
            try check(began.duration(to: .now) < .seconds(3), "revoke_deadline")
            if let binary = ProcessInfo.processInfo.environment["PENNEL_NATIVE_QA_RELAY_BINARY"] {
                stage("forced-relay")
                let relay = Process()
                let input = Pipe()
                let output = Pipe()
                relay.executableURL = URL(fileURLWithPath: binary)
                relay.arguments = ["--bind", "127.0.0.1:0", "--allow", first.identityID, "--allow", second.identityID,
                                   "--candidate", "native", "--run-id", "native-smoke-\(UUID().uuidString)"]
                relay.standardInput = input
                relay.standardOutput = output
                relay.standardError = FileHandle.nullDevice
                try relay.run()
                defer {
                    try? input.fileHandleForWriting.write(contentsOf: Data("{\"command\":\"shutdown\"}\n".utf8))
                    if relay.isRunning { relay.terminate() }
                }
                var line = Data()
                while line.last != 0x0a {
                    var descriptor = pollfd(fd: output.fileHandleForReading.fileDescriptor, events: Int16(POLLIN), revents: 0)
                    guard poll(&descriptor, 1, 10_000) > 0,
                          let byte = try output.fileHandleForReading.read(upToCount: 1), !byte.isEmpty,
                          line.count < 16_384 else { throw SmokeError.failed("relay_start") }
                    line.append(byte)
                }
                struct Ready: Decodable { let clients: [String: QANativeRelayConfiguration] }
                let ready = try JSONDecoder().decode(Ready.self, from: line)
                guard let firstRelay = ready.clients[first.identityID], let secondRelay = ready.clients[second.identityID] else {
                    throw SmokeError.failed("relay_config")
                }
                let relayA = try QANativeTransport(identity: first, relay: firstRelay)
                let relayB = try QANativeTransport(identity: second, relay: secondRelay)
                defer { relayA.close(); relayB.close() }
                _ = try relayA.approvePeer(second.identityID)
                _ = try relayB.approvePeer(first.identityID)
                // Keep the first source's offer listener alive while it makes an outgoing
                // request, then reverse with the second source's listener still waiting.
                let waitingAtA = Task.detached { try relayA.accept() }
                for index in 0..<3 {
                    let accepting = Task.detached { try relayB.accept() }
                    let connected = try relayA.connect(addressJSON: relayB.addressJSON(), expectedPeer: second.identityID)
                    let accepted = try await accepting.value
                    let payload = Data(repeating: UInt8(index), count: 64 * 1024)
                    _ = try connected.write(payload)
                    try check(try accepted.read(maxBytes: 65_536) == payload, "relay_content")
                    try connected.finishWrite()
                    try check(try accepted.read(maxBytes: 1).isEmpty, "relay_half_close")
                    _ = try accepted.write(Data("ack".utf8))
                    try accepted.finishWrite()
                    try check(try connected.read(maxBytes: 3) == Data("ack".utf8), "relay_ack")
                    try check(try connected.read(maxBytes: 1).isEmpty, "relay_eof")
                    connected.close()
                    accepted.close()
                }
                let waitingAtB = Task.detached { () -> Bool in
                    do {
                        let unexpected = try relayB.accept()
                        unexpected.close()
                        return false
                    } catch QANativeTransportError.closed { return true }
                    catch { return false }
                }
                let reverse = try relayB.connect(addressJSON: relayA.addressJSON(), expectedPeer: first.identityID)
                let reverseAccepted = try await waitingAtA.value
                _ = try reverse.write(Data("reverse".utf8))
                try check(try reverseAccepted.read(maxBytes: 64) == Data("reverse".utf8), "relay_reverse_content")
                try reverse.finishWrite()
                try check(try reverseAccepted.read(maxBytes: 1).isEmpty, "relay_reverse_half_close")
                _ = try reverseAccepted.write(Data("ack".utf8))
                try reverseAccepted.finishWrite()
                try check(try reverse.read(maxBytes: 3) == Data("ack".utf8), "relay_reverse_ack")
                try check(try reverse.read(maxBytes: 1).isEmpty, "relay_reverse_eof")
                reverse.close()
                reverseAccepted.close()
                relayA.close()
                relayB.close()
                try check(await waitingAtB.value, "relay_passive_accept_retirement")
            }
            print("{\"native_smoke\":\"passed\",\"mutual_identity\":true,\"exact_bytes\":true,\"half_close_ack\":true,\"revoke_unblocks_read\":true}")
        }.value
    }
}
