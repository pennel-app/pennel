import PennelMobileKit
import SwiftUI

/// Pennel's dictation control in the keyboard bar. It keeps the brand ink and soft-wave colors on top of the
/// otherwise native keyboard: a "Start Pennel" capsule without a session, a round microphone when ready.
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
    /// Ink disappears on the dark keyboard backdrop, so dark mode inverts to soft wave with ink marks.
    private static let fill = Brand.Token(light: Brand.ink, dark: Brand.softWave)
    private static let mark = Brand.Token(light: Brand.softWave, dark: Brand.ink)
    private static let text = Brand.Token(light: Brand.paper, dark: Brand.ink)

    var body: some View {
        content
            .frame(height: height)
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
            HStack(spacing: Brand.Spacing.s1 + 2) {
                Image(systemName: "mic.fill").font(.keyboard(13, weight: .semibold)).foregroundStyle(Self.mark.color)
                Text(label).font(.keyboard(KeyStyle.barSize, weight: .semibold)).foregroundStyle(Self.text.color).lineLimit(1)
            }
            .padding(.horizontal, Brand.Spacing.s3)
            .frame(height: height)
            .background(Self.fill.color, in: Capsule())
            .opacity(model.pad == .needsFullAccess ? 0.6 : 1)
        } else {
            ZStack {
                Circle().fill(Self.fill.color)
                if model.pad == .listening {
                    Circle()
                        .stroke(Self.fill.color.opacity(0.45), lineWidth: 3)
                        .scaleEffect(1 + CGFloat(min(max(model.level, 0), 1)) * 0.35)
                        .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: model.level)
                }
                icon
            }
            .frame(width: height, height: height)
        }
    }

    @ViewBuilder private var icon: some View {
        switch model.pad {
        case .listening:
            Image(systemName: "stop.fill").font(.keyboard(14, weight: .bold)).foregroundStyle(Self.mark.color)
        case .processing:
            ProgressView().controlSize(.small).tint(Self.mark.color)
        case .failed:
            Image(systemName: "arrow.clockwise").font(.keyboard(15, weight: .semibold)).foregroundStyle(Self.mark.color)
        default:
            Image(systemName: "mic.fill").font(.keyboard(16, weight: .semibold)).foregroundStyle(Self.mark.color)
        }
    }
}
