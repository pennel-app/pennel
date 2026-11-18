#if PENNEL_PEER_TRANSPORT_QA
import Foundation
import Testing

struct PeerComparisonObservationTests {
    @Test func firstPCMUsesCallbackTimeEvenWhenStartReturnsLate() {
        func observe(callbackAt: TimeInterval?) -> PeerComparisonFirstPCM {
            var now: TimeInterval = 10
            var callback: TimeInterval?
            return PeerComparisonFirstPCM(start: {
                callback = callbackAt
                now = 15
            }, firstPCM: { callback }, uptime: { now }, wait: { _ in
                Issue.record("A slow Start must not grant a new waiting period.")
                now += 5
            })
        }

        #expect(observe(callbackAt: 11.25).milliseconds == 1250)
        #expect(observe(callbackAt: 12).milliseconds == 2000)
        #expect(observe(callbackAt: 12.001).milliseconds == nil)
        #expect(observe(callbackAt: nil).milliseconds == nil)
    }

    @Test func lateFirstPCMIsRejectedEvenWhenPresentAtTheNextPoll() {
        var now: TimeInterval = 10
        var callback: TimeInterval?
        let timing = PeerComparisonFirstPCM(start: {
            now = 11.999
        }, firstPCM: { callback }, uptime: { now }, wait: { _ in
            now = 12.005
            callback = now
        })
        #expect(timing.deadline == 12)
        #expect(timing.milliseconds == nil)
    }

    @Test func waitingForPCMUsesOnlyTimeLeftAfterStart() {
        var now: TimeInterval = 10
        let timing = PeerComparisonFirstPCM(start: {
            now = 11.75
        }, firstPCM: { nil }, uptime: { now }, wait: { now += $0 })
        #expect(now == 12)
        #expect(timing.milliseconds == nil)
    }

    @Test func timelyPCMRecordedBetweenTheLastEmptyPollAndExpiryCounts() {
        var now: TimeInterval = 10
        var callback: TimeInterval?
        let timing = PeerComparisonFirstPCM(start: {
            now = 11.75
        }, firstPCM: {
            let observed = callback
            callback = 11.875
            now = 12.005
            return observed
        }, uptime: { now }, wait: { _ in
            Issue.record("The observation reached its deadline.")
        })
        #expect(timing.milliseconds == 1875)
    }

    @Test func publishedBlocksStayMonotonicAcrossCompletedTransfers() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let job = PeerComparisonCaptureJob(directory: directory)
        #expect(try job.verifiedBlockCount() == 0)

        let first = job.paths().stage
        try FileManager.default.createDirectory(at: first, withIntermediateDirectories: false)
        for name in ["manifest.json", ".block-pending.tmp", "block-invalid.part", "block-00000000.part"] {
            try Data([0]).write(to: first.appendingPathComponent(name))
        }
        #expect(try job.verifiedBlockCount() == 1)
        try Data([1]).write(to: first.appendingPathComponent("block-00000001.part"))
        #expect(try job.verifiedBlockCount() == 2)
        job.finishedTransfer(blocks: 2)
        #expect(try job.verifiedBlockCount() == 2)
        try FileManager.default.removeItem(at: first)

        let second = job.paths().stage
        try FileManager.default.createDirectory(at: second, withIntermediateDirectories: false)
        try Data([0]).write(to: second.appendingPathComponent("block-00000000.part"))
        #expect(try job.verifiedBlockCount() == 3)
    }

    @Test func onlyTheResumedPayloadCallCountsAsPending() {
        let state = PeerComparisonPayloadReceive()
        #expect(state.phase == .waiting)
        state.observe {
            #expect(state.phase == .pending)
        }
        #expect(state.phase == .returned)
    }
}
#endif
