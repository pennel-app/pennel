import CoreImage.CIFilterBuiltins
import Foundation
import UIKit

enum NativeCloudViews {
    static func availability(_ value: CloudLinkAvailability) -> [String: Any] {
        ["available": value.available, "reason_code": BridgeJSON.optional(value.reasonCode)]
    }

    static func setup(_ value: CloudSyncSetup) -> [String: Any] {
        let phase: String = switch value.phase {
        case .signedOut: "signed_out"
        case .newLibrary: "new_library"
        case .locked: "locked"
        case .ready: "ready"
        case .paused: "paused"
        case .active: "active"
        }
        return ["phase": phase, "sync_enabled": value.syncEnabled,
                "generation": BridgeJSON.optional(value.generation), "can_approve_devices": value.canApproveDevices,
                "uploaded": value.uploaded, "downloaded": value.downloaded, "conflicts": value.conflicts,
                "last_success_ms": BridgeJSON.optional(value.lastSuccessMs)]
    }

    static func link(_ value: CloudLinkView) -> [String: Any] {
        let phase: String = switch value.phase {
        case .idle: "idle"
        case .waitingPeer: "waiting_peer"
        case .confirming: "confirming"
        case .awaitingApproval: "awaiting_approval"
        case .waitingApproval: "waiting_approval"
        case .provisioning: "provisioning"
        case .complete: "complete"
        case .cancelled: "cancelled"
        case .expired: "expired"
        case .failed: "failed"
        }
        let role = value.role.map { $0 == .newDevice ? "new_device" : "trusted_device" }
        return ["phase": phase, "role": BridgeJSON.optional(role), "request_id": BridgeJSON.optional(value.requestId),
                "connection_code": BridgeJSON.optional(value.connectionCode), "qr_payload": BridgeJSON.optional(value.qrPayload),
                "qr_svg": BridgeJSON.optional(value.qrSvg), "qr_image": BridgeJSON.optional(qrImage(value.qrPayload)),
                "verification_code": BridgeJSON.optional(value.verificationCode), "peer_name": BridgeJSON.optional(value.peerName),
                "peer_platform": BridgeJSON.optional(value.peerPlatform), "account_email": BridgeJSON.optional(value.accountEmail),
                "expires_at_ms": BridgeJSON.optional(value.expiresAtMs), "error_code": BridgeJSON.optional(value.errorCode),
                "error_message": BridgeJSON.optional(value.errorMessage)]
    }

    static func role(_ input: String) throws -> CloudLinkRole {
        switch input {
        case "new_device": .newDevice
        case "trusted_device": .trustedDevice
        default: throw PennelBridgeError("invalid_request", "Unknown device connection role.")
        }
    }

    private static func qrImage(_ payload: String?) -> String? {
        guard let payload, payload.utf8.count <= 4_096 else { return nil }
        let generator = CIFilter.qrCodeGenerator()
        generator.message = Data(payload.utf8)
        generator.correctionLevel = "M"
        guard let base = generator.outputImage else { return nil }
        let bounds = base.extent.insetBy(dx: -4, dy: -4)
        let quietZone = CIImage(color: CIColor(red: 1, green: 1, blue: 1, alpha: 1)).cropped(to: bounds)
        let padded = base.composited(over: quietZone).cropped(to: bounds)
        let output = padded.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        guard
              let bitmap = CIContext().createCGImage(output, from: output.extent),
              let png = UIImage(cgImage: bitmap).pngData() else { return nil }
        return "data:image/png;base64,\(png.base64EncodedString())"
    }
}
