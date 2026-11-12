import Foundation
import Security

let args = CommandLine.arguments
guard args.count >= 3, args[1] == "set" || args[1] == "delete", args[2].hasPrefix("review96-owned-") else {
    fatalError("Owned capture account required")
}
let service = "dev.pennel.Pennel.capture.preview"
let query: [CFString: Any] = [kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: args[2]]
if args[1] == "delete" {
    let status = SecItemDelete(query as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else { fatalError("Capture credential cleanup failed") }
    print("owned_capture_credential_deleted")
} else {
    guard args.count == 4, args[3].hasSuffix("Pennel Capture Preview.app") else { fatalError("Capture application required") }
    let secret = FileHandle.standardInput.readDataToEndOfFile()
    guard !secret.isEmpty else { fatalError("Credential input required") }
    var trusted: SecTrustedApplication?
    guard SecTrustedApplicationCreateFromPath(args[3], &trusted) == errSecSuccess, let trusted else { fatalError("Capture identity unavailable") }
    var access: SecAccess?
    guard SecAccessCreate("Owned Review QA" as CFString, [trusted] as CFArray, &access) == errSecSuccess, let access else { fatalError("Capture access unavailable") }
    var value = query
    value[kSecValueData] = secret
    value[kSecAttrAccess] = access
    let prior = SecItemCopyMatching(query as CFDictionary, nil)
    guard prior == errSecItemNotFound else { fatalError("Refusing to replace an existing credential") }
    guard SecItemAdd(value as CFDictionary, nil) == errSecSuccess else { fatalError("Capture credential creation failed") }
    print("owned_capture_credential_created")
}
