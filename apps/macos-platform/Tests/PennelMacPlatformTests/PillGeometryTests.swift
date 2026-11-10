import AppKit
import XCTest
@testable import PennelMacPlatform

final class PillGeometryTests: XCTestCase {
    func testPositionsRespectNegativeDisplayOriginAndVisibleArea() {
        let visible = NSRect(x: -1440, y: 120, width: 1440, height: 840)
        XCTAssertEqual(pillOverlayFrame(visible: visible, cursor: NSPoint(x: -700, y: 600), width: 420, height: 120, placement: .bottomCentre), NSRect(x: -930, y: 120, width: 420, height: 120))
        XCTAssertEqual(pillOverlayFrame(visible: visible, cursor: NSPoint(x: -700, y: 600), width: 420, height: 120, placement: .topCentre), NSRect(x: -930, y: 840, width: 420, height: 120))
        XCTAssertEqual(pillOverlayFrame(visible: visible, cursor: NSPoint(x: -700, y: 600), width: 420, height: 120, placement: .nearCursor), NSRect(x: -910, y: 468, width: 420, height: 120))
    }

    func testCursorAndAllSizesAreClampedToTinyWorkAreas() {
        let visible = NSRect(x: -300, y: -100, width: 300, height: 80)
        for placement in [PillPlacement.bottomCentre, .topCentre, .nearCursor] {
            XCTAssertEqual(pillOverlayFrame(visible: visible, cursor: NSPoint(x: 9000, y: -9000), width: 500, height: 150, placement: placement), visible)
        }
        XCTAssertNil(pillOverlayFrame(visible: .zero, cursor: .zero, width: 420, height: 120, placement: .bottomCentre))
        XCTAssertNil(pillOverlayFrame(visible: visible, cursor: .zero, width: .nan, height: 120, placement: .topCentre))
    }

    func testFractionalPointGeometryNeverRoundsOutsideTheWorkArea() {
        let visible = NSRect(x: -300.5, y: 30.25, width: 800.25, height: 500.5)
        for cursor in [NSPoint(x: -9000, y: 9000), NSPoint(x: 9000, y: -9000)] {
            let frame = pillOverlayFrame(visible: visible, cursor: cursor, width: 420, height: 120, placement: .nearCursor)!
            XCTAssertTrue(visible.contains(frame))
        }
    }
}
