#if PENNEL_PEER_TRANSPORT_QA
import CryptoKit
import Darwin
import Foundation
import Security
import Testing
@testable internal import PennelMobile

struct PeerComparisonTrialIdentity: Sendable {
    let store = PeerComparisonIdentityKeychain()
    let account = "peer-probe-trial-\(UUID().uuidString)"
    let identity: PeerCompareIdentity
    private let candidate: String
    private let nativeItems: QANativeIdentity.KeychainItems?

    init(candidate: String) throws {
        guard ["quinn", "native"].contains(candidate) else { throw PeerCompareError.InvalidConfig }
        self.candidate = candidate
        identity = try PeerComparisonFactory.identity(store: store, account: account, requireNew: true)
        do {
            nativeItems = candidate == "native" ? try PeerComparisonFactory.nativeItems(identity: identity, account: account) : nil
        } catch {
            try store.remove(account: account)
            throw error
        }
    }

    func probe(relay: String?, host: String) throws -> PeerComparisonProbe {
        try PeerComparisonQualificationTests.probe(candidate, store, account, relay, host, identity: identity)
    }

    func remove() {
        do { try nativeItems?.remove() }
        catch { Issue.record(error, "Could not remove the trial's native keychain items.") }
        do { try store.remove(account: account) }
        catch { Issue.record(error, "Could not remove the trial's identity record.") }
    }
}

@Suite(.serialized)
struct PeerComparisonQualificationTests {
    private enum Failure: Error { case invalidProfile, invalidConfiguration, invalidResponse, invalidReceipt }

    #if targetEnvironment(simulator)
    @Test
    func oneShotNativeTrialsRemoveOnlyTheirOwnKeychainItems() async throws {
        try await Task.detached {
            guard AppPaths.qaProfile == "peer-transport" else { throw Failure.invalidProfile }
            func nativeQueries(_ trial: PeerComparisonTrialIdentity) -> (certificate: [String: Any], key: [String: Any]) {
                let label = "app.pennel.peer-compare.qa.\(trial.account).\(trial.identity.identityId)"
                return ([kSecClass as String: kSecClassCertificate, kSecAttrLabel as String: label,
                         kSecUseDataProtectionKeychain as String: true],
                        [kSecClass as String: kSecClassKey, kSecAttrApplicationTag as String: Data(label.utf8),
                         kSecUseDataProtectionKeychain as String: true])
            }
            func checkItems(_ trial: PeerComparisonTrialIdentity, present: Bool) throws {
                let hasIdentityRecord = try trial.store.load(account: trial.account) != nil
                #expect(hasIdentityRecord == present)
                let queries = nativeQueries(trial)
                for query in [queries.certificate, queries.key] {
                    let status = SecItemCopyMatching(query as CFDictionary, nil)
                    #expect(status == (present ? errSecSuccess : errSecItemNotFound))
                }
            }
            func run(_ trial: PeerComparisonTrialIdentity, host: String) throws {
                defer { trial.remove() }
                let probe = try trial.probe(relay: nil, host: host)
                defer { probe.close() }
                try checkItems(trial, present: true)
            }

            let control = try PeerComparisonTrialIdentity(candidate: "native")
            defer { control.remove() }
            let controlProbe = try control.probe(relay: nil, host: "127.0.0.1")
            defer { controlProbe.close() }
            #expect(throws: PeerCompareError.Storage) {
                _ = try PeerComparisonFactory.identity(store: control.store, account: control.account, requireNew: true)
            }
            let completed = try PeerComparisonTrialIdentity(candidate: "native")
            try run(completed, host: "127.0.0.1")
            try checkItems(completed, present: false)

            let failed = try PeerComparisonTrialIdentity(candidate: "native")
            #expect(throws: QANativeTransportError.invalidConfiguration) {
                try run(failed, host: "invalid-host")
            }
            try checkItems(failed, present: false)

            let partial = try PeerComparisonTrialIdentity(candidate: "native")
            do {
                defer { partial.remove() }
                let probe = try partial.probe(relay: nil, host: "127.0.0.1")
                probe.close()
                // Reproduce the state an import leaves between its key and certificate writes.
                let status = SecItemDelete(nativeQueries(partial).certificate as CFDictionary)
                #expect(status == errSecSuccess)
            }
            try checkItems(partial, present: false)
            try checkItems(control, present: true)
            let reopenedID = try PeerComparisonFactory.identity(store: control.store, account: control.account).identityId
            #expect(reopenedID == control.identity.identityId)
        }.value
    }
    #endif

    @Test
    func ownedStorageBoundaryDiagnostics() async throws {
        try await Task.detached {
            guard AppPaths.qaProfile == "peer-transport" else { throw Failure.invalidProfile }
            let root = try Self.directory()
            defer { try? FileManager.default.removeItem(at: root) }
            var item = root
            for depth in 0..<16 {
                var value = stat()
                let result = item.path.withCString { Darwin.lstat($0, &value) }
                let code = result == 0 ? 0 : errno
                let mode = result == 0 ? UInt32(value.st_mode) : 0
                FileHandle.standardError.write(Data("Peer QA filesystem ancestor depth=\(depth) errno=\(code) mode=\(mode).\n".utf8))
                if item.path == "/" { break }
                item.deleteLastPathComponent()
            }
            let descriptor = root.path.withCString { Darwin.open($0, O_RDONLY | O_DIRECTORY | O_NOFOLLOW) }
            let opened = descriptor >= 0 ? 0 : errno
            var synced: Int32 = -1
            if descriptor >= 0 { synced = Darwin.fsync(descriptor) == 0 ? 0 : errno; Darwin.close(descriptor) }
            FileHandle.standardError.write(Data("Peer QA filesystem directory-open errno=\(opened) directory-sync errno=\(synced).\n".utf8))
            let source = root.appendingPathComponent("synthetic")
            let linked = root.appendingPathComponent("linked")
            try Data([0x41]).write(to: source)
            let linkedResult = source.path.withCString { first in linked.path.withCString { Darwin.link(first, $0) } }
            let linkError = linkedResult == 0 ? 0 : errno
            FileHandle.standardError.write(Data("Peer QA filesystem hard-link errno=\(linkError).\n".utf8))
            #expect(opened == 0)
        }.value
    }

    @Test
    func bothCandidatesUseTheSameDurableFileEngine() async throws {
        try await Task.detached {
            guard AppPaths.qaProfile == "peer-transport" else { throw Failure.invalidProfile }
            for candidate in ["quinn", "native"] {
                let root = try Self.directory()
                defer { try? FileManager.default.removeItem(at: root) }
                let firstTrial = try PeerComparisonTrialIdentity(candidate: candidate)
                defer { firstTrial.remove() }
                let a = try firstTrial.probe(relay: nil, host: "127.0.0.1")
                defer { a.close() }
                let secondTrial = try PeerComparisonTrialIdentity(candidate: candidate)
                defer { secondTrial.remove() }
                let b = try secondTrial.probe(relay: nil, host: "127.0.0.1")
                defer { b.close() }
                _ = try a.approvePeer(id: b.identityId())
                _ = try b.approvePeer(id: a.identityId())
                let source = root.appendingPathComponent("synthetic.bin")
                let original = Data(repeating: 0x37, count: 300_001)
                try original.write(to: source)
                _ = try a.offerFile(path: source.path, blockSize: 128 * 1024)
                let target = root.appendingPathComponent("received.bin")
                let stage = root.appendingPathComponent("partial")
                let first = try b.receiveFile(addressJson: a.addressJson(), staging: stage.path, target: target.path, maxBlocks: 1)
                #expect(!first.complete && first.bytesReceived == 128 * 1024)
                #expect(!FileManager.default.fileExists(atPath: target.path))
                let rest = try b.receiveFile(addressJson: a.addressJson(), staging: stage.path, target: target.path, maxBlocks: nil)
                #expect(rest.complete && first.bytesReceived + rest.bytesReceived == UInt64(original.count))
                #expect(try Data(contentsOf: target) == original)
                let repeatTransfer = try b.receiveFile(addressJson: a.addressJson(), staging: stage.path, target: target.path, maxBlocks: nil)
                #expect(repeatTransfer.complete && repeatTransfer.bytesReceived == 0)
                _ = try b.revokePeer(id: a.identityId())
                do {
                    _ = try b.receiveFile(addressJson: a.addressJson(), staging: stage.path, target: target.path, maxBlocks: nil)
                    Issue.record("A revoked QA peer remained accessible")
                } catch PeerCompareError.Unauthorized { }
            }
        }.value
    }

    @Test(.enabled(if: ProcessInfo.processInfo.environment["PENNEL_PEER_COMPARISON_CONFIG_FILE"] == "1"))
    func syntheticMacIPhoneComparison() async throws {
        let configs = try PeerComparisonConfiguration.load("qa-roundtrip.json")
        for config in configs {
            try await Self.roundTrip(config)
        }
    }

    private static func roundTrip(_ config: [String: String]) async throws {
        guard let address = config["coordinator_url"], let url = URL(string: address), url.scheme == "http",
              let token = config["token"], !token.isEmpty,
              let candidate = config["candidate"], ["native", "quinn"].contains(candidate),
              let route = config["route"], ["direct", "forced_relay"].contains(route) else { throw Failure.invalidConfiguration }
        let began = Date()
        let metrics = PeerComparisonProcessMetrics()
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let trial = try await Task.detached { try PeerComparisonTrialIdentity(candidate: candidate) }.value
        defer { trial.remove() }
        Self.marker("identity ready")
        let prepared: Preparation = try await post(url, token, "prepare", ["peer_id": trial.identity.identityId])
        Self.marker("coordinator prepared")
        guard prepared.candidate == candidate, prepared.route == route else { throw Failure.invalidResponse }
        let host = try localIPv4()
        let probe = try await Task.detached { try trial.probe(relay: prepared.relay_config_json, host: host) }.value
        Self.marker("transport ready")
        defer { probe.close() }
        let fixture: Fixture = try await getFixture(url, token)
        Self.marker("fixture ready")
        guard fixture.candidate == candidate, fixture.route == route, fixture.source_identity == prepared.source_identity else { throw Failure.invalidResponse }
        let returned = try await Task.detached {
            _ = try probe.approvePeer(id: fixture.source_identity)
            let target = root.appendingPathComponent("received.bin")
            let stage = root.appendingPathComponent("partial")
            let first = try probe.receiveFile(addressJson: fixture.source_address_json, staging: stage.path, target: target.path, maxBlocks: 1)
            Self.marker("first durable block received")
            guard !first.complete, !FileManager.default.fileExists(atPath: target.path) else { throw Failure.invalidReceipt }
            let resumed = try probe.receiveFile(addressJson: fixture.source_address_json, staging: stage.path, target: target.path, maxBlocks: nil)
            let unchanged = try probe.receiveFile(addressJson: fixture.source_address_json, staging: stage.path, target: target.path, maxBlocks: nil)
            let digest = SHA256.hash(data: try Data(contentsOf: target)).map { String(format: "%02x", $0) }.joined()
            guard resumed.complete, unchanged.complete, unchanged.bytesReceived == 0,
                  first.bytesReceived + resumed.bytesReceived == fixture.total_bytes,
                  digest == fixture.sha256 else { throw Failure.invalidReceipt }
            _ = try probe.offerFile(path: target.path, blockSize: fixture.block_size)
            return (first.bytesReceived, resumed.bytesReceived, unchanged.bytesReceived)
        }.value
        let peerAddress = try await Task.detached { try probe.addressJson() }.value
        let upload: Upload = try await post(url, token, "upload", ["address_json": peerAddress])
        guard upload.complete, upload.sha256 == fixture.sha256, upload.bytes_received == fixture.total_bytes else { throw Failure.invalidReceipt }
        let result = DeviceResult(candidate: candidate, route: route, complete: true, first_bytes: returned.0,
            resumed_bytes: returned.1, unchanged_bytes: returned.2, returned_bytes: upload.bytes_received,
            sha256: fixture.sha256, elapsed_seconds: Date().timeIntervalSince(began), metrics: metrics.snapshot())
        let saved: Saved = try await post(url, token, "result", result)
        #expect(saved.saved)
    }

    static func probe(_ candidate: String, _ store: PeerComparisonIdentityKeychain, _ account: String,
                      _ relay: String?, _ host: String, identity: PeerCompareIdentity) throws -> PeerComparisonProbe {
        if candidate == "quinn" {
            return try PeerComparisonProbe.newQuinn(identityStore: store, identityAccount: account,
                bindAddress: "\(host):0", relayConfigJson: relay)
        }
        return try PeerComparisonFactory.native(identity: identity,
            account: account, bindHost: host, advertisedHost: host, relayJSON: relay)
    }

    static func directory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PeerComparison-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        // Foundation retains /var on physical iOS; the shared engine rejects every symlink.
        guard let resolved = Darwin.realpath(directory.path, nil) else { throw Failure.invalidConfiguration }
        defer { free(resolved) }
        return URL(fileURLWithPath: String(cString: resolved), isDirectory: true)
    }

    private static func marker(_ stage: String) {
        FileHandle.standardError.write(Data("Peer QA phase: \(stage).\n".utf8))
    }

    static func localIPv4() throws -> String {
        #if targetEnvironment(simulator)
        return "127.0.0.1"
        #else
        var first: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&first) == 0 else { throw Failure.invalidConfiguration }
        defer { freeifaddrs(first) }
        var cursor = first
        while let current = cursor {
            defer { cursor = current.pointee.ifa_next }
            guard let address = current.pointee.ifa_addr, address.pointee.sa_family == UInt8(AF_INET),
                  String(cString: current.pointee.ifa_name) == "en0" else { continue }
            var name = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            guard getnameinfo(address, socklen_t(address.pointee.sa_len), &name, socklen_t(name.count), nil, 0, NI_NUMERICHOST) == 0 else { continue }
            return String(cString: name)
        }
        throw Failure.invalidConfiguration
        #endif
    }

    private static func request<T: Decodable>(_ request: URLRequest) async throws -> T {
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 32 * 1024 else { throw Failure.invalidResponse }
        return try JSONDecoder().decode(T.self, from: data)
    }
    private static func post<T: Decodable, B: Encodable>(_ base: URL, _ token: String, _ path: String, _ body: B) async throws -> T {
        var query = URLRequest(url: base.appendingPathComponent(path))
        query.httpMethod = "POST"; query.timeoutInterval = 90
        query.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        query.setValue("application/json", forHTTPHeaderField: "Content-Type")
        query.setValue("no-store", forHTTPHeaderField: "Cache-Control")
        query.httpBody = try JSONEncoder().encode(body)
        return try await request(query)
    }
    private static func getFixture(_ base: URL, _ token: String) async throws -> Fixture {
        for _ in 0..<100 {
            var query = URLRequest(url: base.appendingPathComponent("fixture"))
            query.timeoutInterval = 5
            query.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            let (data, response) = try await URLSession.shared.data(for: query)
            if (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 32 * 1024 {
                return try JSONDecoder().decode(Fixture.self, from: data)
            }
            try await Task.sleep(for: .milliseconds(200))
        }
        throw Failure.invalidResponse
    }
    private struct Preparation: Decodable, Sendable { let candidate: String; let route: String; let source_identity: String; let relay_config_json: String? }
    private struct Fixture: Decodable, Sendable { let candidate: String; let route: String; let source_address_json: String; let source_identity: String; let block_size: UInt32; let total_bytes: UInt64; let sha256: String }
    private struct Upload: Decodable { let complete: Bool; let sha256: String; let bytes_received: UInt64 }
    private struct Saved: Decodable { let saved: Bool }
    private struct DeviceResult: Encodable { let candidate: String; let route: String; let complete: Bool; let first_bytes: UInt64; let resumed_bytes: UInt64; let unchanged_bytes: UInt64; let returned_bytes: UInt64; let sha256: String; let elapsed_seconds: TimeInterval; let metrics: PeerComparisonProcessMetrics.Snapshot }
}
#endif
