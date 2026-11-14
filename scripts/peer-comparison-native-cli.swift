import Foundation
import CryptoKit
import CoreFoundation
import Darwin

private enum CLIError: Error { case invalidArguments, invalidCommand, invalidIdentity }

@main
struct PeerComparisonCLI {
    static func main() {
        DispatchQueue.global(qos: .userInitiated).async {
            do { try run(); exit(0) }
            catch { emit(["ok": false, "error": "qa_probe_failed"]); exit(1) }
        }
        dispatchMain()
    }

    private static func run() throws {
        var options: [String: String] = [:]
        var arguments = Array(CommandLine.arguments.dropFirst())
        while !arguments.isEmpty {
            let name = arguments.removeFirst()
            guard ["--candidate", "--identity-file", "--run-id", "--relay-config", "--bind", "--advertise-host"].contains(name),
                  !arguments.isEmpty, options[name] == nil else { throw CLIError.invalidArguments }
            options[name] = arguments.removeFirst()
        }
        guard let file = options["--identity-file"], let run = options["--run-id"],
              ["quinn", "native"].contains(options["--candidate"] ?? "") else { throw CLIError.invalidArguments }
        let candidate = options["--candidate"] ?? "native"
        let account = try identityAccount(file: URL(fileURLWithPath: file), run: run, candidate: candidate)
        let store = PeerComparisonIdentityKeychain()
        let identity = try PeerComparisonFactory.identity(store: store, account: account)
        let relay = try options["--relay-config"].map { name -> String in
            let data = try Data(contentsOf: URL(fileURLWithPath: name))
            guard data.count <= 32 * 1024, let text = String(data: data, encoding: .utf8) else { throw CLIError.invalidArguments }
            return text
        }
        let bind = options["--bind"] ?? "127.0.0.1:0"
        let probe: PeerComparisonProbe
        if candidate == "quinn" {
            probe = try PeerComparisonProbe.newQuinn(identityStore: store, identityAccount: account, bindAddress: bind, relayConfigJson: relay)
        } else {
            let pieces = bind.split(separator: ":", omittingEmptySubsequences: false)
            guard pieces.count == 2, pieces[1] == "0" else { throw CLIError.invalidArguments }
            probe = try PeerComparisonFactory.native(identity: identity, account: account,
                bindHost: String(pieces[0]), advertisedHost: options["--advertise-host"] ?? String(pieces[0]), relayJSON: relay)
        }
        defer { probe.close() }
        let route = relay == nil ? "direct" : "forced_relay"
        emit(["ok": true, "event": "ready", "result": ["identity_id": probe.identityId(),
            "address_json": try probe.addressJson(), "candidate": candidate, "route": route]])
        while let data = try autoreleasepool(invoking: readCommand) {
            let shouldClose = autoreleasepool { () -> Bool in
            do {
                guard let request = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let command = request["command"] as? String else { throw CLIError.invalidCommand }
                func string(_ name: String) throws -> String {
                    guard let result = request[name] as? String else { throw CLIError.invalidCommand }
                    return result
                }
                let result: Any
                switch command {
                case "identity": result = probe.identityId()
                case "address": result = try probe.addressJson()
                case "approve": result = try probe.approvePeer(id: string("peer_id"))
                case "revoke": result = try probe.revokePeer(id: string("peer_id"))
                case "offer":
                    guard let value = request["block_size"] else { throw CLIError.invalidCommand }
                    let size = try unsigned(value)
                    let manifest = try probe.offerFile(path: string("path"), blockSize: size)
                    result = try JSONSerialization.jsonObject(with: Data(manifest.utf8))
                case "receive":
                    let maximum: UInt32?
                    if let value = request["max_blocks"], !(value is NSNull) { maximum = try unsigned(value) }
                    else { maximum = nil }
                    let report = try probe.receiveFile(addressJson: string("address_json"), staging: string("staging"),
                        target: string("target"), maxBlocks: maximum)
                    result = ["complete": report.complete, "bytes_sent": report.bytesSent,
                        "bytes_received": report.bytesReceived, "blocks_transferred": report.blocksTransferred,
                        "total_bytes": report.totalBytes, "sha256": report.sha256]
                case "cancel": probe.cancel(); result = true
                case "close": probe.close(); emit(["ok": true, "result": true]); return true
                default: throw CLIError.invalidCommand
                }
                emit(["ok": true, "result": result])
                return false
            } catch {
                emit(["ok": false, "error": code(error)])
                return false
            }
            }
            if shouldClose { return }
        }
    }

    private static func identityAccount(file: URL, run: String, candidate: String) throws -> String {
        if FileManager.default.fileExists(atPath: file.path) {
            let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
            guard attributes[.type] as? FileAttributeType == .typeRegular,
                  (attributes[.posixPermissions] as? NSNumber)?.intValue == 0o600 else { throw CLIError.invalidIdentity }
            let data = try Data(contentsOf: file)
            guard data.count <= 4096, let object = try JSONSerialization.jsonObject(with: data) as? [String: String],
                  let account = object["keychain_account"] else { throw CLIError.invalidIdentity }
            return account
        }
        let digest = SHA256.hash(data: Data("\(file.standardizedFileURL.path):\(run):\(candidate)".utf8))
            .map { String(format: "%02x", $0) }.joined()
        let account = "peer-probe-\(candidate)-\(digest.prefix(32))"
        let metadata = try JSONSerialization.data(withJSONObject: ["keychain_account": account], options: [.sortedKeys])
        let descriptor = open(file.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw CLIError.invalidIdentity }
        defer { close(descriptor) }
        try metadata.withUnsafeBytes { buffer in
            var offset = 0
            while offset < buffer.count {
                let count = Darwin.write(descriptor, buffer.baseAddress!.advanced(by: offset), buffer.count - offset)
                if count < 0 && errno == EINTR { continue }
                guard count > 0 else { throw CLIError.invalidIdentity }
                offset += count
            }
        }
        guard fsync(descriptor) == 0 else { throw CLIError.invalidIdentity }
        return account
    }

    private static func readCommand() throws -> Data? {
        var bytes = Data()
        while let byte = try FileHandle.standardInput.read(upToCount: 1), !byte.isEmpty {
            guard bytes.count < 16 * 1024 else { throw CLIError.invalidCommand }
            if byte[0] == 10 { return bytes }
            bytes.append(byte)
        }
        if bytes.isEmpty { return nil }
        throw CLIError.invalidCommand
    }

    private static func unsigned(_ value: Any) throws -> UInt32 {
        guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else { throw CLIError.invalidCommand }
        let value = number.doubleValue
        guard value.isFinite, value >= 0, value <= Double(UInt32.max), value.rounded(.towardZero) == value else {
            throw CLIError.invalidCommand
        }
        return UInt32(value)
    }

    private static func emit(_ value: [String: Any]) {
        guard let bytes = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) else { return }
        FileHandle.standardOutput.write(bytes)
        FileHandle.standardOutput.write(Data([10]))
    }

    private static func code(_ error: Error) -> String {
        guard let value = error as? PeerCompareError else { return "invalid_command" }
        switch value {
        case .Unauthorized: return "unauthorized"
        case .Cancelled: return "cancelled"
        case .Closed: return "closed"
        case .Storage: return "storage"
        case .InvalidBlock: return "invalid_block"
        case .InvalidManifest: return "invalid_manifest"
        case .SourceChanged: return "source_changed"
        case .LimitExceeded: return "limit_exceeded"
        case .InvalidConfig: return "invalid_config"
        case .InvalidProtocol: return "invalid_protocol"
        case .Transport: return "transport"
        }
    }
}
