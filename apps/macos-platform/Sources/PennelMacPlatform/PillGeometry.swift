import AppKit

/// Raw values are the placement tags passed by the shared desktop shell.
enum PillPlacement: Int32 {
    case bottomCentre = 0
    case topCentre = 1
    case nearCursor = 2
}

/// AppKit uses logical points and an upward Y axis, including on displays with negative origins.
func pillOverlayFrame(visible: NSRect, cursor: NSPoint, width: CGFloat, height: CGFloat, placement: PillPlacement) -> NSRect? {
    guard [visible.minX, visible.minY, visible.width, visible.height, cursor.x, cursor.y, width, height].allSatisfy(\.isFinite),
          visible.width > 0, visible.height > 0, width > 0, height > 0 else { return nil }
    let size = NSSize(width: min(width, visible.width), height: min(height, visible.height))
    let desired: NSPoint
    switch placement {
    case .bottomCentre: desired = NSPoint(x: visible.midX - size.width / 2, y: visible.minY)
    case .topCentre: desired = NSPoint(x: visible.midX - size.width / 2, y: visible.maxY - size.height)
    case .nearCursor: desired = NSPoint(x: cursor.x - size.width / 2, y: cursor.y - size.height - 12)
    }
    return NSRect(x: min(max(desired.x.rounded(), visible.minX), visible.maxX - size.width),
                  y: min(max(desired.y.rounded(), visible.minY), visible.maxY - size.height),
                  width: size.width, height: size.height)
}
