import Foundation
import Testing
import Darwin
@testable import PennelMobileKit

struct KeyboardWritingTests {
    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-writing-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func request(id: UUID = UUID(), at: Date = Date()) -> KeyboardWritingRequest {
        KeyboardWritingRequest(id: id, at: at, action: "cleanup", text: "Synthetic text", configuration: "fixture-provider", authorization: "fixture-generation")
    }

    @Test func explicit_actions_are_bounded_and_expire() {
        let now = Date()
        #expect(request(at: now).isValid(at: now))
        #expect(!request(at: now.addingTimeInterval(-61)).isValid(at: now))
        var invalid = request(at: now)
        invalid.action = "execute"
        #expect(!invalid.isValid(at: now))
        invalid.action = "cleanup"
        invalid.text = String(repeating: "ë", count: 4_097)
        #expect(!invalid.isValid(at: now))
        invalid.text = " \n"
        #expect(!invalid.isValid(at: now))
        #expect(!KeyboardWritingAvailability(at: now.addingTimeInterval(-6), available: true, configuration: "fixture").isFresh(at: now))
    }

    @Test func cancellation_of_old_request_preserves_new_request_and_all_recent_ids() throws {
        let url = try directory()
        defer { try? FileManager.default.removeItem(at: url) }
        let app = KeyboardWritingStore(directory: url)
        let keyboard = KeyboardWritingStore(directory: url)
        let old = request(), next = request()
        try keyboard.write(next, as: .request)
        app.cancel(id: old.id)
        #expect(app.isCancelled(id: old.id))
        #expect(app.read(KeyboardWritingRequest.self, as: .request)?.id == next.id)
        keyboard.cancel(id: next.id)
        #expect(app.isCancelled(id: old.id))
        #expect(app.isCancelled(id: next.id))
        #expect(app.read(KeyboardWritingRequest.self, as: .request) == nil)
    }

    @Test func claim_is_once_only_and_expired_content_is_removed() throws {
        let url = try directory()
        defer { try? FileManager.default.removeItem(at: url) }
        let store = KeyboardWritingStore(directory: url)
        let current = request()
        try store.write(current, as: .request)
        #expect(store.takeRequest(at: Date())?.id == current.id)
        #expect(store.takeRequest(at: Date()) == nil)
        let old = request(at: Date().addingTimeInterval(-61))
        try store.write(old, as: .request)
        try store.write(KeyboardWritingResult(id: old.id, at: old.at, text: "old result"), as: .result)
        store.purge(at: Date())
        #expect(store.read(KeyboardWritingRequest.self, as: .request) == nil)
        #expect(store.read(KeyboardWritingResult.self, as: .result) == nil)
    }

    @Test func obsolete_cancellations_never_remove_concurrently_published_new_work() throws {
        let url = try directory()
        defer { try? FileManager.default.removeItem(at: url) }
        let original = request(), newest = request()
        let first = KeyboardWritingStore(directory: url), second = KeyboardWritingStore(directory: url)
        try first.write(newest, as: .request)
        DispatchQueue.concurrentPerform(iterations: 40) { index in
            if index.isMultiple(of: 2) { try? first.write(newest, as: .request) }
            else { second.cancel(id: original.id) }
        }
        #expect(second.takeRequest(at: Date())?.id == newest.id)
    }

    @Test func oversized_and_malformed_shared_files_are_not_decoded() throws {
        let url = try directory()
        defer { try? FileManager.default.removeItem(at: url) }
        let store = KeyboardWritingStore(directory: url)
        try Data(repeating: 0x20, count: 65_537).write(to: url.appendingPathComponent(KeyboardWritingStore.File.request.rawValue))
        #expect(store.read(KeyboardWritingRequest.self, as: .request) == nil)
        store.purge(at: Date())
        #expect(!FileManager.default.fileExists(atPath: url.appendingPathComponent(KeyboardWritingStore.File.request.rawValue).path))
    }

    @Test func suspended_lock_owner_cannot_block_keyboard_reads_or_cancellation() throws {
        let url = try directory()
        defer { try? FileManager.default.removeItem(at: url) }
        let store = KeyboardWritingStore(directory: url)
        let value = request()
        try store.write(KeyboardWritingAvailability(at: Date(), available: true, configuration: "fixture"), as: .availability)
        let held = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0), finished = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            let descriptor = open(url.appendingPathComponent(".writing.lock").path, O_RDWR)
            guard descriptor >= 0 else { held.signal(); finished.signal(); return }
            flock(descriptor, LOCK_EX)
            held.signal()
            _ = release.wait(timeout: .now() + 2)
            flock(descriptor, LOCK_UN)
            close(descriptor)
            finished.signal()
        }
        #expect(held.wait(timeout: .now() + 2) == .success)
        defer { release.signal(); _ = finished.wait(timeout: .now() + 3) }
        #expect(store.read(KeyboardWritingAvailability.self, as: .availability)?.available == true)
        store.cancel(id: value.id)
        #expect(store.isCancelled(id: value.id))
        #expect(throws: (any Error).self) { try store.write(value, as: .request) }
    }
}
