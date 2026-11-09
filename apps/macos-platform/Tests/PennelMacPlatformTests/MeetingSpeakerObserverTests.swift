import XCTest
import CoreGraphics
@testable import PennelMacPlatform

private struct Node: ParticipantNode {
    var role = "AXGroup"
    var subrole = ""
    var label = ""
    var text = ""
    var domClasses: [String] = []
    var identifier = ""
    var domIdentifier = ""
    var selected = false
    var checked: Bool? = nil
    var modal: Bool? = nil
    var size: CGSize? = nil
    var children: [Node] = []
    var childrenReadable = true
}

private func tile(_ label: String, shown: String? = nil, ring: [String]? = nil, role: String = "AXMenuItem") -> Node {
    var children = [Node(children: [Node(children: shown.map { [Node(role: "AXStaticText", text: $0)] } ?? [])])]
    if let ring { children.append(Node(domClasses: ring)) }
    return Node(role: role, label: label, children: children)
}

private func window(_ tiles: Node...) -> Node {
    Node(role: "AXWindow", children: [Node(role: "AXWebArea", children: [Node(children: tiles)])])
}

private func soloCallWindow() -> Node {
    window(
        Node(role: "AXToolbar", label: "Meeting controls", domIdentifier: "horizontalMiddleEnd", children: [
            Node(role: "AXButton", label: "Chat", domIdentifier: "chat-button"),
            Node(role: "AXButton", label: "People", domIdentifier: "roster-button"),
            Node(children: [Node(role: "AXButton", label: "Turn camera on", domIdentifier: "video-button")]),
            Node(children: [Node(role: "AXButton", label: "Unmute mic", domIdentifier: "microphone-button")]),
            Node(role: "AXButton", label: "Share", domIdentifier: "share-button"),
        ]),
        Node(role: "AXGroup", label: "Calling controls", domIdentifier: "horizontalEnd", children: [
            Node(children: [Node(role: "AXButton", label: "Leave", domIdentifier: "hangup-button")]),
        ]),
        Node(role: "AXGroup", label: "Shared content view", children: [
            Node(), Node(children: [Node(role: "AXStaticText", text: "Invite people to join you")]), Node(),
        ])
    )
}

private func prejoinWindow() -> Node {
    window(Node(children: [
        Node(role: "AXButton", label: "Cancel"),
        Node(role: "AXButton", label: "Join now", domIdentifier: "prejoin-join-button"),
    ]))
}

final class MeetingSpeakerObserverTests: XCTestCase {
    private let idleRing = ["fui-Primitive", "___dv8x4j0", "f1429bq1"]
    private let speakingRing = ["fui-Primitive", "___1vvhwjq", "vdi-frame-occlusion"]

    func testMeasuredHostAloneCallIsAvailableWithAnEmptyRoster() {
        let solo = soloCallWindow()
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(solo), .call)
        XCTAssertEqual(TeamsParticipantParser.snapshot(windows: [solo]),
                       MeetingSpeakerSnapshot(available: true, participants: [], speaking: []))
    }

    func testSoloWordsWithoutNativeControlIdsCannotProveCallAndMissingRosterStaysUnknown() {
        var chat = soloCallWindow()
        chat.children[0].children[0].children[0].domIdentifier = ""
        var noPlaceholder = soloCallWindow()
        noPlaceholder.children[0].children[0].children[2].children = []
        for unsupported in [chat, window(Node(role: "AXStaticText", text: "Invite people to join you"))] {
            XCTAssertEqual(TeamsParticipantParser.callWindowKind(unsupported), .unreadable)
            XCTAssertFalse(TeamsParticipantParser.snapshot(windows: [unsupported]).available)
        }
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(noPlaceholder), .call)
        XCTAssertTrue(TeamsParticipantParser.hasNativeCallControls(noPlaceholder))
        XCTAssertFalse(TeamsParticipantParser.snapshot(windows: [noPlaceholder]).available,
                       "a proven active call does not invent an empty roster")
    }

    func testKnownPrejoinIsNotAnActiveCallAndCannotOfferNames() {
        var prejoin = prejoinWindow()
        prejoin.children[0].children[0].children.append(tile("Myself video, Ann, video is on", role: "AXImage"))
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(prejoin), .prejoin)
        XCTAssertEqual(TeamsParticipantParser.callEligibility([prejoin]), false)
        XCTAssertFalse(TeamsParticipantParser.snapshot(windows: [prejoin]).available)
        XCTAssertEqual(TeamsParticipantParser.callEligibility([soloCallWindow()]), true)
        XCTAssertNil(TeamsParticipantParser.callEligibility([window(Node(role: "AXButton", label: "Join now"))]))
    }

    func testCallLifecycleCannotBeAuthorizedByChatWordsOrParticipantTileLabelsAlone() {
        XCTAssertFalse(TeamsParticipantParser.hasNativeCallControls(window(
            tile("Ann, video is on", shown: "Ann"),
            Node(role: "AXButton", label: "Join now"),
            Node(role: "AXStaticText", text: "Invite people to join you")
        )))
        XCTAssertTrue(TeamsParticipantParser.hasNativeCallControls(soloCallWindow()))
    }

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

    func testIgnoresDutchOwnVideoLabels() {
        let snapshot = TeamsParticipantParser.snapshot(windows: [window(
            tile("Mijn video, Ann, microfoon aan, video is ingeschakeld", ring: speakingRing, role: "AXImage"),
            tile("Bert Jacobs, video is ingeschakeld", shown: "Bert Jacobs", ring: idleRing)
        )])
        XCTAssertEqual(snapshot.participants, ["Bert Jacobs"])
        XCTAssertEqual(snapshot.speaking, [])
    }

    func testReadsMeasuredPersonalCameraOffTileWithoutAVideoStateSuffix() {
        let guest = Node(role: "AXMenuItem", label: "Pennel QA A (Guest), Context menu is available", children: [
            Node(children: [Node(domClasses: ["fui-Flex", "vdi-occlusion"])]),
            Node(domClasses: speakingRing),
            Node(role: "AXStaticText", text: "Pennel QA A (Guest)"),
        ])
        let snapshot = TeamsParticipantParser.snapshot(windows: [window(guest)])
        XCTAssertEqual(snapshot.participants, ["Pennel QA A (Guest)"])
        XCTAssertEqual(snapshot.speaking, ["Pennel QA A (Guest)"])
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(window(guest)), .call)
    }

    func testContextMenuLabelsAloneAndUnrelatedVideoClassesAreNotParticipantTiles() {
        for candidate in [
            Node(role: "AXMenuItem", label: "Ann, Context menu is available"),
            Node(role: "AXMenuItem", label: "Ann, Context menu is available", children: [Node(domClasses: ["video-tile"])]),
            Node(role: "AXMenuItem", label: "Ann, Available", children: [Node(domClasses: ["vdi-occlusion"])]),
            Node(role: "AXButton", label: "Ann, Context menu is available", children: [Node(domClasses: ["vdi-occlusion"])]),
        ] {
            XCTAssertFalse(TeamsParticipantParser.snapshot(windows: [window(candidate)]).available)
            XCTAssertEqual(TeamsParticipantParser.callWindowKind(window(candidate)), .unreadable)
        }
    }

    func testDuplicateDisplayNamesAreFlaggedInsteadOfCollapsedAsOnePerson() {
        let snapshot = TeamsParticipantParser.snapshot(windows: [window(
            tile("Ann, video is on", shown: "Ann", ring: speakingRing),
            tile("ann, video is off", shown: "ann", ring: idleRing)
        )])
        XCTAssertTrue(snapshot.duplicateNames)
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


final class MeetingSpeakerWindowTests: XCTestCase {
    private func window(pid: Int, number: Int, x: Double = 10, onScreen: Bool = true) -> [String: Any] {
        [
            kCGWindowOwnerPID as String: pid,
            kCGWindowNumber as String: number,
            kCGWindowIsOnscreen as String: onScreen,
            kCGWindowLayer as String: 0,
            kCGWindowAlpha as String: 1.0,
            kCGWindowBounds as String: ["X": x, "Y": 20.0, "Width": 800.0, "Height": 600.0],
        ]
    }

    func testVisibilityRequiresTheSameProcessAndWindowBounds() {
        let bounds = CGRect(x: 10, y: 20, width: 800, height: 600)
        XCTAssertEqual(MeetingSpeakerObserver.visibleWindowNumber(pid: 42, bounds: bounds, windows: [window(pid: 42, number: 7)]), 7)
        XCTAssertNil(MeetingSpeakerObserver.visibleWindowNumber(pid: 42, bounds: bounds, windows: [window(pid: 43, number: 7)]))
        XCTAssertNil(MeetingSpeakerObserver.visibleWindowNumber(pid: 42, bounds: bounds, windows: [window(pid: 42, number: 7, x: 100)]))
        XCTAssertNil(MeetingSpeakerObserver.visibleWindowNumber(pid: 42, bounds: bounds, windows: [window(pid: 42, number: 7, onScreen: false)]))
    }

    func testCompletelyCoveredWindowsAreUnavailableWithoutReadingTheirContents() {
        let bounds = CGRect(x: 10, y: 20, width: 800, height: 600)
        let cover = window(pid: 55, number: 8)
        let target = window(pid: 42, number: 7)
        XCTAssertNil(MeetingSpeakerObserver.visibleWindowNumber(pid: 42, bounds: bounds, windows: [cover, target]))
        XCTAssertEqual(MeetingSpeakerObserver.visibleWindowNumber(pid: 42, bounds: bounds, windows: [target, cover]), 7)
    }

    func testAmbiguousSameSizedWindowsAreUnavailable() {
        let bounds = CGRect(x: 10, y: 20, width: 800, height: 600)
        XCTAssertNil(MeetingSpeakerObserver.visibleWindowNumber(pid: 42, bounds: bounds, windows: [window(pid: 42, number: 7), window(pid: 42, number: 8)]))
    }
}


final class MeetingSpeakerCallAssociationTests: XCTestCase {
    func testOnlyAPreviouslyTrustedBoundPrejoinOrDestroyedWindowProvesCleanEnd() {
        let ended = TeamsCallWindowState(kind: .prejoin, focused: true, bound: true)
        XCTAssertTrue(MeetingSpeakerObserver.cleanCallEnded([ended], hadBinding: true, boundWindowDestroyed: false))
        XCTAssertFalse(MeetingSpeakerObserver.cleanCallEnded([ended], hadBinding: false, boundWindowDestroyed: false))
        let main = TeamsCallWindowState(kind: .other, focused: true)
        XCTAssertTrue(MeetingSpeakerObserver.cleanCallEnded([main], hadBinding: true, boundWindowDestroyed: true))
        XCTAssertFalse(MeetingSpeakerObserver.cleanCallEnded([main], hadBinding: true, boundWindowDestroyed: false))
        XCTAssertFalse(MeetingSpeakerObserver.cleanCallEnded([main, TeamsCallWindowState(kind: .unreadable)], hadBinding: true, boundWindowDestroyed: true))
    }

    func testHiddenMinimizedUnfocusedAmbiguousAndUnsupportedWindowsNeverProveEnd() {
        for candidate in [
            TeamsCallWindowState(kind: .prejoin, focused: true, minimized: true, bound: true),
            TeamsCallWindowState(kind: .prejoin, focused: true, visible: false, bound: true),
            TeamsCallWindowState(kind: .prejoin, bound: true),
            TeamsCallWindowState(kind: .unreadable, focused: true, bound: true),
            TeamsCallWindowState(kind: .other, focused: true, bound: true),
        ] {
            XCTAssertFalse(MeetingSpeakerObserver.cleanCallEnded([candidate], hadBinding: true, boundWindowDestroyed: false))
        }
        XCTAssertFalse(MeetingSpeakerObserver.cleanCallEnded([
            TeamsCallWindowState(kind: .prejoin, focused: true, bound: true),
            TeamsCallWindowState(kind: .call),
        ], hadBinding: true, boundWindowDestroyed: false))
    }

    private func callWindow(_ name: String) -> Node {
        window(tile("\(name), video is on", shown: name))
    }

    private func chatWindow() -> Node {
        window(
            Node(role: "AXTabGroup", identifier: "app-bar", children: [
                Node(role: "AXTab", identifier: "chat-button", selected: true),
                Node(role: "AXTab", identifier: "activity-button"),
                Node(role: "AXTab", identifier: "calendar-button"),
            ]),
            Node(role: "AXMenuItem", label: "Ann Peeters, Available"),
            Node(role: "AXImage", label: "Bert Jacobs, Away"),
            Node(role: "AXTextArea", text: "Private conversation body")
        )
    }

    private func personalNavigation(selected: String = "Calendar") -> Node {
        // Teams personal/free 26289: identifiers are empty, and AXValue marks a checked
        // checkbox even though AXSelected is false. Each control has one group wrapper.
        Node(role: "AXGroup", label: "Apps", children:
            ["Chat", "Meet", "People", "Copilot", "Communities", "Calendar", "Activity"].map { name in
                Node(role: "AXGroup", label: name, children: [
                    Node(role: "AXCheckBox", label: name, checked: name == selected),
                ])
            }
        )
    }

    private func sharingButtonWindow() -> Node {
        Node(role: "AXWindow", subrole: "AXDialog", label: "Window", modal: false,
             size: CGSize(width: 66, height: 20), children: [
                Node(role: "AXButton", label: "WindowSharingSessionButton"),
             ])
    }

    func testMeasuredNativeWindowSharingButtonIsAnAuxiliarySurface() {
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(sharingButtonWindow()), .other)
        XCTAssertNil(MeetingSpeakerObserver.callAssociationReason([
            TeamsCallWindowState(kind: .call, focused: true),
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(sharingButtonWindow())),
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(sharingButtonWindow())),
        ]))
    }

    func testSharingButtonExclusionDoesNotAcceptOtherDialogsOrHideASecondCall() {
        var wrongSubrole = sharingButtonWindow()
        wrongSubrole.subrole = "AXStandardWindow"
        var wrongTitle = sharingButtonWindow()
        wrongTitle.label = "Other window"
        var wrongButton = sharingButtonWindow()
        wrongButton.children[0].label = "More options"
        var missingModality = sharingButtonWindow()
        missingModality.modal = nil
        var modal = sharingButtonWindow()
        modal.modal = true
        var wrongSize = sharingButtonWindow()
        wrongSize.size = CGSize(width: 66, height: 21)
        var additionalContent = sharingButtonWindow()
        additionalContent.children.append(Node())
        var unreadable = sharingButtonWindow()
        unreadable.childrenReadable = false
        var unreadableButton = sharingButtonWindow()
        unreadableButton.children[0].childrenReadable = false
        var nestedContent = sharingButtonWindow()
        nestedContent.children[0].children = [Node(role: "AXGroup")]
        for node in [wrongSubrole, wrongTitle, wrongButton, missingModality, modal, wrongSize, additionalContent, unreadable, unreadableButton, nestedContent] {
            XCTAssertEqual(TeamsParticipantParser.callWindowKind(node), .unreadable)
        }
        XCTAssertEqual(MeetingSpeakerObserver.callAssociationReason([
            TeamsCallWindowState(kind: .call, focused: true),
            TeamsCallWindowState(kind: .call, minimized: true, visible: false),
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(sharingButtonWindow())),
        ]), "ambiguous_call")
        var actualCall = sharingButtonWindow()
        actualCall.children.append(tile("Ann, video is off", shown: "Ann"))
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(actualCall), .call)
    }

    func testMeasuredPersonalNavigationRecognizesTheMainShellOnEachTab() {
        for selected in ["Chat", "Meet", "Calendar"] {
            XCTAssertEqual(TeamsParticipantParser.callWindowKind(window(personalNavigation(selected: selected))), .other)
        }
    }

    func testPersonalNavigationRequiresTheCompleteMeasuredShapeAndOneCheckedControl() {
        var missingControl = personalNavigation()
        missingControl.children.removeLast()
        var multipleChecked = personalNavigation()
        multipleChecked.children[0].children[0].checked = true
        var unknownChecked = personalNavigation()
        unknownChecked.children[0].children[0].checked = nil
        var wrongRole = personalNavigation()
        wrongRole.children[0].children[0].role = "AXButton"
        var mismatchedWrapper = personalNavigation()
        mismatchedWrapper.children[0].label = "Calendar"
        var arbitraryGroup = personalNavigation()
        arbitraryGroup.label = "Other controls"
        for navigation in [missingControl, multipleChecked, unknownChecked, wrongRole, mismatchedWrapper, arbitraryGroup] {
            XCTAssertEqual(TeamsParticipantParser.callWindowKind(window(navigation)), .unreadable)
        }
    }

    func testParticipantTilesStillMakeAMainNavigationWindowACall() {
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(window(
            personalNavigation(), tile("Ann, video is off", shown: "Ann")
        )), .call)
    }

    func testTwoDistinctCallsAreAmbiguousEvenWithOneFocused() {
        let windows = [callWindow("Ann"), callWindow("Bert")]
        let states = windows.enumerated().map { offset, node in
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(node), focused: offset == 0)
        }
        XCTAssertEqual(MeetingSpeakerObserver.callAssociationReason(states), "ambiguous_call")
    }

    func testMinimizedOrCoveredCompetingCallStillMakesAudioAssociationAmbiguous() {
        let focused = TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(callWindow("Ann")), focused: true)
        for competing in [
            TeamsCallWindowState(kind: .call, minimized: true, visible: false),
            TeamsCallWindowState(kind: .call, visible: false),
        ] {
            XCTAssertEqual(MeetingSpeakerObserver.callAssociationReason([focused, competing]), "ambiguous_call")
        }
    }

    func testOrdinaryChatAndMainWindowDoNotBecomeCallsFromCommaLabels() {
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(chatWindow()), .other)
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(callWindow("Ann")), .call)
        let states = [
            TeamsCallWindowState(kind: .call, focused: true),
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(chatWindow()), visible: false),
        ]
        XCTAssertNil(MeetingSpeakerObserver.callAssociationReason(states))
        XCTAssertFalse(TeamsParticipantParser.snapshot(windows: [chatWindow()]).available)
    }

    func testUnreadableOrUnclassifiedCompetingWindowBlocksAutomaticAssociation() {
        let states = [
            TeamsCallWindowState(kind: .call, focused: true),
            TeamsCallWindowState(kind: .unreadable, minimized: true, visible: false),
        ]
        XCTAssertEqual(MeetingSpeakerObserver.callAssociationReason(states), "ambiguous_call")
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(Node(role: "AXWindow")), .unreadable)
    }

    func testGenericCompleteControlTreeIsAnUnclassifiedPossibleSecondCall() {
        let compactAudioCall = window(
            Node(role: "AXButton", label: "Audio control"),
            Node(role: "AXButton", label: "More options")
        )
        XCTAssertEqual(TeamsParticipantParser.callWindowKind(compactAudioCall), .unreadable)
        let states = [
            TeamsCallWindowState(kind: .call, focused: true),
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(compactAudioCall), visible: false),
        ]
        XCTAssertEqual(MeetingSpeakerObserver.callAssociationReason(states), "ambiguous_call")
    }

    func testNavigationRoleOrIdentifierAloneDoesNotProveANonCallWindow() {
        for navigation in [
            Node(role: "AXTabGroup", children: [Node(role: "AXButton", label: "Chat")]),
            Node(role: "AXTabGroup", identifier: "app-bar", children: [Node(role: "AXTab", identifier: "chat-button")]),
            Node(role: "AXGroup", identifier: "not-app-navigation", children: [Node(role: "AXTab", identifier: "chat-button", selected: true)]),
        ] {
            XCTAssertEqual(TeamsParticipantParser.callWindowKind(window(navigation)), .unreadable)
        }
    }

    func testSameCallCompactAndFullViewsRemainAmbiguousWithoutAnIndependentCallIdentity() {
        let states = [
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(callWindow("Ann")), focused: true),
            TeamsCallWindowState(kind: TeamsParticipantParser.callWindowKind(callWindow("Ann"))),
        ]
        XCTAssertEqual(MeetingSpeakerObserver.callAssociationReason(states), "ambiguous_call")
    }
}
