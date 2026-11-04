import Foundation
import Testing
@testable internal import PennelMobile

struct HistoryLimitTests {
    @Test func history_limits_preserve_requested_counts_and_reject_invalid_numbers() throws {
        #expect(try BridgePayload(values: [:]).positiveUInt32("limit", default: 200) == 200)
        for value: UInt32 in [5, 200, 400, UInt32.max] {
            #expect(try BridgePayload(values: ["limit": value]).positiveUInt32("limit", default: 200) == value)
        }
        for value: Any in [0, -1, 2.5, UInt64(UInt32.max) + 1, true, "5", NSNull(), Double.infinity] {
            #expect(throws: (any Error).self) {
                _ = try BridgePayload(values: ["limit": value]).positiveUInt32("limit", default: 200)
            }
        }
    }
}
