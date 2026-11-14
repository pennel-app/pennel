#if PENNEL_PEER_TRANSPORT_QA
import CryptoKit
import Darwin
import Foundation
import Testing
@testable internal import PennelMobile

@Suite(.serialized)
struct PeerComparisonQualificationTests {
    private enum Failure: Error { case invalidProfile, invalidConfiguration, invalidResponse, invalidReceipt }

    @Test
    func bothCandidatesUseTheSameDurableFileEngine() async throws {
        try await Task.detached {
            guard AppPaths.qaProfile == "peer-transport" else { throw Failure.invalidProfile }
            for candidate in ["quinn", "native"] {
                let root = try Self.directory()
                defer { try? FileManager.default.removeItem(at: root) }
                let store = PeerComparisonIdentityKeychain()
                let aKey = "peer-probe-local-\(UUID().uuidString)"
                let bKey = "peer-probe-local-\(UUID().uuidString)"
                defer { try? store.remove(account: aKey); try? store.remove(account: bKey) }
                let a = try Self.probe(candidate, store, aKey, nil, "127.0.0.1")
                let b = try Self.probe(candidate, store, bKey, nil, "127.0.0.1")
                defer { a.close(); b.close() }
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

    @Test(.enabled(if: ProcessInfo.processInfo.environment["PENNEL_PEER_COMPARISON_CONFIG"] != nil))
    func syntheticMacIPhoneComparison() async throws {
        guard AppPaths.qaProfile == "peer-transport", let value = ProcessInfo.processInfo.environment["PENNEL_PEER_COMPARISON_CONFIG"],
              let configs = try JSONSerialization.jsonObject(with: Data(value.utf8)) as? [[String: String]] else {
            throw Failure.invalidConfiguration
        }
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
        let store = PeerComparisonIdentityKeychain()
        let account = "peer-probe-device-\(UUID().uuidString)"
        defer { try? store.remove(account: account) }
        let identity = try await Task.detached { try PeerComparisonFactory.identity(store: store, account: account) }.value
        let prepared: Preparation = try await post(url, token, "prepare", ["peer_id": identity.identityId])
        guard prepared.candidate == candidate, prepared.route == route else { throw Failure.invalidResponse }
        let host = try localIPv4()
        let probe = try await Task.detached { try Self.probe(candidate, store, account, prepared.relay_config_json, host) }.value
        defer { probe.close() }
        let fixture: Fixture = try await getFixture(url, token)
        guard fixture.candidate == candidate, fixture.route == route, fixture.source_identity == prepared.source_identity else { throw Failure.invalidResponse }
        let returned = try await Task.detached {
            _ = try probe.approvePeer(id: fixture.source_identity)
            let target = root.appendingPathComponent("received.bin")
            let stage = root.appendingPathComponent("partial")
            let first = try probe.receiveFile(addressJson: fixture.source_address_json, staging: stage.path, target: target.path, maxBlocks: 1)
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
                              _ relay: String?, _ host: String) throws -> PeerComparisonProbe {
        if candidate == "quinn" {
            return try PeerComparisonProbe.newQuinn(identityStore: store, identityAccount: account,
                bindAddress: "\(host):0", relayConfigJson: relay)
        }
        return try PeerComparisonFactory.native(identity: PeerComparisonFactory.identity(store: store, account: account),
            account: account, bindHost: host, advertisedHost: host, relayJSON: relay)
    }

    private static func directory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PeerComparison-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        return directory.resolvingSymlinksInPath()
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
