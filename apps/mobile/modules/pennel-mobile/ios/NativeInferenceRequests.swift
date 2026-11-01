import Foundation

@MainActor
final class NativeInferenceRequests {
    struct Ticket {
        let id: String
        let generation: UUID
        let fingerprint: String
        let cancellation: InferenceCancellation
    }

    private var active: [String: Ticket] = [:]
    private var mutationDepth = 0
    private let engine: PennelEngine?

    init(engine: PennelEngine? = nil) { self.engine = engine }

    nonisolated static func changesCloudIdentity(_ method: String) -> Bool {
        switch method {
        case "cloud.loginStart", "cloud.loginPoll", "cloud.loginCancel", "cloud.logout", "cloud.deleteAccount",
             "cloud.linkStart", "cloud.linkJoin", "cloud.linkPoll", "cloud.linkApprove", "cloud.linkCancel", "cloud.revoke": true
        default: false
        }
    }

    func begin(id: String, fingerprint: String) throws -> Ticket {
        guard !id.isEmpty, id.utf8.count <= 96,
              id.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [45, 46, 95].contains($0) }) else {
            throw PennelBridgeError("invalid_request", "Use a short, unique request identifier.")
        }
        guard mutationDepth == 0 else { throw PennelBridgeError("operation_busy", "Wait for provider changes to finish.") }
        guard active[id] == nil else { throw PennelBridgeError("operation_busy", "This request is already running.") }
        guard active.count < 2 else { throw PennelBridgeError("operation_busy", "Wait for another model request to finish.") }
        let ticket = Ticket(id: id, generation: UUID(), fingerprint: fingerprint, cancellation: InferenceCancellation())
        active[id] = ticket
        return ticket
    }

    func availability(fingerprint: String, currentFingerprint: @MainActor () throws -> String,
                      work: @escaping @Sendable () throws -> String) async throws -> String {
        guard mutationDepth == 0 else { throw PennelBridgeError("operation_busy", "Wait for provider changes to finish.") }
        let result = try await Task.detached(priority: .userInitiated) { try work() }.value
        guard mutationDepth == 0, fingerprint == (try currentFingerprint()) else { throw PennelError.Cancelled }
        return result
    }

    func cancel(id: String) -> Bool {
        guard let ticket = active[id] else { return false }
        ticket.cancellation.cancel()
        return true
    }

    func cancelAll() { for ticket in active.values { ticket.cancellation.cancel() } }

    func changingCredentials<T>(_ mutation: () throws -> T) rethrows -> T {
        mutationDepth += 1
        let coreMutation = engine?.beginInferenceMutation()
        defer { coreMutation?.release(); mutationDepth -= 1 }
        cancelAll()
        return try mutation()
    }

    func changingConfiguration<T>(_ mutation: @MainActor () async throws -> T) async rethrows -> T {
        mutationDepth += 1
        let coreMutation = engine?.beginInferenceMutation()
        defer { coreMutation?.release(); mutationDepth -= 1 }
        cancelAll()
        return try await mutation()
    }

    func finish(_ ticket: Ticket, currentFingerprint: String) -> Bool {
        guard active[ticket.id]?.generation == ticket.generation else { return false }
        active[ticket.id] = nil
        return !ticket.cancellation.isCancelled() && ticket.fingerprint == currentFingerprint
    }
}
