import Foundation
import Testing

/// The same Swift Testing cases run inside a personally provisioned app so the
/// Data Protection Keychain is exercised instead of skipped by an unsigned host.
@main
struct NativeTransportTestRunner {
    static func main() async {
        let status: CInt = await Testing.__swiftPMEntryPoint()
        exit(status)
    }
}
