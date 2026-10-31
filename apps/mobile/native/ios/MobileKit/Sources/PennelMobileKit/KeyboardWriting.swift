import Foundation
#if canImport(Darwin)
import Darwin
#else
import Glibc
#endif

public struct KeyboardWritingAvailability: Codable, Equatable, Sendable {
    public var at: Date
    public var available: Bool
    public var configuration: String
    public var providerName: String?
    public var authorization: String

    public init(at: Date, available: Bool, configuration: String, providerName: String? = nil, authorization: String = "") {
        self.at = at
        self.available = available
        self.configuration = configuration
        self.providerName = providerName
        self.authorization = authorization
    }

    public func isFresh(at now: Date) -> Bool {
        (-2...5).contains(now.timeIntervalSince(at))
    }
}

public struct KeyboardWritingRequest: Codable, Equatable, Sendable {
    public var id: UUID
    public var at: Date
    public var action: String
    public var text: String
    public var configuration: String
    public var authorization: String

    public init(id: UUID, at: Date, action: String, text: String, configuration: String, authorization: String = "") {
        self.id = id
        self.at = at
        self.action = action
        self.text = text
        self.configuration = configuration
        self.authorization = authorization
    }

    public func isValid(at now: Date) -> Bool {
        (-2...60).contains(now.timeIntervalSince(at)) && !configuration.isEmpty
            && ["cleanup", "shorten", "translateToEnglish", "translateToDutch"].contains(action)
            && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && text.count <= 4_096
    }
}

public struct KeyboardWritingResult: Codable, Equatable, Sendable {
    public var id: UUID
    public var at: Date
    public var text: String?
    public var errorCode: String?

    public init(id: UUID, at: Date, text: String? = nil, errorCode: String? = nil) {
        self.id = id
        self.at = at
        self.text = text
        self.errorCode = errorCode
    }
}

public final class KeyboardWritingStore: @unchecked Sendable {
    public enum File: String, CaseIterable, Sendable {
        case availability = "writing-availability.json"
        case request = "writing-request.json"
        case result = "writing-result.json"
        case cancellation = "writing-cancel.json"
    }

    private struct Cancellation: Codable { let id: UUID; let at: Date }
    public let directory: URL
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private let localLock = NSRecursiveLock()
    private var lockDepth = 0

    public init(directory: URL) { self.directory = directory }

    public func write<T: Encodable>(_ value: T, as file: File) throws {
        try locked {
            if file == .request {
                let count = (try? FileManager.default.contentsOfDirectory(atPath: directory.path))?
                    .filter { $0.hasPrefix("writing-cancel-") && $0.hasSuffix(".json") }.count ?? 0
                guard count < 128 else { throw CocoaError(.fileWriteOutOfSpace) }
            }
            let data = try encoder.encode(value)
            guard data.count <= 65_536 else { throw CocoaError(.fileWriteOutOfSpace) }
            try data.write(to: directory.appendingPathComponent(file.rawValue), options: .atomic)
        }
    }

    public func read<T: Decodable>(_ type: T.Type, as file: File) -> T? {
        localLock.lock()
        defer { localLock.unlock() }
        return decode(type, at: directory.appendingPathComponent(file.rawValue))
    }

    private func decode<T: Decodable>(_ type: T.Type, at url: URL) -> T? {
            guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
                  let size = attributes[.size] as? NSNumber, size.intValue <= 65_536,
                  let data = try? Data(contentsOf: url), data.count <= 65_536 else { return nil }
            return try? decoder.decode(type, from: data)
    }

    public func takeRequest(at now: Date) -> KeyboardWritingRequest? {
        try? locked {
            guard let request = read(KeyboardWritingRequest.self, as: .request) else { remove(.request); return nil }
            remove(.request)
            return request.isValid(at: now) && !isCancelled(id: request.id) ? request : nil
        }
    }

    public func cancel(id: UUID, at now: Date = Date()) {
        // An independent, atomic per-ID marker can cancel work even if the app is suspended holding its lock.
        localLock.lock()
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try encoder.encode(Cancellation(id: id, at: now)).write(to: cancellationURL(id), options: .atomic)
        } catch { }
        localLock.unlock()
        try? locked {
            if read(KeyboardWritingRequest.self, as: .request)?.id == id { remove(.request) }
            if read(KeyboardWritingResult.self, as: .result)?.id == id { remove(.result) }
        }
    }

    public func isCancelled(id: UUID) -> Bool {
        localLock.lock()
        defer { localLock.unlock() }
        return decode(Cancellation.self, at: cancellationURL(id))?.id == id
    }

    public func clearResult(id: UUID) {
        try? locked { if read(KeyboardWritingResult.self, as: .result)?.id == id { remove(.result) } }
    }

    public func purge(at now: Date) {
        try? locked { purgeLocked(at: now) }
    }

    private func purgeLocked(at now: Date) {
        if let request = read(KeyboardWritingRequest.self, as: .request) {
            if !request.isValid(at: now) { remove(.request) }
        } else { remove(.request) }
        if let result = read(KeyboardWritingResult.self, as: .result) {
            if !(-2...60).contains(now.timeIntervalSince(result.at)) { remove(.result) }
        } else { remove(.result) }
        for url in (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
            where url.lastPathComponent.hasPrefix("writing-cancel-") && url.pathExtension == "json" {
            if let cancellation = decode(Cancellation.self, at: url), (-2...60).contains(now.timeIntervalSince(cancellation.at)) { continue }
            try? FileManager.default.removeItem(at: url)
        }
    }

    // The app and extension both participate: read/modify/remove must not delete a newer request.
    private func locked<T>(_ body: () throws -> T) throws -> T {
        localLock.lock()
        defer { localLock.unlock() }
        if lockDepth > 0 { return try body() }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let descriptor = open(directory.appendingPathComponent(".writing.lock").path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw CocoaError(.fileReadNoPermission) }
        defer { close(descriptor) }
        guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { throw CocoaError(.fileReadUnknown) }
        lockDepth += 1
        defer { lockDepth -= 1; flock(descriptor, LOCK_UN) }
        return try body()
    }

    private func cancellationURL(_ id: UUID) -> URL { directory.appendingPathComponent("writing-cancel-\(id.uuidString).json") }

    private func remove(_ file: File) { try? FileManager.default.removeItem(at: directory.appendingPathComponent(file.rawValue)) }
}
