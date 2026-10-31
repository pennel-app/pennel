import Foundation
import Network
import Security
import Testing
@testable internal import PennelMobile

private final class MockCloudServer: @unchecked Sendable {
    enum StartupError: Error, Equatable { case cancelled, deadline, invalidPort }

    private let listener: NWListener
    private let lock = NSLock()
    private var bodies: [String: [String: Any]] = [:]
    private let queue: DispatchQueue
    private var startup: Result<UInt16, any Error>?
    private(set) var url = ""

    init(queue: DispatchQueue = DispatchQueue(label: "mock-cloud"), startupTimeout: Duration = .seconds(5)) async throws {
        guard startupTimeout > .zero else { throw StartupError.deadline }
        self.queue = queue
        listener = try NWListener(using: .tcp, on: .any)
        listener.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            let result: Result<UInt16, any Error>
            switch state {
            case .ready:
                if let port = self.listener.port?.rawValue, port > 0 { result = .success(port) }
                else { result = .failure(StartupError.invalidPort) }
            case .failed(let error): result = .failure(error)
            case .cancelled: result = .failure(StartupError.cancelled)
            default: return
            }
            self.lock.withLock { if self.startup == nil { self.startup = result } }
        }
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { connection.cancel(); return }
            self.handle(connection)
        }
        listener.start(queue: queue)
        let deadline = ContinuousClock.now.advanced(by: startupTimeout)
        do {
            while true {
                try Task.checkCancellation()
                if let result = lock.withLock({ startup }) {
                    url = try Self.endpoint(port: result.get())
                    return
                }
                guard ContinuousClock.now < deadline else { throw StartupError.deadline }
                try await Task.sleep(for: .milliseconds(10))
            }
        } catch {
            listener.cancel()
            throw error
        }
    }

    static func endpoint(port: UInt16?) throws -> String {
        guard let port, port > 0 else { throw StartupError.invalidPort }
        return "http://127.0.0.1:\(port)"
    }

    func body(for path: String) -> [String: Any]? { lock.withLock { bodies[path] } }

    func stop() { listener.cancel() }

    private func handle(_ connection: NWConnection) {
        connection.start(queue: queue)
        receive(connection, buffer: Data())
    }

    private func receive(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65_536) { [self] data, _, done, error in
            var buffer = buffer
            if let data { buffer.append(data) }
            if let request = Self.parse(buffer) {
                respond(connection, request: request)
            } else if done || error != nil {
                connection.cancel()
            } else {
                receive(connection, buffer: buffer)
            }
        }
    }

    private static func parse(_ data: Data) -> (path: String, body: Data)? {
        let separator = Data("\r\n\r\n".utf8)
        guard let end = data.range(of: separator), let head = String(data: data[..<end.lowerBound], encoding: .utf8) else {
            return nil
        }
        let length = head.split(separator: "\r\n")
            .first { $0.lowercased().hasPrefix("content-length:") }
            .flatMap { Int($0.split(separator: ":")[1].trimmingCharacters(in: .whitespaces)) } ?? 0
        let body = data[end.upperBound...]
        guard body.count >= length else { return nil }
        let path = head.split(separator: " ").dropFirst().first.map(String.init) ?? ""
        return (path, Data(body.prefix(length)))
    }

    private func respond(_ connection: NWConnection, request: (path: String, body: Data)) {
        let json = (try? JSONSerialization.jsonObject(with: request.body)) as? [String: Any]
        lock.withLock { bodies[request.path] = json }
        let (status, payload) = request.path == "/v1/auth/device"
            ? ("200 OK", #"{"user_code":"ABCD-EFGH","verification_uri":"https://example.invalid/verify","device_code":"dev-1","interval":1}"#)
            : ("404 Not Found", "{}")
        let response = "HTTP/1.1 \(status)\r\nContent-Type: application/json\r\nContent-Length: \(payload.utf8.count)\r\nConnection: close\r\n\r\n\(payload)"
        connection.send(content: Data(response.utf8), contentContext: .finalMessage, isComplete: true,
                        completion: .contentProcessed { _ in connection.cancel() })
    }
}

private final class MemorySecrets: SecretStore, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = [:]
    func get(account: String) -> String? { lock.withLock { values[account] } }
    func set(account: String, value: String?) { lock.withLock { values[account] = value } }
}

private final class FakeCloud: CloudService, @unchecked Sendable {
    let code: String
    init(code: String) { self.code = code }

    func cloudLoginStart(deviceName: String, platform: String) throws -> CloudLoginStart { throw PennelError.NotSignedIn }
    func cloudLoginPoll(deviceCode: String) throws -> CloudLoginPoll { .pending }
    func cloudAccount() throws -> CloudAccount { throw PennelError.NotSignedIn }
    func cloudDevices() throws -> [CloudDevice] { [] }
    func cloudLogout() throws {}
    func cloudDeleteAccount() throws {}
    func cloudRenameDevice(id: String, name: String) throws {}
    func cloudRevokeDevice(id: String) throws {}
    func cloudSyncEnable() throws -> String { code }
    func cloudSyncDisable() throws {}
    func cloudSyncUnlock(code: String) throws {}
    func cloudSyncReset() throws -> String { code }
    func cloudSyncNewRecoveryCode() throws -> String { code }
    func cloudSyncConfirmDeletes() throws {}
    func cloudSyncLast() -> SyncReport {
        SyncReport(status: .needsSetup, uploaded: 0, downloaded: 0, conflicts: 0, lastSuccessMs: nil)
    }
}

@MainActor
private func idleRunner() -> CloudSyncRunner {
    CloudSyncRunner(pass: { _ in SyncReport(status: .off, uploaded: 0, downloaded: 0, conflicts: 0, lastSuccessMs: nil) },
                    unfinalisedMeetingIds: { [] }, timers: { _, _ in {} })
}

private func filesContaining(_ needle: Data, under root: URL) -> [String] {
    var hits: [String] = []
    let files = FileManager.default.enumerator(at: root, includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey])
    for case let url as URL in files ?? NSEnumerator() {
        let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard values?.isRegularFile == true, (values?.fileSize ?? 0) < 50_000_000,
              let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { continue }
        if data.range(of: needle) != nil { hits.append(url.lastPathComponent) }
    }
    return hits
}

@MainActor
struct CloudAccountTests {
    @Test func mock_listener_becomes_ready_without_blocking_the_main_queue() async throws {
        let server = try await MockCloudServer(queue: .main)
        defer { server.stop() }
        let port = try #require(URL(string: server.url)?.port)
        #expect(port > 0)
    }

    @Test func mock_listener_reports_deadline_and_rejects_missing_ports() async throws {
        await #expect(throws: MockCloudServer.StartupError.deadline) {
            _ = try await MockCloudServer(queue: .main, startupTimeout: .zero)
        }
        let cancelled = Task { try await MockCloudServer(queue: .main) }
        cancelled.cancel()
        await #expect(throws: CancellationError.self) { _ = try await cancelled.value }
        #expect(throws: MockCloudServer.StartupError.invalidPort) { try MockCloudServer.endpoint(port: nil) }
        #expect(throws: MockCloudServer.StartupError.invalidPort) { try MockCloudServer.endpoint(port: 0) }
    }

    @Test func login_start_sends_ios_platform() async throws {
        let server = try await MockCloudServer()
        defer { server.stop() }
        setenv("PENNEL_CLOUD_URL", server.url, 1)
        defer { unsetenv("PENNEL_CLOUD_URL") }

        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-cloud-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let engine = try PennelEngine.newPreview(dataDir: dir.path)
        try engine.setSecretStore(store: MemorySecrets())
        var opened: [URL] = []
        let model = CloudAccountModel(
            service: engine, runner: idleRunner(), defaults: UserDefaults(suiteName: "pennel-test-\(UUID().uuidString)")!,
            settings: { engine.settings() }, openURL: { opened.append($0) },
            sleep: { _ in try? await Task.sleep(for: .milliseconds(20)) })

        let login = Task { await model.beginLogin() }
        var waitError: (any Error)?
        do {
            let deadline = ContinuousClock.now.advanced(by: .seconds(5))
            while opened.isEmpty && model.notice == nil && ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(10))
            }
        } catch { waitError = error }
        let openedBeforeCleanup = !opened.isEmpty
        let cause = model.notice.map { $0 == "Failed: No connection" ? "network" : "explicit model notice" }
            ?? "verification-page deadline"
        model.cancelLogin()
        login.cancel()
        await login.value
        if let waitError { throw waitError }
        if !openedBeforeCleanup { Issue.record("Login did not open the fixture verification page: \(cause).") }

        let body = try #require(server.body(for: "/v1/auth/device"))
        #expect(body["platform"] as? String == "ios")
        #expect(body["device_name"] as? String == "iPhone")
        #expect(opened.map(\.absoluteString) == ["https://example.invalid/verify"])
    }

    @Test func recovery_code_not_persisted() async throws {
        let code = "TEST-\(UUID().uuidString)-CODE"
        let defaults = UserDefaults(suiteName: "pennel-test-\(UUID().uuidString)")!
        weak var released: CloudAccountModel?
        do {
            let model = CloudAccountModel(
                service: FakeCloud(code: code), runner: idleRunner(), defaults: defaults,
                settings: { defaultSettings() }, openURL: { _ in })
            released = model
            await model.setSync(true)
            #expect(model.shownCode == code)
            model.codeSaved()
            #expect(model.shownCode == nil)
        }
        #expect(released == nil)

        let needle = Data(code.utf8)
        let hits = filesContaining(needle, under: URL(fileURLWithPath: NSHomeDirectory()))
        #expect(hits.isEmpty)
        #expect(!String(describing: defaults.dictionaryRepresentation()).contains(code))
        #expect(!String(describing: UserDefaults.standard.dictionaryRepresentation()).contains(code))

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecReturnData as String: true,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        var result: CFTypeRef?
        if SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let items = result as? [[String: Any]] {
            for item in items {
                let data = item[kSecValueData as String] as? Data ?? Data()
                #expect(data.range(of: needle) == nil)
            }
        }
    }

    @Test func usage_and_relative_time_match_the_desktop() {
        let account = CloudAccount(email: "x", audioSeconds: 5_400, tokens: 1_250_000, storageBytes: 42_000_000,
                                   limitAudioSeconds: 36_000, limitTokens: 5_000_000, limitStorageBytes: 2_000_000_000)
        let usage = CloudFormat.usage(account)
        #expect(usage.audio == "1 u 30 min van 10 u")
        #expect(usage.tokens == "1,3 mln van 5 mln tokens")
        #expect(usage.storage == "42 MB van 2 GB")
        #expect(CloudFormat.relativeTime(thenMs: 0, nowMs: 30_000) == "zojuist")
        #expect(CloudFormat.relativeTime(thenMs: 0, nowMs: 5 * 60_000) == "5 min geleden")
        #expect(CloudFormat.relativeTime(thenMs: 0, nowMs: 3 * 3_600_000) == "3 u geleden")
        #expect(CloudFormat.relativeTime(thenMs: 0, nowMs: 86_400_000) == "1 dag geleden")
    }

    @Test func login_polling_slows_down_and_expires() async throws {
        var waits: [TimeInterval] = []
        var clock = Date(timeIntervalSince1970: 0)
        var answers: [CloudLoginPoll] = [.pending, .slowDown, .pending]
        let outcome = try await LoginPolling.run(
            interval: 5, expiresAt: Date(timeIntervalSince1970: 100), now: { clock },
            sleep: { waits.append($0); clock += $0 }, cancelled: { false },
            poll: { answers.isEmpty ? .pending : answers.removeFirst() })
        #expect(Array(waits.prefix(4)) == [5, 5, 10, 10])
        #expect(outcome == .expired)
    }
}
