#if PENNEL_PEER_TRANSPORT_QA
import CryptoKit
import Darwin
import Foundation
import Testing
import UIKit
#if canImport(React)
import React
internal import Expo
#endif
@testable internal import PennelMobile

/// Checkpoint and resume are separate operator-controlled runs. Neither phase claims
/// that iOS kept a suspended application reachable or transferred while it was locked.
@Suite(.serialized)
struct PeerComparisonLifecycleTests {
    private static let fixtureBytes: UInt64 = 8_388_633
    private static let fixtureDigest = "c8fe33c65893641cacb6bd1620c25f19e8100bc26bb88ba361bf5cc2583671d2"
    private enum Failure: Error {
        case configuration, persistentState, response, receipt, reload, overlap, storage
    }

    private struct Configuration: Sendable {
        let base: URL
        let token: String
        let candidate: String
        let route: String
        let phase: String
        let lifecycleID: String

        init(_ value: [String: String]) throws {
            guard let text = value["coordinator_url"], let base = URL(string: text), base.scheme == "http",
                  base.host != nil, base.user == nil, base.password == nil, base.fragment == nil,
                  let token = value["token"], !token.isEmpty, token.utf8.count <= 4096,
                  let candidate = value["candidate"], ["quinn", "native"].contains(candidate),
                  value["route"] == "forced_relay", let phase = value["phase"],
                  ["checkpoint", "resume"].contains(phase), let lifecycle = value["lifecycle_id"],
                  !lifecycle.isEmpty, lifecycle.utf8.count <= 40,
                  lifecycle.utf8.allSatisfy({ $0 == 45 || (48...57).contains($0)
                      || (65...90).contains($0) || (97...122).contains($0) }) else { throw Failure.configuration }
            self.base = base
            self.token = token
            self.candidate = candidate
            self.route = "forced_relay"
            self.phase = phase
            self.lifecycleID = lifecycle
        }

        var account: String { "peer-probe-lifecycle-\(lifecycleID)" }
    }

    private struct Checkpoint: Codable, Sendable {
        let schema_version: Int
        let lifecycle_id: String
        let candidate: String
        let route: String
        let phone_identity: String
        let source_identity: String
        let sha256: String
        let total_bytes: UInt64
        let block_size: UInt32
        let first_bytes: UInt64
    }

    private final class Context: @unchecked Sendable {
        let config: Configuration
        let directory: URL
        let store: PeerComparisonIdentityKeychain
        let identity: PeerCompareIdentity
        let probe: PeerComparisonProbe
        let fixture: Fixture
        let checkpoint: Checkpoint?
        let nativeIdentity: QANativeIdentity?
        let metrics = PeerComparisonProcessMetrics()
        let began = ProcessInfo.processInfo.systemUptime

        init(config: Configuration, directory: URL, store: PeerComparisonIdentityKeychain,
             identity: PeerCompareIdentity, probe: PeerComparisonProbe, fixture: Fixture,
             checkpoint: Checkpoint?, nativeIdentity: QANativeIdentity?) {
            self.config = config
            self.directory = directory
            self.store = store
            self.identity = identity
            self.probe = probe
            self.fixture = fixture
            self.checkpoint = checkpoint
            self.nativeIdentity = nativeIdentity
        }

        var target: URL { directory.appendingPathComponent("received.bin") }
        var stage: URL { directory.appendingPathComponent("partial", isDirectory: true) }
    }

    private final class JobState: @unchecked Sendable {
        private let lock = NSLock()
        private var started = false
        private var finished = false
        func begin() { lock.withLock { started = true } }
        func end() { lock.withLock { finished = true } }
        func ongoing() -> Bool { lock.withLock { started && !finished } }
    }

    @Test(.enabled(if: ProcessInfo.processInfo.environment["PENNEL_PEER_LIFECYCLE_CONFIG_FILE"] == "1"))
    func checkpointAndResumeAcrossJavaScriptReload() async throws {
        guard AppPaths.qaProfile == "peer-transport" else { throw Failure.configuration }
        let configs = try PeerComparisonConfiguration.load("qa-lifecycle.json").map(Configuration.init)
        guard configs.count == 2, Set(configs.map(\.candidate)) == ["quinn", "native"],
              Set(configs.map(\.phase)).count == 1, Set(configs.map(\.lifecycleID)).count == 2 else {
            throw Failure.configuration
        }
        if configs[0].phase == "resume" {
            // Establish a fully loaded replacement instance before starting either
            // transfer. The later reload must replace that instance again.
            try await Self.reloadJavaScript()
            Self.marker(8)
        }
        let contexts = try await Self.contexts(configs)
        defer { contexts.forEach { $0.probe.cancel(); $0.probe.close() } }
        if configs[0].phase == "checkpoint" {
            Self.marker(1)
            for context in contexts {
                try await Task.detached { try Self.checkpoint(context) }.value
            }
            Self.marker(2)
            return
        }

        // All probes and their stable keychain identities exist before any React reload.
        Self.marker(3)
        let states = contexts.map { _ in JobState() }
        let tasks = zip(contexts, states).map { context, state in
            Task.detached {
                state.begin()
                defer { state.end() }
                return try Self.resume(context)
            }
        }
        defer {
            contexts.forEach { $0.probe.cancel(); $0.probe.close() }
            tasks.forEach { $0.cancel() }
        }
        let began = ContinuousClock.now
        while !states.allSatisfy({ $0.ongoing() }) {
            guard began.duration(to: .now) < .seconds(2), !tasks.contains(where: \.isCancelled) else {
                throw Failure.overlap
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        Self.marker(4)
        try await Self.reloadJavaScript()
        guard states.allSatisfy({ $0.ongoing() }) else { throw Failure.overlap }
        Self.marker(5)

        var receipts: [Resumed] = []
        for task in tasks { receipts.append(try await task.value) }
        Self.marker(6)
        // No coordinator success or cleanup is published until actual JS reload evidence
        // and both native transfers have passed their independent byte checks.
        for (context, receipt) in zip(contexts, receipts) {
            try await Self.finish(context, receipt: receipt)
        }
        Self.marker(7)
    }

    private static func contexts(_ configs: [Configuration]) async throws -> [Context] {
        var result: [Context] = []
        do {
            for config in configs {
                let directory = try ownedDirectory(config)
                let checkpoint: Checkpoint? = config.phase == "resume" ? try readCheckpoint(directory) : nil
                let store = PeerComparisonIdentityKeychain()
                let identity = try await Task.detached {
                    if config.phase == "resume", try store.load(account: config.account) == nil {
                        throw Failure.persistentState
                    }
                    return try PeerComparisonFactory.identity(store: store, account: config.account)
                }.value
                if let checkpoint {
                    guard checkpoint.schema_version == 1, checkpoint.lifecycle_id == config.lifecycleID,
                          checkpoint.candidate == config.candidate, checkpoint.route == config.route,
                          checkpoint.phone_identity == identity.identityId else { throw Failure.persistentState }
                }
                let prepared: Preparation = try await post(config, "prepare", ["peer_id": identity.identityId])
                guard prepared.candidate == config.candidate, prepared.route == config.route else { throw Failure.response }
                let host = try PeerComparisonQualificationTests.localIPv4()
                let nativeIdentity: QANativeIdentity? = try await Task.detached {
                    guard config.candidate == "native" else { return nil }
                    guard let certificate = Data(base64Encoded: identity.certificateDerB64),
                          let key = Data(base64Encoded: identity.privateKeyDerB64) else { throw Failure.persistentState }
                    // Retain an exact scoped cleanup handle; the shared factory reuses these items.
                    return try QANativeIdentity(certificateDER: certificate, privateKeyDER: key, account: config.account)
                }.value
                let probe = try await Task.detached {
                    try PeerComparisonQualificationTests.probe(config.candidate, store, config.account,
                                                               prepared.relay_config_json, host)
                }.value
                do {
                    let fixture = try await getFixture(config)
                    guard fixture.candidate == config.candidate, fixture.route == config.route,
                          fixture.source_identity == prepared.source_identity, validDigest(fixture.source_identity),
                          fixture.sha256 == fixtureDigest, fixture.block_size == 1024 * 1024,
                          fixture.total_bytes == fixtureBytes else { throw Failure.response }
                    if let checkpoint {
                        guard checkpoint.source_identity == fixture.source_identity, checkpoint.sha256 == fixture.sha256,
                              checkpoint.total_bytes == fixture.total_bytes, checkpoint.block_size == fixture.block_size,
                              checkpoint.first_bytes == UInt64(fixture.block_size) else { throw Failure.persistentState }
                    }
                    _ = try await Task.detached { try probe.approvePeer(id: fixture.source_identity) }.value
                    result.append(Context(config: config, directory: directory, store: store, identity: identity,
                                          probe: probe, fixture: fixture, checkpoint: checkpoint, nativeIdentity: nativeIdentity))
                } catch { probe.close(); throw error }
            }
            return result
        } catch {
            result.forEach { $0.probe.cancel(); $0.probe.close() }
            throw error
        }
    }

    private static func checkpoint(_ context: Context) throws {
        let first = try context.probe.receiveFile(addressJson: context.fixture.source_address_json,
            staging: context.stage.path, target: context.target.path, maxBlocks: 1)
        guard !first.complete, first.bytesReceived == UInt64(context.fixture.block_size), first.blocksTransferred == 1,
              first.sha256 == context.fixture.sha256, first.totalBytes == context.fixture.total_bytes,
              !FileManager.default.fileExists(atPath: context.target.path) else { throw Failure.receipt }
        try regularFile(context.stage.appendingPathComponent("manifest.json"), limit: 2 * 1024 * 1024)
        let block = context.stage.appendingPathComponent("block-00000000.part")
        try regularFile(block, limit: Int(context.fixture.block_size))
        guard (try FileManager.default.attributesOfItem(atPath: block.path)[.size] as? NSNumber)?.uint64Value == first.bytesReceived else {
            throw Failure.receipt
        }
        let record = Checkpoint(schema_version: 1, lifecycle_id: context.config.lifecycleID,
            candidate: context.config.candidate, route: context.config.route, phone_identity: context.identity.identityId,
            source_identity: context.fixture.source_identity, sha256: context.fixture.sha256,
            total_bytes: context.fixture.total_bytes, block_size: context.fixture.block_size, first_bytes: first.bytesReceived)
        let file = context.directory.appendingPathComponent("checkpoint.json")
        guard !FileManager.default.fileExists(atPath: file.path) else { throw Failure.persistentState }
        try JSONEncoder().encode(record).write(to: file, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        try sync(file, directory: false)
        try sync(context.directory, directory: true)
        context.probe.close()
    }

    private static func resume(_ context: Context) throws -> Resumed {
        guard let checkpoint = context.checkpoint else { throw Failure.persistentState }
        let resumed = try context.probe.receiveFile(addressJson: context.fixture.source_address_json,
            staging: context.stage.path, target: context.target.path, maxBlocks: nil)
        guard resumed.complete, resumed.bytesReceived == checkpoint.total_bytes - checkpoint.first_bytes,
              resumed.totalBytes == checkpoint.total_bytes, resumed.sha256 == checkpoint.sha256 else { throw Failure.receipt }
        let unchanged = try context.probe.receiveFile(addressJson: context.fixture.source_address_json,
            staging: context.stage.path, target: context.target.path, maxBlocks: nil)
        guard unchanged.complete, unchanged.bytesReceived == 0,
              digest(try Data(contentsOf: context.target)) == checkpoint.sha256 else { throw Failure.receipt }
        _ = try context.probe.offerFile(path: context.target.path, blockSize: checkpoint.block_size)
        return Resumed(first: checkpoint.first_bytes, resumed: resumed.bytesReceived, unchanged: unchanged.bytesReceived)
    }

    private static func finish(_ context: Context, receipt: Resumed) async throws {
        let address = try await Task.detached { try context.probe.addressJson() }.value
        let returned: Upload = try await post(context.config, "upload", ["address_json": address])
        guard returned.complete, returned.sha256 == context.fixture.sha256,
              returned.bytes_received == context.fixture.total_bytes else { throw Failure.receipt }
        let result = DeviceResult(candidate: context.config.candidate, route: context.config.route, complete: true,
            first_bytes: receipt.first, resumed_bytes: receipt.resumed, unchanged_bytes: receipt.unchanged,
            returned_bytes: returned.bytes_received, sha256: context.fixture.sha256,
            elapsed_seconds: max(0, ProcessInfo.processInfo.systemUptime - context.began), metrics: context.metrics.snapshot())
        let saved: Saved = try await post(context.config, "result", result)
        guard saved.saved else { throw Failure.receipt }
        context.probe.close()
        try context.nativeIdentity?.removeFromKeychain()
        try context.store.remove(account: context.config.account)
        // This directory was validated against the fixed QA root and lifecycle identifier.
        try FileManager.default.removeItem(at: context.directory)
    }

    private static func ownedDirectory(_ config: Configuration) throws -> URL {
        guard let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else { throw Failure.storage }
        let base = documents.appendingPathComponent("QA/peer-transport", isDirectory: true)
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let directory = base.appendingPathComponent("lifecycle-\(config.lifecycleID)", isDirectory: true)
        if config.phase == "checkpoint" {
            guard !FileManager.default.fileExists(atPath: directory.path) else { throw Failure.persistentState }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false,
                                                   attributes: [.posixPermissions: 0o700])
        }
        let values = try directory.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        let attributes = try FileManager.default.attributesOfItem(atPath: directory.path)
        guard values.isDirectory == true, values.isSymbolicLink != true,
              (attributes[.posixPermissions] as? NSNumber)?.intValue == 0o700,
              let resolved = Darwin.realpath(directory.path, nil), let basePath = Darwin.realpath(base.path, nil) else {
            throw Failure.persistentState
        }
        defer { free(resolved); free(basePath) }
        let canonical = String(cString: resolved)
        guard canonical == String(cString: basePath) + "/lifecycle-" + config.lifecycleID else { throw Failure.persistentState }
        return URL(fileURLWithPath: canonical, isDirectory: true)
    }

    private static func readCheckpoint(_ directory: URL) throws -> Checkpoint {
        let file = directory.appendingPathComponent("checkpoint.json")
        try regularFile(file, limit: 16 * 1024)
        let record = try JSONDecoder().decode(Checkpoint.self, from: Data(contentsOf: file))
        guard validDigest(record.phone_identity), validDigest(record.source_identity), validDigest(record.sha256),
              record.block_size == 1024 * 1024, record.first_bytes == UInt64(record.block_size),
              record.total_bytes == fixtureBytes, record.sha256 == fixtureDigest else { throw Failure.persistentState }
        try regularFile(directory.appendingPathComponent("partial/manifest.json"), limit: 2 * 1024 * 1024)
        return record
    }

    private static func regularFile(_ file: URL, limit: Int) throws {
        let values = try file.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard values.isRegularFile == true, values.isSymbolicLink != true,
              let size = values.fileSize, size > 0, size <= limit else { throw Failure.persistentState }
    }

    private static func sync(_ file: URL, directory: Bool) throws {
        let flags = O_RDONLY | O_NOFOLLOW | (directory ? O_DIRECTORY : 0)
        let descriptor = file.path.withCString { Darwin.open($0, flags) }
        guard descriptor >= 0 else { throw Failure.storage }
        defer { Darwin.close(descriptor) }
        guard Darwin.fsync(descriptor) == 0 else { throw Failure.storage }
    }

    private final class ReloadWitness: @unchecked Sendable {
        let stream: AsyncStream<Bool>
        private let continuation: AsyncStream<Bool>.Continuation
        private let lock = NSLock()
        private var command = false
        private var loaded = false
        private var observers: [NSObjectProtocol] = []

        init() {
            let pair = AsyncStream<Bool>.makeStream(bufferingPolicy: .bufferingNewest(1))
            stream = pair.stream
            continuation = pair.continuation
        }

        @MainActor
        func trigger() throws {
            #if canImport(React)
            guard let provider = UIApplication.shared.delegate as? any ExpoReactNativeFactoryProvider,
                  let factory = provider.reactNativeFactory,
                  let host = factory.rootViewFactory.perform(NSSelectorFromString("reactHost"))?.takeUnretainedValue() as? NSObject,
                  let previousPresenter = Self.presenter(host) else { throw Failure.reload }
            let commandToken = NotificationCenter.default.addObserver(
                forName: .RCTTriggerReloadCommand, object: nil, queue: .main) { [weak self] note in
                guard note.userInfo?[RCTTriggerReloadCommandReasonKey] as? String == "Peer transport QA" else { return }
                self?.lock.withLock { self?.command = true }
            }
            let loadToken = NotificationCenter.default.addObserver(
                forName: .RCTJavaScriptDidLoad, object: nil, queue: .main) { [weak self] note in
                // Both selectors are public React APIs. Swift's prebuilt umbrella
                // forward-declares these classes, so invoke their public getters.
                guard let self, let instance = note.object as? NSObject,
                      let loadedPresenter = Self.presenter(instance),
                      loadedPresenter !== previousPresenter,
                      loadedPresenter === Self.presenter(host) else { return }
                let observed = self.lock.withLock {
                    guard self.command, !self.loaded else { return false }
                    self.loaded = true
                    return true
                }
                if observed { self.continuation.yield(true); self.continuation.finish() }
            }
            lock.withLock { observers = [commandToken, loadToken] }
            RCTTriggerReloadCommandListeners("Peer transport QA")
            #else
            throw Failure.reload
            #endif
        }

        private static func presenter(_ object: NSObject) -> NSObject? {
            let getter = NSSelectorFromString("surfacePresenter")
            guard object.responds(to: getter) else { return nil }
            return object.perform(getter)?.takeUnretainedValue() as? NSObject
        }

        func remove() {
            let tokens = lock.withLock { let result = observers; observers.removeAll(); return result }
            tokens.forEach { NotificationCenter.default.removeObserver($0) }
            continuation.finish()
        }
        deinit { remove() }
    }

    private static func reloadJavaScript() async throws {
        let witness = ReloadWitness()
        defer { witness.remove() }
        try await witness.trigger()
        try await withThrowingTaskGroup(of: Bool.self) { group in
            group.addTask {
                for await loaded in witness.stream { return loaded }
                throw Failure.reload
            }
            group.addTask { try await Task.sleep(for: .seconds(30)); throw Failure.reload }
            guard try await group.next() == true else { throw Failure.reload }
            group.cancelAll()
        }
    }

    private static func request<T: Decodable>(_ value: URLRequest) async throws -> T {
        let (data, response) = try await URLSession.shared.data(for: value)
        guard (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 32 * 1024 else { throw Failure.response }
        return try JSONDecoder().decode(T.self, from: data)
    }

    private static func post<T: Decodable, B: Encodable>(_ config: Configuration, _ path: String, _ body: B) async throws -> T {
        var query = URLRequest(url: config.base.appendingPathComponent(path))
        query.httpMethod = "POST"
        query.timeoutInterval = 90
        query.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        query.setValue("application/json", forHTTPHeaderField: "Content-Type")
        query.setValue("no-store", forHTTPHeaderField: "Cache-Control")
        query.httpBody = try JSONEncoder().encode(body)
        return try await request(query)
    }

    private static func getFixture(_ config: Configuration) async throws -> Fixture {
        for _ in 0..<100 {
            var query = URLRequest(url: config.base.appendingPathComponent("fixture"))
            query.timeoutInterval = 5
            query.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
            let (data, response) = try await URLSession.shared.data(for: query)
            if (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 32 * 1024 {
                return try JSONDecoder().decode(Fixture.self, from: data)
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        throw Failure.response
    }

    private static func validDigest(_ value: String) -> Bool {
        value.utf8.count == 64 && value.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
    private static func digest(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
    private static func marker(_ phase: Int) { FileHandle.standardError.write(Data("Peer QA lifecycle phase=\(phase) candidates=2.\n".utf8)) }
    private struct Preparation: Decodable, Sendable { let candidate: String; let route: String; let source_identity: String; let relay_config_json: String? }
    private struct Fixture: Decodable, Sendable { let candidate: String; let route: String; let source_identity: String; let source_address_json: String; let block_size: UInt32; let total_bytes: UInt64; let sha256: String }
    private struct Resumed: Sendable { let first: UInt64; let resumed: UInt64; let unchanged: UInt64 }
    private struct Upload: Decodable { let complete: Bool; let sha256: String; let bytes_received: UInt64 }
    private struct Saved: Decodable { let saved: Bool }
    private struct DeviceResult: Encodable {
        let candidate: String; let route: String; let complete: Bool
        let first_bytes: UInt64; let resumed_bytes: UInt64; let unchanged_bytes: UInt64; let returned_bytes: UInt64
        let sha256: String; let elapsed_seconds: Double; let metrics: PeerComparisonProcessMetrics.Snapshot
    }
}
#endif
