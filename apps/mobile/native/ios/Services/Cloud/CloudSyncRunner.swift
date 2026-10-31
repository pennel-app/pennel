import BackgroundTasks
import Foundation
import Observation
import os

/// Lets a recording create its folder while no sync pass can be scanning the meetings folder.
@MainActor
protocol SyncHolding: AnyObject {
    /// Defers new passes and waits for the one in flight; false when it did not finish in time (nothing is held then).
    func hold(maxWait: Duration) async -> Bool
    func release()
}

/// Runs `cloud_sync_run` with at most one pass at a time and one queued rerun, whatever the number of triggers.
/// Meetings not yet finalised are passed on every pass: their folders are never pushed before then (spec §5.6).
@MainActor
@Observable
final class CloudSyncRunner: SyncHolding {
    typealias Pass = @Sendable ([String]) async -> SyncReport
    /// Schedules a repeating callback and returns its cancel closure.
    typealias Timers = @MainActor (TimeInterval, @escaping @MainActor () -> Void) -> () -> Void

    static let interval: TimeInterval = 180

    static let shared = CloudSyncRunner(
        pass: { ids in
            let engine = await MainActor.run { AppModel.shared.engine }
            return await Task.detached { engine.cloudSyncRun(recordingMeetingIds: ids) }.value
        },
        unfinalisedMeetingIds: { MeetingRuntime.shared.unfinalisedMeetingIds })

    private(set) var report: SyncReport?
    private(set) var isRunning = false

    @ObservationIgnored private let pass: Pass
    @ObservationIgnored private let unfinalisedMeetingIds: @MainActor () -> [String]
    @ObservationIgnored private let timers: Timers
    @ObservationIgnored private var current: Task<Void, Never>?
    @ObservationIgnored private var rerunQueued = false
    @ObservationIgnored private var cancelTimer: (() -> Void)?
    @ObservationIgnored private var attached = false
    @ObservationIgnored private var holds = 0
    @ObservationIgnored private var deferred = false

    private static let log = Logger(subsystem: "app.pennel.ios", category: "sync")

    init(
        pass: @escaping Pass,
        unfinalisedMeetingIds: @escaping @MainActor () -> [String],
        timers: @escaping Timers = CloudSyncRunner.systemTimers
    ) {
        self.pass = pass
        self.unfinalisedMeetingIds = unfinalisedMeetingIds
        self.timers = timers
    }

    static let systemTimers: Timers = { interval, fire in
        let timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { _ in
            MainActor.assumeIsolated(fire)
        }
        return { timer.invalidate() }
    }

    /// Every trigger ends up here: a pass in flight gets exactly one rerun, however many pokes arrive.
    func poke() {
        if holds > 0 {
            deferred = true
            return
        }
        if current != nil {
            rerunQueued = true
            return
        }
        current = Task { await drain() }
    }

    func hold(maxWait: Duration = .seconds(15)) async -> Bool {
        holds += 1
        let deadline = ContinuousClock.now + maxWait
        while current != nil {
            if ContinuousClock.now >= deadline {
                release()
                return false
            }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return true
    }

    func release() {
        holds = max(0, holds - 1)
        if holds == 0, deferred {
            deferred = false
            poke()
        }
    }

    /// Resolves when no pass is running or queued.
    func idle() async {
        while let task = current { await task.value }
    }

    func enterForeground() {
        poke()
        guard cancelTimer == nil else { return }
        cancelTimer = timers(Self.interval) { [weak self] in self?.poke() }
    }

    func enterBackground() {
        cancelTimer?()
        cancelTimer = nil
    }

    /// Finish and write triggers of the meeting runtime. The slots are single, so the previous handlers are kept.
    func attach(to runtime: MeetingRuntime) {
        guard !attached else { return }
        attached = true
        let finished = runtime.onFinished
        runtime.onFinished = { [weak self] finish in
            finished?(finish)
            self?.poke()
        }
        let updated = runtime.onUpdated
        runtime.onUpdated = { [weak self] id in
            updated?(id)
            self?.poke()
        }
    }

    private func drain() async {
        repeat {
            rerunQueued = false
            isRunning = true
            report = await pass(unfinalisedMeetingIds())
        } while rerunQueued && holds == 0
        if rerunQueued { deferred = true }
        isRunning = false
        current = nil
    }
}

/// `BGAppRefreshTask` `app.pennel.ios.sync`: iOS decides when it runs, and Pennel promises nothing about it.
@MainActor
enum CloudSyncBackground {
    static var identifier: String { (Bundle.main.bundleIdentifier ?? "app.pennel.ios") + ".sync" }
    private static let earliest: TimeInterval = 15 * 60
    private static let log = Logger(subsystem: "app.pennel.ios", category: "sync")

    /// Must run before the app finishes launching.
    static func register() {
        guard !AppPaths.isQAProfile else { return }
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { task in
            MainActor.assumeIsolated { run(task) }
        }
    }

    static func schedule() {
        guard !AppPaths.isQAProfile else { return }
        let request = BGAppRefreshTaskRequest(identifier: identifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: earliest)
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            log.error("refresh not scheduled: \(ErrorText.code(error), privacy: .public)")
        }
    }

    private static func run(_ task: BGTask) {
        schedule()
        let runner = CloudSyncRunner.shared
        var completed = false
        let complete = { (success: Bool) in
            guard !completed else { return }
            completed = true
            task.setTaskCompleted(success: success)
        }
        Task {
            runner.poke()
            await runner.idle()
            complete(true)
        }
        task.expirationHandler = { MainActor.assumeIsolated { complete(false) } }
    }
}
