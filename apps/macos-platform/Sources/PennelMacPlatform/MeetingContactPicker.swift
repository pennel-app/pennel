import AppKit
import Contacts
import ContactsUI
import Foundation

struct MeetingContactSnapshot: Sendable {
    let sourceId: String
    let sourceRevision: String
    let displayName: String
    let email: String?
    let selectedAtMs: UInt64

    init(contact: CNContact, selectedAtMs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000)) {
        sourceId = contact.identifier
        let descriptor = CNContactFormatter.descriptorForRequiredKeys(for: .fullName)
        displayName = contact.areKeysAvailable([descriptor])
            ? CNContactFormatter.string(from: contact, style: .fullName) ?? "" : ""
        email = contact.isKeyAvailable(CNContactEmailAddressesKey)
            ? contact.emailAddresses.first.map { String($0.value) } : nil
        self.selectedAtMs = selectedAtMs
        sourceRevision = peopleSourceRevision([
            "source_id": sourceId, "display_name": displayName, "email": email as Any? ?? NSNull(),
        ])
    }

    var json: [String: Any] {
        ["source": "macos_contacts", "source_id": sourceId, "source_revision": sourceRevision,
         "display_name": displayName, "email": email as Any? ?? NSNull(), "selected_at_ms": selectedAtMs]
    }
}

@MainActor
private enum MeetingContactPickerState {
    static var active: MeetingContactPickerSession?
}

@MainActor
final class MeetingContactPickerSession: NSObject, @preconcurrency CNContactPickerDelegate {
    let picker: CNContactPicker
    private weak var owner: NSWindow?
    private weak var previousResponder: NSResponder?
    private var completion: ((MeetingContactSnapshot?) -> Void)?
    private var observers: [NSObjectProtocol] = []

    init(picker: CNContactPicker, owner: NSWindow, completion: @escaping (MeetingContactSnapshot?) -> Void) {
        self.picker = picker
        self.owner = owner
        previousResponder = owner.firstResponder
        self.completion = completion
        super.init()
        // Selected-contact snapshots are sufficient. Never enumerate a CNContactStore here.
        picker.displayedKeys = []
        picker.delegate = self
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: NSWindow.willCloseNotification, object: owner, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(nil) }
        })
        observers.append(center.addObserver(forName: NSWindow.didChangeOcclusionStateNotification, object: owner, queue: .main) { [weak self, weak owner] _ in
            MainActor.assumeIsolated {
                if owner?.isVisible != true { self?.finish(nil) }
            }
        })
    }

    func contactPicker(_ picker: CNContactPicker, didSelect contact: CNContact) {
        finish(owner == nil ? nil : MeetingContactSnapshot(contact: contact))
    }

    func contactPickerDidClose(_ picker: CNContactPicker) { finish(nil) }

    private func finish(_ snapshot: MeetingContactSnapshot?) {
        guard let completion else { return }
        self.completion = nil
        picker.delegate = nil
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers.removeAll()
        picker.close()
        if let owner, let view = previousResponder as? NSView, view.window === owner { owner.makeFirstResponder(view) }
        if MeetingContactPickerState.active === self { MeetingContactPickerState.active = nil }
        completion(snapshot)
    }
}

@MainActor
func pickMeetingContact(
    owner: NSWindow,
    present: ((CNContactPicker, NSWindow, MeetingContactPickerSession) -> Void)? = nil
) async -> MeetingContactSnapshot? {
    guard MeetingContactPickerState.active == nil, owner.attachedSheet == nil,
          let view = owner.contentView else { return nil }
    return await withCheckedContinuation { (continuation: CheckedContinuation<MeetingContactSnapshot?, Never>) in
        let picker = CNContactPicker()
        let session = MeetingContactPickerSession(picker: picker, owner: owner) { continuation.resume(returning: $0) }
        MeetingContactPickerState.active = session
        if let present { present(picker, owner, session) }
        else {
            picker.showRelative(to: NSRect(x: view.bounds.midX, y: view.bounds.midY, width: 1, height: 1), of: view, preferredEdge: .minY)
        }
    }
}
