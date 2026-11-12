import Foundation
import Security

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}
let args = CommandLine.arguments
SecKeychainSetUserInteractionAllowed(false)
guard args.count >= 3, args[1] == "set" || args[1] == "delete", args[2].hasPrefix("review96-owned-") else {
    fail("Owned capture account required")
}
let service = "dev.pennel.Pennel.capture.preview"
let query: [CFString: Any] = [kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: args[2]]
if args[1] == "delete" {
    let status = SecItemDelete(query as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else { fail("Capture credential cleanup failed") }
    print("owned_capture_credential_deleted")
} else {
    guard args.count == 4, args[3].hasSuffix("Pennel Capture Preview.app") else { fail("Capture application required") }
    let secret = FileHandle.standardInput.readDataToEndOfFile()
    guard !secret.isEmpty else { fail("Credential input required") }
    var trusted: SecTrustedApplication?
    var owner: SecTrustedApplication?
    let executable = args[3] + "/Contents/MacOS/pennel"
    guard SecTrustedApplicationCreateFromPath(executable, &trusted) == errSecSuccess, let trusted,
          SecTrustedApplicationCreateFromPath(args[0], &owner) == errSecSuccess, let owner else { fail("Owned application identities unavailable") }
    var access: SecAccess?
    guard SecAccessCreate("Owned Review QA" as CFString, [trusted, owner] as CFArray, &access) == errSecSuccess, let access else { fail("Capture access unavailable") }
    var value = query
    value[kSecValueData] = secret
    value[kSecAttrAccess] = access
    let prior = SecItemCopyMatching(query as CFDictionary, nil)
    guard prior == errSecItemNotFound else { fail("Refusing to replace an existing credential") }
    guard SecItemAdd(value as CFDictionary, nil) == errSecSuccess else { fail("Capture credential creation failed") }
    print("owned_capture_credential_created")
}
