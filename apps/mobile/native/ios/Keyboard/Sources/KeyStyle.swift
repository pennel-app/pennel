import PennelMobileKit
import SwiftUI
import UIKit

/// The keyboard follows the system keyboard rather than Pennel's app palette, so it sits naturally in any host app.
/// Content stays clear so iOS draws its own keyboard backdrop; key colors are tuned over that backdrop.
/// Only the dictation button keeps Pennel's ink and soft-wave colors.
enum KeyStyle {
    /// iOS 26 draws every key in one shade; only a press changes it.
    static let letterKey = Brand.Token(light: 0xFFFFFF, dark: 0xFFFFFF, darkAlpha: 0.24)
    static let functionKey = letterKey
    static let pressedKey = Brand.Token(light: 0xC9CCD3, dark: 0xFFFFFF, darkAlpha: 0.42)
    static let keyShadow = Brand.Token(light: 0x000000, dark: 0x000000, lightAlpha: 0.14, darkAlpha: 0)
    static let label = Brand.Token(light: 0x000000, dark: 0xFFFFFF)
    static let secondaryLabel = Brand.Token(light: 0x3C3C43, dark: 0xEBEBF5, lightAlpha: 0.60, darkAlpha: 0.60)
    static let tertiaryLabel = Brand.Token(light: 0x3C3C43, dark: 0xEBEBF5, lightAlpha: 0.30, darkAlpha: 0.30)
    static let accent = Brand.Token(light: 0x007AFF, dark: 0x0A84FF)
    static let accentText = Brand.Token(light: 0xFFFFFF, dark: 0xFFFFFF)
    static let separator = Brand.Token(light: 0x3C3C43, dark: 0xEBEBF5, lightAlpha: 0.18, darkAlpha: 0.18)

    static let keyRadius: CGFloat = 10
    static let keyHeight: CGFloat = 42
    static let compactKeyHeight: CGFloat = 32
    static let horizontalGap: CGFloat = 6
    static let rowGap: CGFloat = 11
    static let sideMargin: CGFloat = 3
    static let letterSize: CGFloat = 25
    static let functionSize: CGFloat = 17
    static let barSize: CGFloat = 15

    static func uiFont(_ size: CGFloat, weight: UIFont.Weight = .regular) -> UIFont {
        .systemFont(ofSize: size, weight: weight)
    }

    static func styleKeyShadow(_ layer: CALayer) {
        layer.shadowColor = UIColor(keyShadow.color).cgColor
        layer.shadowOpacity = 1
        layer.shadowRadius = 0
        layer.shadowOffset = CGSize(width: 0, height: 1)
    }
}

extension Font {
    static func keyboard(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
    }
}
