import Foundation
import Testing
import PennelMobileKit
@testable internal import PennelMobile

@MainActor
struct KeyboardWritingServerTests {
    @Test func foreground_writing_does_not_require_a_microphone_session() {
        #expect(KeyboardWritingServer.canReceive(appIsActive: true, session: nil, now: Date()))
    }

    @Test func background_requests_require_an_actual_fresh_ready_session() {
        let now = Date()
        #expect(!KeyboardWritingServer.canReceive(appIsActive: false, session: nil, now: now))
        for phase in [SessionPhase.off, .listening, .processing] {
            let state = SessionState(phase: phase, expiresAt: nil, heartbeatAt: now, level: 0)
            #expect(!KeyboardWritingServer.canReceive(appIsActive: false, session: state, now: now))
        }
        var ready = SessionState(phase: .ready, expiresAt: now.addingTimeInterval(60), heartbeatAt: now, level: 0)
        #expect(KeyboardWritingServer.canReceive(appIsActive: false, session: ready, now: now))
        ready.heartbeatAt = now.addingTimeInterval(-6)
        #expect(!KeyboardWritingServer.canReceive(appIsActive: false, session: ready, now: now))
        ready.heartbeatAt = now.addingTimeInterval(20)
        #expect(!KeyboardWritingServer.canReceive(appIsActive: false, session: ready, now: now))
        ready.heartbeatAt = now
        ready.expiresAt = now.addingTimeInterval(-1)
        #expect(!KeyboardWritingServer.canReceive(appIsActive: false, session: ready, now: now))
        ready.expiresAt = nil
        #expect(!KeyboardWritingServer.canReceive(appIsActive: false, session: ready, now: now))
    }
}
