import AppKit
import Contacts
import ContactsUI
import XCTest
@testable import PennelMacPlatform

final class MeetingPeopleSourceTests: XCTestCase {
    func testCalendarSourceAvailabilityDoesNotSurviveRevocationDeletionOrAccountSwitch() {
        let expected = CalendarSourceScope(calendarId: "calendar", sourceId: "account-a")
        XCTAssertTrue(calendarSourceIsAvailable(hasAccess: true, expected: expected, current: expected))
        XCTAssertFalse(calendarSourceIsAvailable(hasAccess: false, expected: expected, current: expected))
        XCTAssertFalse(calendarSourceIsAvailable(hasAccess: true, expected: expected, current: nil))
        XCTAssertFalse(calendarSourceIsAvailable(hasAccess: true, expected: expected, current: CalendarSourceScope(calendarId: "other-calendar", sourceId: "account-a")))
        XCTAssertFalse(calendarSourceIsAvailable(hasAccess: true, expected: expected, current: CalendarSourceScope(calendarId: "calendar", sourceId: "account-b")))
        XCTAssertFalse(calendarSourceIsAvailable(hasAccess: true, expected: expected, current: CalendarSourceScope(calendarId: "calendar", sourceId: nil)))
        XCTAssertTrue(calendarSourceIsAvailable(hasAccess: true, expected: CalendarSourceScope(calendarId: "calendar", sourceId: nil), current: expected))
    }

    func testAttendeeSnapshotRetainsUnknownFieldsAndDoesNotInferEmailFromOtherURLs() {
        let participant = CalendarParticipantSnapshot(name: nil, url: URL(string: "https://example.test/people/guest"), response: nil, isSelf: true)
        XCTAssertNil(participant.email)
        XCTAssertNil(participant.name)
        XCTAssertNil(participant.response)
        XCTAssertEqual(participant.json["is_self"] as? Bool, true)
        XCTAssertTrue(participant.json["name"] is NSNull)
        XCTAssertTrue(participant.json["email"] is NSNull)
        XCTAssertEqual(CalendarParticipantSnapshot.email(from: URL(string: "mailto:els%2Bmeeting@example.test?subject=Hello")), "els+meeting@example.test")
    }

    func testRichCalendarContextRetainsSelfDeclinedAndDuplicateNames() {
        let people = [
            CalendarParticipantSnapshot(name: "Els", url: URL(string: "mailto:self@example.test"), response: "accepted", isSelf: true),
            CalendarParticipantSnapshot(name: "Tom", url: nil, response: "declined", isSelf: false),
            CalendarParticipantSnapshot(name: "Tom", url: nil, response: nil, isSelf: false),
            CalendarParticipantSnapshot(name: nil, url: nil, response: nil, isSelf: false),
        ]
        let context = CalendarContextSnapshot(sourceId: "source", calendarId: "calendar", eventId: "event", externalEventId: nil, instanceDate: Date(timeIntervalSince1970: 100), modifiedDate: nil, participants: people)
        let event = CurrentEvent(title: "Meeting", calendarId: "calendar", calendarName: "Calendar", attendees: people.filter { !$0.isSelf }.compactMap(\.name), calendarContext: context)
        XCTAssertEqual(event.attendees, ["Tom", "Tom"])
        XCTAssertEqual(context.participants.count, 4)
        XCTAssertEqual((event.json["calendar_context"] as? [String: Any])?["event_id"] as? String, "event")
        XCTAssertEqual(context.participants[1].response, "declined")
        XCTAssertEqual(context.json["instance_id"] as? String, "1970-01-01T00:01:40.000Z")
    }

    func testCalendarRevisionChangesWithSourceOrParticipantStatusWithoutGuessingIdentity() {
        func snapshot(source: String?, response: String?) -> CalendarContextSnapshot {
            CalendarContextSnapshot(sourceId: source, calendarId: "calendar", eventId: nil, externalEventId: nil, instanceDate: nil, modifiedDate: nil, participants: [CalendarParticipantSnapshot(name: "Guest", url: nil, response: response, isSelf: false)])
        }
        XCTAssertEqual(snapshot(source: nil, response: nil).revision, snapshot(source: nil, response: nil).revision)
        XCTAssertNotEqual(snapshot(source: "account-a", response: nil).revision, snapshot(source: "account-b", response: nil).revision)
        XCTAssertNotEqual(snapshot(source: "account-a", response: "accepted").revision, snapshot(source: "account-a", response: "declined").revision)
        XCTAssertTrue(snapshot(source: nil, response: nil).json["event_id"] is NSNull)
    }

    func testChosenContactSnapshotCopiesOnlyMinimalFieldsAndRevisionIgnoresSelectionTime() {
        let contact = CNMutableContact()
        contact.givenName = "Els"
        contact.familyName = "Peeters"
        contact.emailAddresses = [CNLabeledValue(label: CNLabelWork, value: "els@example.test" as NSString)]
        contact.note = "Private contact notes must not leave the picker"
        let first = MeetingContactSnapshot(contact: contact, selectedAtMs: 10)
        let later = MeetingContactSnapshot(contact: contact, selectedAtMs: 20)
        XCTAssertEqual(first.displayName, "Els Peeters")
        XCTAssertEqual(first.email, "els@example.test")
        XCTAssertEqual(first.sourceRevision, later.sourceRevision)
        XCTAssertEqual(first.json["source"] as? String, "macos_contacts")
        XCTAssertEqual(Set(first.json.keys), Set(["source", "source_id", "source_revision", "display_name", "email", "selected_at_ms"]))
        XCTAssertFalse(String(describing: first.json).contains("Private contact notes"))
    }

    @MainActor
    func testContactPickerCancellationRestoresOwnerFocusAndIgnoresLateSelection() async {
        _ = NSApplication.shared
        let owner = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 200), styleMask: [.titled], backing: .buffered, defer: false)
        owner.isReleasedWhenClosed = false
        let field = MeetingContactFocusView(frame: owner.contentView!.bounds)
        owner.contentView!.addSubview(field)
        XCTAssertTrue(owner.makeFirstResponder(field))
        let result = await pickMeetingContact(owner: owner) { picker, parent, delegate in
            XCTAssertTrue(parent === owner)
            XCTAssertTrue(picker.displayedKeys.isEmpty)
            owner.makeFirstResponder(nil)
            delegate.contactPickerDidClose(picker)
            let late = CNMutableContact()
            late.givenName = "Late result"
            delegate.contactPicker(picker, didSelect: late)
        }
        XCTAssertNil(result)
        XCTAssertTrue(owner.firstResponder === field)
        owner.close()
    }

    @MainActor
    func testContactPickerRejectsSelectionAfterItsOwnerCloses() async {
        _ = NSApplication.shared
        let owner = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 200), styleMask: [.titled], backing: .buffered, defer: false)
        owner.isReleasedWhenClosed = false
        let result = await pickMeetingContact(owner: owner) { picker, _, delegate in
            owner.close()
            let contact = CNMutableContact()
            contact.givenName = "Late result"
            delegate.contactPicker(picker, didSelect: contact)
        }
        XCTAssertNil(result)
    }
}

private final class MeetingContactFocusView: NSView {
    override var acceptsFirstResponder: Bool { true }
}
