import PennelMobileKit
import SwiftUI

struct DictationPad: View {
    let model: KeyboardModel
    let openSettings: () -> Void
    var prepareFeedback: () -> Void = {}
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: Brand.Spacing.s2) {
            switch model.pad {
            case .needsFullAccess:
                Text(model.copy.text(.fullAccess))
                    .font(.figtree(Brand.FontSize.content, weight: .medium))
                    .foregroundStyle(Brand.foreground.color)
                    .multilineTextAlignment(.center)
                Button(model.copy.text(.settings), action: openSettings)
                    .buttonStyle(PadButtonStyle())
            case .failed(let message):
                Text(message)
                    .font(.figtree(Brand.FontSize.content, weight: .medium))
                    .foregroundStyle(Brand.foreground.color)
                    .multilineTextAlignment(.center)
                Button(model.primaryActionLabel) {
                    if model.recoveryAction == .recordAgain { model.recordAgain() } else { model.retry() }
                }
                    .buttonStyle(PadButtonStyle())
            case .sessionOff:
                pill(label: model.copy.text(.sessionOff))
                caption(model.copy.text(.manualStart))
            case .ready:
                pill(label: nil)
                caption(model.copy.text(.ready))
            case .listening:
                pill(label: nil)
                HStack(spacing: Brand.Spacing.s3) {
                    caption(model.copy.text(.listening))
                    Button { model.cancel() } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 22))
                            .foregroundStyle(Brand.muted.color)
                            .frame(width: Brand.Spacing.s10 + Brand.Spacing.s1, height: Brand.Spacing.s10 + Brand.Spacing.s1)
                            .contentShape(Rectangle())
                    }
                    .accessibilityLabel(model.copy.text(.cancel))
                }
            case .processing:
                pill(label: nil)
                HStack(spacing: Brand.Spacing.s3) {
                    caption(model.copy.text(.processing))
                    Button { model.cancel() } label: {
                        Image(systemName: "xmark.circle.fill")
                            .frame(width: Brand.Spacing.s10 + Brand.Spacing.s1, height: Brand.Spacing.s10 + Brand.Spacing.s1)
                            .contentShape(Rectangle())
                    }
                        .accessibilityLabel(model.copy.text(.cancel))
                }
            }
            if let partial = model.partialText, model.pad == .listening {
                Text(partial)
                    .font(.figtree(Brand.FontSize.content))
                    .foregroundStyle(Brand.muted.color)
                    .lineLimit(3)
                    .accessibilityLabel(model.copy.language == .dutch ? "Voorbeeld: \(partial)" : "Preview: \(partial)")
            }
            if let preview = model.lastResultPreview, model.pad != .listening, model.pad != .processing {
                Text(preview)
                    .font(.figtree(Brand.FontSize.content))
                    .foregroundStyle(Brand.muted.color)
                    .lineLimit(2)
                Button(model.copy.text(.insertLast)) { model.insertLast() }
                    .buttonStyle(PadButtonStyle())
            }
        }
        .frame(maxWidth: .infinity)
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .font(.figtree(Brand.FontSize.body, weight: .medium))
            .foregroundStyle(Brand.muted.color)
            .multilineTextAlignment(.center)
    }

    private func pill(label: String?) -> some View {
        let active = model.pad == .listening
        return ZStack {
            Capsule().fill(Brand.pillBackground.color)
            if let label {
                Text(label)
                    .font(.figtree(Brand.FontSize.section, weight: .semibold))
                    .foregroundStyle(Brand.pillText.color)
            } else {
                if model.pad == .processing {
                    PenMark()
                        .frame(height: 40)
                        .transition(.opacity)
                } else {
                    Bars(level: active ? model.level : 0)
                        .frame(height: 36)
                        .transition(.opacity)
                }
            }
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.25), value: model.pad == .processing)
        .frame(height: 76)
        .padding(.horizontal, Brand.Spacing.s6)
        .contentShape(Capsule())
        .accessibilityHidden(true)
        .overlay {
            KeyboardDictationGesture(label: model.primaryActionLabel,
                hint: model.pad == .sessionOff ? model.copy.text(.manualStart) : "",
                enabled: model.primaryActionEnabled, holdEnabled: model.pad == .ready,
                tap: model.tapPill, holdBegan: model.holdBegan, holdEnded: model.holdEnded,
                prepareFeedback: prepareFeedback)
                .padding(.horizontal, Brand.Spacing.s6)
        }
    }
}

private struct Bars: View {
    let level: Float
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GeometryReader { proxy in
            let heights = Brand.Wave.barHeights(level: level, height: proxy.size.height)
            let width = proxy.size.height * Brand.Wave.barWidthRatio
            HStack(spacing: proxy.size.height * Brand.Wave.spacingRatio) {
                ForEach(heights.indices, id: \.self) { index in
                    Capsule()
                        .fill(Brand.pillAccent.color)
                        .frame(width: width, height: heights[index])
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: level)
        }
    }
}

struct PadButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.figtree(Brand.FontSize.content, weight: .semibold))
            .foregroundStyle(Brand.buttonText.color)
            .padding(.horizontal, Brand.Spacing.s5)
            .frame(minHeight: Brand.Spacing.s10 + Brand.Spacing.s1)
            .background(Brand.button.color.opacity(configuration.isPressed ? 0.8 : 1), in: Capsule())
    }
}

/// The stroke and full stop of `assets/brand/mark.svg`, in Soft wave because orange never sits on ink.
private struct PenMark: View {
    private static let strokeWidth: CGFloat = 7
    private static let dot = CGRect(x: 74 - 7.5, y: 90 - 7.5, width: 15, height: 15)

    private static let stroke: Path = {
        var path = Path()
        path.move(to: CGPoint(x: 16, y: 76))
        path.addCurve(to: CGPoint(x: 30, y: 56), control1: CGPoint(x: 21, y: 68), control2: CGPoint(x: 26, y: 62))
        path.addCurve(to: CGPoint(x: 25, y: 134), control1: CGPoint(x: 29, y: 82), control2: CGPoint(x: 27, y: 112))
        path.addCurve(to: CGPoint(x: 35, y: 70), control1: CGPoint(x: 28, y: 112), control2: CGPoint(x: 31, y: 86))
        path.addCurve(to: CGPoint(x: 66, y: 72), control1: CGPoint(x: 40, y: 56), control2: CGPoint(x: 66, y: 52))
        path.addCurve(to: CGPoint(x: 36, y: 88), control1: CGPoint(x: 66, y: 92), control2: CGPoint(x: 44, y: 98))
        path.addCurve(to: CGPoint(x: 56, y: 90), control1: CGPoint(x: 42, y: 92), control2: CGPoint(x: 50, y: 92))
        // The SVG group is translate(12 -44) skewX(-8).
        let skew = CGAffineTransform(a: 1, b: 0, c: tan(-8 * .pi / 180), d: 1, tx: 12, ty: -44)
        return path.applying(skew)
    }()

    private static let dotPath = Path(ellipseIn: dot).applying(CGAffineTransform(a: 1, b: 0, c: tan(-8 * .pi / 180), d: 1, tx: 12, ty: -44))

    var body: some View {
        GeometryReader { proxy in
            // The mark occupies roughly x 6..76, y 8..92 of the 96 box.
            let box = CGRect(x: 6, y: 6, width: 76, height: 88)
            let scale = proxy.size.height / box.height
            let transform = CGAffineTransform(translationX: -box.minX, y: -box.minY).scaledBy(x: scale, y: scale)
            ZStack {
                Self.stroke.applying(transform)
                    .stroke(Brand.pillAccent.color, style: StrokeStyle(lineWidth: Self.strokeWidth * scale, lineCap: .round, lineJoin: .round))
                Self.dotPath.applying(transform).fill(Brand.pillAccent.color)
            }
            .frame(width: box.width * scale, height: proxy.size.height, alignment: .topLeading)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .accessibilityHidden(true)
    }
}
