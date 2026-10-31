import Foundation
import Observation
import os

/// What the Account screen needs from the engine; a fake stands in for it in tests.
protocol CloudService: Sendable {
    func cloudLoginStart(deviceName: String, platform: String) throws -> CloudLoginStart
    func cloudLoginPoll(deviceCode: String) throws -> CloudLoginPoll
    func cloudAccount() throws -> CloudAccount
    func cloudDevices() throws -> [CloudDevice]
    func cloudLogout() throws
    func cloudDeleteAccount() throws
    func cloudRenameDevice(id: String, name: String) throws
    func cloudRevokeDevice(id: String) throws
    func cloudSyncEnable() throws -> String
    func cloudSyncDisable() throws
    func cloudSyncUnlock(code: String) throws
    func cloudSyncReset() throws -> String
    func cloudSyncNewRecoveryCode() throws -> String
    func cloudSyncConfirmDeletes() throws
    func cloudSyncLast() -> SyncReport
}

extension PennelEngine: CloudService {}

/// The device-code login loop, as the desktop's `pollLogin`: wait the interval, ask, add 5 s on `slow_down`, stop at expiry.
enum LoginPolling {
    enum Outcome: Equatable { case done, expired, cancelled }

    static let slowDown: TimeInterval = 5
    static let lifetime: TimeInterval = 15 * 60

    @MainActor
    static func run(
        interval: TimeInterval,
        expiresAt: Date,
        now: () -> Date,
        sleep: (TimeInterval) async -> Void,
        cancelled: () -> Bool,
        poll: () async throws -> CloudLoginPoll
    ) async throws -> Outcome {
        var wait = interval
        while true {
            if cancelled() { return .cancelled }
            if now() >= expiresAt { return .expired }
            await sleep(wait)
            if cancelled() { return .cancelled }
            if now() >= expiresAt { return .expired }
            let result = try await poll()
            if cancelled() { return .cancelled }
            switch result {
            case .done: return .done
            case .expired: return .expired
            case .slowDown: wait += slowDown
            case .pending: break
            }
        }
    }
}

@MainActor
@Observable
final class CloudAccountModel {
    enum Login: Equatable {
        case idle
        case code(CloudLoginStart)
        case expired
    }

    nonisolated static let platform = "ios"
    static let defaultDeviceName = "iPhone"
    static let confirmedKey = "pennel.recoveryCodeConfirmed"
    /// Same text as the core's mass-delete brake, which the report only carries as an error message.
    static let massDelete = "Many meetings disappeared from your folder. Sync has deleted nothing; confirm under Settings → Account if this was intentional."

    static func isMassDelete(_ message: String?) -> Bool {
        message == massDelete || message == "Veel meetings zijn verdwenen uit je map. Sync heeft niets verwijderd; bevestig in Instellingen → Account als dat de bedoeling was."
    }

    private(set) var login = Login.idle
    private(set) var account: CloudAccount?
    private(set) var accountError: String?
    private(set) var devices: [CloudDevice]?
    private(set) var report: SyncReport
    private(set) var loaded = false
    private(set) var signedOut = false
    var offerCloud = false
    /// Held in memory only: shown once and never written anywhere.
    private(set) var shownCode: String?
    var askCode = false
    var lostCode = false
    var codeDraft = ""
    var deviceName = CloudAccountModel.defaultDeviceName
    var notice: String?

    @ObservationIgnored private let service: CloudService
    @ObservationIgnored private let runner: CloudSyncRunner
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let settings: @MainActor () -> Settings
    @ObservationIgnored private let settingsChanged: @MainActor () -> Void
    @ObservationIgnored private let openURL: @MainActor (URL) -> Void
    @ObservationIgnored private let now: () -> Date
    @ObservationIgnored private let sleep: (TimeInterval) async -> Void
    @ObservationIgnored private var loginRun = 0

    private static let log = Logger(subsystem: "app.pennel.ios", category: "cloud")

    init(
        service: CloudService,
        runner: CloudSyncRunner,
        defaults: UserDefaults = AppPaths.defaults,
        settings: @escaping @MainActor () -> Settings,
        settingsChanged: @escaping @MainActor () -> Void = {},
        openURL: @escaping @MainActor (URL) -> Void,
        now: @escaping () -> Date = Date.init,
        sleep: @escaping (TimeInterval) async -> Void = { try? await Task.sleep(for: .seconds($0)) }
    ) {
        self.service = service
        self.runner = runner
        self.defaults = defaults
        self.settings = settings
        self.settingsChanged = settingsChanged
        self.openURL = openURL
        self.now = now
        self.sleep = sleep
        report = service.cloudSyncLast()
    }

    var syncEnabled: Bool { settings().cloudSync }
    var syncing: Bool { runner.isRunning }

    /// Only an explicit false counts: the flag is absent on devices that never created a code.
    var recoveryUnconfirmed: Bool {
        defaults.object(forKey: Self.confirmedKey) as? Bool == false && syncEnabled
    }

    var needsVault: Bool {
        switch report.status {
        case .needsSetup, .needsRecoveryCode, .vaultReset: true
        default: false
        }
    }

    var unlockVisible: Bool {
        if shownCode != nil { return false }
        switch report.status {
        case .needsRecoveryCode, .vaultReset: return askCode || syncEnabled
        default: return askCode
        }
    }

    var braked: Bool {
        if case .error(let message) = report.status { return Self.isMassDelete(message) }
        return false
    }

    var usage: CloudFormat.Usage? { account.map(CloudFormat.usage) }

    // MARK: Loading

    /// Re-reads everything the screen shows; a signed-out account shows the login block only.
    func refresh() async {
        report = await runPlain { $0.cloudSyncLast() }
        do {
            let loadedAccount = try await run { try $0.cloudAccount() }
            account = loadedAccount
            accountError = nil
            signedOut = false
            devices = try? await run { try $0.cloudDevices() }
        } catch PennelError.NotSignedIn {
            account = nil
            devices = nil
            signedOut = true
        } catch {
            accountError = ErrorText.failed(error)
            signedOut = false
            devices = nil
        }
        loaded = true
    }

    // MARK: Login

    func beginLogin() async {
        loginRun += 1
        let run = loginRun
        notice = nil
        let name = deviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        let device = name.isEmpty ? Self.defaultDeviceName : name
        do {
            let start = try await self.run { try $0.cloudLoginStart(deviceName: device, platform: Self.platform) }
            guard run == loginRun else { return }
            login = .code(start)
            if let url = URL(string: start.verificationUri) { openURL(url) }
            let outcome = try await LoginPolling.run(
                interval: TimeInterval(max(1, start.intervalSecs)),
                expiresAt: now().addingTimeInterval(LoginPolling.lifetime),
                now: now, sleep: sleep, cancelled: { run != loginRun }
            ) { [self] in
                do { return try await self.run { try $0.cloudLoginPoll(deviceCode: start.deviceCode) } }
                catch PennelError.Network { return .pending }
            }
            switch outcome {
            case .cancelled: return
            case .expired: login = .expired
            case .done:
                login = .idle
                offerCloud = true
                await refresh()
                settingsChanged()
                runner.poke()
            }
        } catch {
            guard run == loginRun else { return }
            login = .idle
            notice = ErrorText.failed(error)
        }
    }

    func reopenBrowser() {
        if case .code(let start) = login, let url = URL(string: start.verificationUri) { openURL(url) }
    }

    func cancelLogin() {
        loginRun += 1
        login = .idle
    }

    // MARK: Account

    func logout() async {
        await perform(refreshAfter: false) {
            try await self.run { try $0.cloudLogout() }
            self.resetLocalState()
        }
        await refresh()
        settingsChanged()
    }

    func deleteAccount() async {
        await perform(refreshAfter: false) {
            try await self.run { try $0.cloudDeleteAccount() }
            self.resetLocalState()
        }
        await refresh()
        settingsChanged()
    }

    private func resetLocalState() {
        loginRun += 1
        login = .idle
        offerCloud = false
        shownCode = nil
        askCode = false
        lostCode = false
        codeDraft = ""
        notice = nil
        defaults.removeObject(forKey: Self.confirmedKey)
    }

    func rename(_ device: CloudDevice, to name: String) async {
        let next = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !next.isEmpty, next != device.name else { return }
        await perform { try await self.run { try $0.cloudRenameDevice(id: device.id, name: next) } }
    }

    func revoke(_ device: CloudDevice) async {
        await perform { try await self.run { try $0.cloudRevokeDevice(id: device.id) } }
    }

    // MARK: Sync and recovery code

    func setSync(_ on: Bool) async {
        if !on {
            await perform { try await self.run { try $0.cloudSyncDisable() } }
            settingsChanged()
            return
        }
        await createCode()
    }

    func createCode() async {
        notice = nil
        do {
            showCode(try await run { try $0.cloudSyncEnable() })
            askCode = false
        } catch PennelError.VaultLocked {
            askCode = true
        } catch {
            notice = ErrorText.failed(error)
        }
        await refresh()
        settingsChanged()
        runner.poke()
    }

    func unlock() async {
        let code = codeDraft
        await perform {
            try await self.run { try $0.cloudSyncUnlock(code: code) }
            self.defaults.set(true, forKey: Self.confirmedKey)
            self.codeDraft = ""
            self.askCode = false
        }
        settingsChanged()
        runner.poke()
    }

    func resetVault() async {
        await perform {
            self.showCode(try await self.run { try $0.cloudSyncReset() })
            self.askCode = false
            self.lostCode = false
            self.codeDraft = ""
        }
        runner.poke()
    }

    func newRecoveryCode() async {
        await perform { self.showCode(try await self.run { try $0.cloudSyncNewRecoveryCode() }) }
    }

    func confirmDeletes() async {
        await perform { try await self.run { try $0.cloudSyncConfirmDeletes() } }
        runner.poke()
    }

    func codeSaved() {
        shownCode = nil
        defaults.set(true, forKey: Self.confirmedKey)
    }

    func syncNow() {
        runner.poke()
    }

    /// The runner publishes each finished pass; the screen follows it.
    func adopt(_ report: SyncReport) {
        self.report = report
    }

    private func showCode(_ code: String) {
        shownCode = code
        defaults.set(false, forKey: Self.confirmedKey)
    }

    private func perform(refreshAfter: Bool = true, _ work: () async throws -> Void) async {
        notice = nil
        do { try await work() } catch { notice = ErrorText.failed(error) }
        if refreshAfter { await refresh() }
    }

    /// The core blocks on the network; keep that off the main actor.
    private func run<T: Sendable>(_ work: @escaping @Sendable (CloudService) throws -> T) async throws -> T {
        let service = service
        return try await Task.detached { try work(service) }.value
    }

    private func runPlain<T: Sendable>(_ work: @escaping @Sendable (CloudService) -> T) async -> T {
        let service = service
        return await Task.detached { work(service) }.value
    }
}
