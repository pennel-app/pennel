import XCTest
@testable import PennelMacPlatform

private struct Node: ParticipantNode {
    var role = "AXGroup"
    var label = ""
    var text = ""
    var domClasses: [String] = []
    var children: [Node] = []
}

private func tile(_ label: String, shown: String? = nil, ring: [String]? = nil, role: String = "AXMenuItem") -> Node {
    var children = [Node(children: [Node(children: shown.map { [Node(role: "AXStaticText", text: $0)] } ?? [])])]
    if let ring { children.append(Node(domClasses: ring)) }
    return Node(role: role, label: label, children: children)
}

private func window(_ tiles: Node...) -> Node {
    Node(role: "AXWindow", children: [Node(role: "AXWebArea", children: [Node(children: tiles)])])
}

final class MeetingSpeakerObserverTests: XCTestCase {
    private let idleRing = ["fui-Primitive", "___dv8x4j0", "f1429bq1"]
    private let speakingRing = ["fui-Primitive", "___1vvhwjq", "vdi-frame-occlusion"]

    func testReadsParticipantsAndTheHighlightedSpeaker() {
        let snapshot = TeamsParticipantParser.snapshot(windows: [window(
            tile("Ann Peeters, video is on, Context menu is available", shown: "Ann Peeters", ring: speakingRing),
            tile("Bert Jacobs, video is on, Context menu is available", shown: "Bert Jacobs", ring: idleRing),
            tile("Cas Wouters, video is on, muted, Context menu is available", shown: "Cas Wouters")
        )])

        XCTAssertEqual(snapshot, MeetingSpeakerSnapshot(
            available: true,
            participants: ["Ann Peeters", "Bert Jacobs", "Cas Wouters"],
            speaking: ["Ann Peeters"]
        ))
    }

    func testDisplayedNameKeepsCommasThatTheLabelSplits() {
        let snapshot = TeamsParticipantParser.snapshot(windows: [window(
            tile("Peeters, Ann, video is on, Context menu is available", shown: "Peeters, Ann", ring: speakingRing)
        )])

        XCTAssertEqual(snapshot.speaking, ["Peeters, Ann"])
    }

    func testFallsBackToTheLabelWhenTheTileShowsNoName() {
        let snapshot = TeamsParticipantParser.snapshot(windows: [window(
            tile("Ann Peeters, video is off, Context menu is available", ring: speakingRing)
        )])

        XCTAssertEqual(snapshot.participants, ["Ann Peeters"])
        XCTAssertEqual(snapshot.speaking, ["Ann Peeters"])
    }

    func testIgnoresTheLocalUsersTile() {
        let snapshot = TeamsParticipantParser.snapshot(windows: [window(
            tile("Myself video, Ignace, Unmuted, video is on", ring: speakingRing, role: "AXImage"),
            tile("Ann Peeters, video is on", shown: "Ann Peeters", ring: idleRing)
        )])

        XCTAssertEqual(snapshot.participants, ["Ann Peeters"])
        XCTAssertEqual(snapshot.speaking, [])
    }

    func testMergesTheCompactAndFullMeetingWindows() {
        let compact = window(tile("Ann Peeters, video is on", ring: speakingRing))
        let full = window(
            tile("Ann Peeters, video is on", shown: "Ann Peeters", ring: speakingRing),
            tile("Bert Jacobs, video is on", shown: "Bert Jacobs", ring: idleRing)
        )

        let snapshot = TeamsParticipantParser.snapshot(windows: [compact, full])

        XCTAssertEqual(snapshot.participants, ["Ann Peeters", "Bert Jacobs"])
        XCTAssertEqual(snapshot.speaking, ["Ann Peeters"])
    }

    func testAWindowWithoutTilesIsUnavailable() {
        let chat = Node(role: "AXWindow", children: [
            Node(role: "AXButton", label: "Ann Peeters, Available"),
            Node(role: "AXStaticText", text: "Hello, everyone"),
        ])

        XCTAssertEqual(
            TeamsParticipantParser.snapshot(windows: [chat]),
            MeetingSpeakerSnapshot(available: false, participants: [], speaking: [])
        )
    }

    func testReturnsTheElementsThatHoldTilesForCheapResampling() {
        let row = Node(label: "row", children: [
            tile("Ann Peeters, video is on", shown: "Ann Peeters", ring: speakingRing),
            tile("Bert Jacobs, video is on", shown: "Bert Jacobs", ring: idleRing),
        ])
        let scan = TeamsParticipantParser.scan([Node(role: "AXWindow", children: [Node(children: [row])])])

        XCTAssertEqual(scan.containers.map(\.label), ["row"])
        XCTAssertEqual(TeamsParticipantParser.scan(scan.containers).snapshot, scan.snapshot)
    }

    func testOnlyTeamsIsSupported() {
        XCTAssertFalse(MeetingSpeakerObserver.snapshot(appId: "us.zoom.xos").available)
    }
}

final class MeetingSpeakerMergeTests: XCTestCase {
    func testMergeKeepsFirstSeenOrderWithoutDuplicates() {
        let merged = MeetingSpeakerObserver.merge(
            MeetingSpeakerSnapshot(available: true, participants: ["Ann", "Bert"], speaking: ["Ann"]),
            MeetingSpeakerSnapshot(available: true, participants: ["Bert", "Cas"], speaking: ["Ann", "Cas"])
        )

        XCTAssertEqual(merged, MeetingSpeakerSnapshot(available: true, participants: ["Ann", "Bert", "Cas"], speaking: ["Ann", "Cas"]))
    }
}
