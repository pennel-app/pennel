import PennelMobileKit
import SwiftUI

/// Pennel's dictation control in the keyboard bar: a high-contrast capsule that reads "Start" without a session
/// and becomes a compact waveform, stop or progress control once the session is ready.
struct PennelDictationButton: View {
    let model: KeyboardModel
    let label: String
    let height: CGFloat
    let actionValue: String
    let hint: String
    let tap: () -> Void
    let holdBegan: () -> Void
    let holdEnded: () -> Void
    var prepareFeedback: () -> Void = {}
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Inverts against the keyboard backdrop: white on dark keyboards, near-black on light ones.
    private static let fill = Brand.Token(light: 0x1C1C1E, dark: 0xFFFFFF)
    private static let mark = Brand.Token(light: 0xFFFFFF, dark: 0x000000)

    var body: some View {
        content
            .foregroundStyle(Self.mark.color)
            .frame(height: height)
            .glassEffect(.regular.tint(Self.fill.color), in: Capsule())
            .opacity(model.pad == .needsFullAccess ? 0.6 : 1)
            .frame(minWidth: Brand.Spacing.s10 + Brand.Spacing.s1, minHeight: Brand.Spacing.s10 + Brand.Spacing.s1)
            .contentShape(Rectangle())
            .accessibilityHidden(true)
            .overlay {
                KeyboardDictationGesture(label: launches ? label : model.primaryActionLabel, hint: hint, value: actionValue,
                    enabled: model.pad != .processing, holdEnabled: model.pad == .ready,
                    tap: tap, holdBegan: holdBegan, holdEnded: holdEnded, prepareFeedback: prepareFeedback)
            }
            .animation(reduceMotion ? nil : .easeOut(duration: 0.2), value: model.pad)
    }

    private var launches: Bool { model.pad == .sessionOff || model.pad == .needsFullAccess }

    @ViewBuilder private var content: some View {
        if launches {
            HStack(spacing: Brand.Spacing.s3) {
                Text(label).font(.keyboard(19, weight: .medium)).lineLimit(1)
                Image(systemName: "waveform").font(.keyboard(19, weight: .semibold))
            }
            .padding(.horizontal, Brand.Spacing.s5)
        } else {
            icon.frame(width: height * 1.6)
        }
    }

    @ViewBuilder private var icon: some View {
        switch model.pad {
        case .listening:
            Image(systemName: "waveform")
                .font(.keyboard(19, weight: .semibold))
                .symbolEffect(.variableColor.iterative, options: .repeating, isActive: !reduceMotion)
        case .processing:
            ProgressView().controlSize(.small).tint(Self.mark.color)
        case .failed:
            Image(systemName: "arrow.clockwise").font(.keyboard(17, weight: .semibold))
        default:
            Image(systemName: "waveform").font(.keyboard(19, weight: .semibold))
        }
    }
}
