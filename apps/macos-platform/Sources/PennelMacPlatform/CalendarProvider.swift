import EventKit
import CryptoKit

func peopleSourceRevision(_ value: [String: Any]) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) else { return "" }
    return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

private func sourceDate(_ date: Date?) -> Any {
    guard let date else { return NSNull() }
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter.string(from: date)
}

struct CalendarParticipantSnapshot: Sendable {
    let sourceId: String?
    let name: String?
    let email: String?
    let response: String?
    let isSelf: Bool

    init(name: String?, url: URL?, response: String?, isSelf: Bool) {
        sourceId = url?.absoluteString
        self.name = name
        email = Self.email(from: url)
        self.response = response
        self.isSelf = isSelf
    }

    static func email(from url: URL?) -> String? {
        guard let url, url.scheme?.lowercased() == "mailto",
              let value = URLComponents(url: url, resolvingAgainstBaseURL: false)?.path,
              value.split(separator: "@", omittingEmptySubsequences: false).count == 2,
              !value.hasPrefix("@"), !value.hasSuffix("@"),
              !value.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.contains($0) || CharacterSet.controlCharacters.contains($0) }),
              !value.contains(",") else { return nil }
        return value
    }

    var json: [String: Any] {
        ["source_id": sourceId as Any? ?? NSNull(), "name": name as Any? ?? NSNull(),
         "email": email as Any? ?? NSNull(), "response": response as Any? ?? NSNull(), "is_self": isSelf]
    }
}

struct CalendarContextSnapshot: Sendable {
    let sourceId: String?
    let calendarId: String
    let eventId: String?
    let externalEventId: String?
    let instanceDate: Date?
    let modifiedDate: Date?
    let participants: [CalendarParticipantSnapshot]

    private var fields: [String: Any] {
        ["source": "eventkit", "source_id": sourceId as Any? ?? NSNull(), "account_id": NSNull(),
         "calendar_id": calendarId, "event_id": eventId as Any? ?? NSNull(),
         "external_event_id": externalEventId as Any? ?? NSNull(), "instance_id": sourceDate(instanceDate),
         "modified_at": sourceDate(modifiedDate), "participants": participants.map(\.json)]
    }

    var revision: String { peopleSourceRevision(fields) }
    var json: [String: Any] {
        var value = fields
        value["source_revision"] = revision
        return value
    }
}

struct CalendarSourceScope: Equatable, Sendable {
    let calendarId: String
    let sourceId: String?
}

func calendarSourceIsAvailable(hasAccess: Bool, expected: CalendarSourceScope, current: CalendarSourceScope?) -> Bool {
    guard hasAccess, !expected.calendarId.isEmpty, let current, current.calendarId == expected.calendarId else { return false }
    return expected.sourceId == nil || expected.sourceId == current.sourceId
}

struct CurrentEvent: Sendable {
    let title: String
    let calendarId: String
    let calendarName: String
    let attendees: [String]
    let calendarContext: CalendarContextSnapshot

    var json: [String: Any] {
        ["title": title, "calendar_id": calendarId, "calendar_name": calendarName,
         "attendees": attendees, "calendar_context": calendarContext.json]
    }
}

final class CalendarProvider {
    private let store = EKEventStore()
    private static let meetingHints = ["teams.microsoft.com", "zoom.us", "meet.google.com", "webex.com", "whereby.com"]

    var hasAccess: Bool {
        EKEventStore.authorizationStatus(for: .event) == .fullAccess
    }

    func requestAccess() async -> Bool {
        (try? await store.requestFullAccessToEvents()) ?? false
    }

    func calendars() -> [(id: String, name: String)] {
        guard hasAccess else { return [] }
        return store.calendars(for: .event).map { ($0.calendarIdentifier, $0.title) }
    }

    func sourceAvailable(calendarId: String, sourceId: String?) -> Bool {
        guard hasAccess else { return false }
        let current = store.calendar(withIdentifier: calendarId).map {
            CalendarSourceScope(calendarId: $0.calendarIdentifier, sourceId: $0.source?.sourceIdentifier)
        }
        return calendarSourceIsAvailable(hasAccess: hasAccess,
            expected: CalendarSourceScope(calendarId: calendarId, sourceId: sourceId), current: current)
    }

    /// The event happening now (or starting within 5 minutes), preferring one with a meeting link.
    func currentEvent(now: Date = Date()) -> CurrentEvent? {
        guard hasAccess else { return nil }
        let predicate = store.predicateForEvents(
            withStart: now.addingTimeInterval(-12 * 3600),
            end: now.addingTimeInterval(12 * 3600),
            calendars: nil
        )
        let running = store.events(matching: predicate).filter { event in
            !event.isAllDay && event.startDate.addingTimeInterval(-5 * 60) <= now && now <= event.endDate
        }
        let event = running.first(where: Self.hasMeetingLink) ?? running.first
        return event.map { event in
            let participants = (event.attendees ?? []).map { participant in
                CalendarParticipantSnapshot(name: participant.name, url: participant.url,
                    response: Self.response(participant.participantStatus), isSelf: participant.isCurrentUser)
            }
            return CurrentEvent(
                title: event.title ?? "Meeting",
                calendarId: event.calendar.calendarIdentifier,
                calendarName: event.calendar.title,
                attendees: participants.filter { !$0.isSelf }.compactMap(\.name),
                calendarContext: CalendarContextSnapshot(sourceId: event.calendar.source?.sourceIdentifier,
                    calendarId: event.calendar.calendarIdentifier, eventId: event.eventIdentifier,
                    externalEventId: event.calendarItemExternalIdentifier,
                    instanceDate: event.occurrenceDate ?? event.startDate, modifiedDate: event.lastModifiedDate,
                    participants: participants)
            )
        }
    }

    private static func response(_ status: EKParticipantStatus) -> String? {
        switch status {
        case .unknown: nil
        case .pending: "pending"
        case .accepted: "accepted"
        case .declined: "declined"
        case .tentative: "tentative"
        case .delegated: "delegated"
        case .completed: "completed"
        case .inProcess: "in_process"
        @unknown default: nil
        }
    }

    private static func hasMeetingLink(_ event: EKEvent) -> Bool {
        let text = [event.url?.absoluteString, event.location, event.notes].compactMap { $0 }.joined(separator: " ")
        return meetingHints.contains { text.localizedCaseInsensitiveContains($0) }
    }
}
