import Foundation
import PennelMobileKit

enum KeyboardWritingFailure: String, Error, LocalizedError {
    case unavailable, expired, cancelled, failed, tooLong
    var errorDescription: String? { rawValue }
}

@MainActor
final class KeyboardWritingClient {
    private let store: KeyboardWritingStore
    private var pending: KeyboardWritingRequest?
    private var completion: (@MainActor (Result<String, Error>) -> Void)?

    init(directory: URL) { store = KeyboardWritingStore(directory: directory) }

    var isAvailable: Bool {
        let now = Date()
        guard let value = store.read(KeyboardWritingAvailability.self, as: .availability),
              value.available, value.isFresh(at: now) else { return false }
        return true
    }

    var providerName: String? { store.read(KeyboardWritingAvailability.self, as: .availability)?.providerName }

    func request(action: KeyboardWritingAction, text: String, completion: @escaping @MainActor (Result<String, Error>) -> Void) {
        cancel()
        guard text.count <= 4_096 else { completion(.failure(KeyboardWritingFailure.tooLong)); return }
        guard isAvailable, let availability = store.read(KeyboardWritingAvailability.self, as: .availability) else {
            completion(.failure(KeyboardWritingFailure.unavailable)); return
        }
        let request = KeyboardWritingRequest(id: UUID(), at: Date(), action: action.rawValue, text: text, configuration: availability.configuration, authorization: availability.authorization)
        guard request.isValid(at: Date()) else { completion(.failure(KeyboardWritingFailure.failed)); return }
        do {
            try store.write(request, as: .request)
            pending = request
            self.completion = completion
        } catch { completion(.failure(KeyboardWritingFailure.failed)) }
    }

    func refresh() {
        guard let pending else { return }
        if Date().timeIntervalSince(pending.at) > 35 { finish(.failure(KeyboardWritingFailure.expired)); return }
        guard let result = store.read(KeyboardWritingResult.self, as: .result), result.id == pending.id else { return }
        store.clearResult(id: pending.id)
        guard (-2...60).contains(Date().timeIntervalSince(result.at)), !store.isCancelled(id: pending.id) else {
            finish(.failure(KeyboardWritingFailure.expired)); return
        }
        if let text = result.text, !text.isEmpty, text.count <= 4_096 { finish(.success(text)) }
        else { finish(.failure(KeyboardWritingFailure.failed)) }
    }

    func cancel() {
        if let pending { store.cancel(id: pending.id) }
        pending = nil
        completion = nil
    }

    private func finish(_ result: Result<String, Error>) {
        let callback = completion
        if let pending { store.cancel(id: pending.id) }
        pending = nil
        completion = nil
        callback?(result)
    }
}
