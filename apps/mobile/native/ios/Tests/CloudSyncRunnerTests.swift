import Foundation
import Testing
@testable internal import PennelMobile

private actor Gate {
    private(set) var calls: [[String]] = []
    private var waiters: [CheckedContinuation<Void, Never>] = []
    private var opened = false

    func enter(_ ids: [String]) async {
        calls.append(ids)
        guard !opened else { return }
        await withCheckedContinuation { waiters.append($0) }
    }

    func open() {
        opened = true
        waiters.forEach { $0.resume() }
        waiters = []
    }
}

private let emptyReport = SyncReport(status: .off, uploaded: 0, downloaded: 0, conflicts: 0, lastSuccessMs: nil)

@MainActor
private final class Fixture {
    let gate = Gate()
    var active: String?
    var retained: String?
    var scheduled: [(interval: TimeInterval, fire: @MainActor () -> Void)] = []
    private(set) var cancelled = 0
    private(set) var runner: CloudSyncRunner!

    init() {
        let gate = gate
        runner = CloudSyncRunner(
            pass: { ids in
                await gate.enter(ids)
                return emptyReport
            },
            unfinalisedMeetingIds: { [unowned self] in [active, retained].compactMap { $0 } },
            timers: { [unowned self] interval, fire in
                scheduled.append((interval, fire))
                return { [unowned self] in cancelled += 1 }
            })
    }

    func waitForCalls(_ count: Int) async {
        while await gate.calls.count < count { await Task.yield() }
    }
}

@MainActor
struct CloudSyncRunnerTests {
    @Test func runner_coalesces_pokes() async {
        let fixture = Fixture()
        fixture.runner.poke()
        await fixture.waitForCalls(1)
        for _ in 0..<4 { fixture.runner.poke() }
        await fixture.gate.open()
        await fixture.runner.idle()

        #expect(await fixture.gate.calls.count == 2)
        #expect(!fixture.runner.isRunning)
    }

    @Test func runner_passes_active_meeting_id() async {
        let fixture = Fixture()
        fixture.active = "2026-10-25 Weekly"
        fixture.runner.poke()
        await fixture.waitForCalls(1)
        fixture.active = nil
        fixture.runner.poke()
        await fixture.gate.open()
        await fixture.runner.idle()
        fixture.runner.poke()
        await fixture.runner.idle()

        #expect(await fixture.gate.calls == [["2026-10-25 Weekly"], [], []])
    }

    @Test func runner_passes_a_retained_meeting_with_the_recording_one() async {
        let fixture = Fixture()
        fixture.active = "2026-10-26 Nieuw"
        fixture.retained = "2026-10-25 Niet bewaard"
        await fixture.gate.open()
        fixture.runner.poke()
        await fixture.runner.idle()

        #expect(await fixture.gate.calls == [["2026-10-26 Nieuw", "2026-10-25 Niet bewaard"]])
    }

    @Test func timer_runs_only_while_foregrounded() async {
        let fixture = Fixture()
        await fixture.gate.open()
        fixture.runner.enterForeground()
        fixture.runner.enterForeground()
        await fixture.runner.idle()
        #expect(fixture.scheduled.map(\.interval) == [180])

        fixture.scheduled[0].fire()
        await fixture.runner.idle()
        #expect(await fixture.gate.calls.count == 2)

        fixture.runner.enterBackground()
        #expect(fixture.cancelled == 1)
    }

    @Test func pass_in_flight_never_overlaps_a_starting_recording() async {
        let fixture = Fixture()
        fixture.runner.poke()
        await fixture.waitForCalls(1)

        var events: [String] = []
        let started = AsyncStream<Void>.makeStream()
        let start = Task { @MainActor in
            started.continuation.yield()
            started.continuation.finish()
            let held = await fixture.runner.hold(maxWait: .seconds(60))
            events.append("held \(held)")
            fixture.active = "new-meeting"
            fixture.runner.poke()
            events.append("folder created")
            fixture.runner.release()
        }
        _ = await started.stream.first(where: { @Sendable _ in true })
        #expect(events.isEmpty)

        await fixture.gate.open()
        await start.value
        await fixture.runner.idle()

        #expect(events == ["held true", "folder created"])
        #expect(await fixture.gate.calls == [[], ["new-meeting"]])
    }

    @Test func hold_gives_up_when_the_pass_does_not_finish() async {
        let fixture = Fixture()
        fixture.runner.poke()
        await fixture.waitForCalls(1)

        #expect(await fixture.runner.hold(maxWait: .milliseconds(100)) == false)
        fixture.runner.poke()
        await fixture.gate.open()
        await fixture.runner.idle()
        #expect(await fixture.gate.calls.count == 2)
    }
}
