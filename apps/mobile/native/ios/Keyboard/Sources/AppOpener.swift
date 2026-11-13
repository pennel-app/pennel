import UIKit

@MainActor
enum AppOpener {
    static func openSettings(from context: NSExtensionContext?) {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        context?.open(url, completionHandler: nil)
    }

    /// Opens Pennel's own URL from the keyboard. iOS has no public keyboard API for this; the window scene and
    /// application in the responder chain accept it only while Full Access is on, which dictation already needs.
    static func openContainingApp(_ url: URL, from controller: UIInputViewController, completion: @escaping (Bool) -> Void) {
        let selector = NSSelectorFromString("openURL:options:completionHandler:")
        // UIKit may answer on any queue; the result is only ever delivered on the main actor.
        nonisolated(unsafe) let deliver = completion
        let finish: @convention(block) (Bool) -> Void = { opened in
            DispatchQueue.main.async { MainActor.assumeIsolated { deliver(opened) } }
        }
        if let scene = controller.view.window?.windowScene, scene.responds(to: selector) {
            typealias SceneOpen = @convention(c) (AnyObject, Selector, NSURL, AnyObject?, (@convention(block) (Bool) -> Void)?) -> Void
            unsafeBitCast(scene.method(for: selector), to: SceneOpen.self)(scene, selector, url as NSURL, nil, finish)
            return
        }
        var responder: UIResponder? = controller
        while let current = responder {
            if current is UIApplication, current.responds(to: selector) {
                typealias ApplicationOpen = @convention(c) (AnyObject, Selector, NSURL, NSDictionary, (@convention(block) (Bool) -> Void)?) -> Void
                unsafeBitCast(current.method(for: selector), to: ApplicationOpen.self)(current, selector, url as NSURL, NSDictionary(), finish)
                return
            }
            responder = current.next
        }
        completion(false)
    }
}
