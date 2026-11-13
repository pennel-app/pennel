import PennelMobileKit
import SwiftUI

struct KeyboardRoot: View {
    let model: KeyboardModel
    let typing: KeyboardTypingModel
    let layout: KeyboardLayout
    let showsGlobe: Bool
    let inputModeTarget: KeyboardInputModeTarget
    let openSettings: () -> Void
    var openPennel: (URL, @escaping (Bool) -> Void) -> Void = { _, done in done(false) }
    let insert: (String) -> Void
    let delete: () -> Void
    let space: () -> Void
    let insertSuggestion: (String) -> Void
    let toggleSuggestions: () -> Void
    let feedback: (FeedbackEvent) -> Void
    let heightChanged: (CGFloat) -> Void
    var prepareFeedback: () -> Void = {}
    var moveCursor: (Int) -> Void = { _ in }
    var deleteWord: () -> Void = {}
    var swipe: (KeyboardSwipeGesture) -> Void = { _ in }
    var insertToolText: (String) -> Void = { _ in }
    var requestWriting: (KeyboardWritingAction) -> Void = { _ in }
    var applyWriting: () -> Void = {}
    var restoreWriting: () -> Void = {}
    var cancelWriting: () -> Void = {}
    var undoCorrection: () -> Void = {}
    var saveSnippet: (String, String) -> Void = { _, _ in }
    var deleteSnippet: (KeyboardSnippet) -> Void = { _ in }
    var recordTouch: (KeyboardSwipeGesture) -> Void = { _ in }
    @State private var showsHelp = false
    @State private var launchFailed = false
    @State private var showsTools = false
    @State private var toolTab = KeyboardToolTab.emoji
    @State private var toolQuery = ""
    @State private var toolTypingState = KeyboardTypingState()
    @State private var layerBeforeSlide = KeyboardLayer.letters
    @State private var toolsBeforeSlide = false
    @State private var letterInputTarget = KeyboardLetterInputTarget()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    @Environment(\.openURL) private var openURL
    @ScaledMetric(relativeTo: .body) private var scaledKeyHeight = KeyStyle.keyHeight
    /// Minimum touch target for bar controls.
    private let controlWidth = Brand.Spacing.s10 + Brand.Spacing.s1
    private let barHeight = Brand.Spacing.s10 + Brand.Spacing.s1
    private var keyHeight: CGFloat {
        if verticalSizeClass == .compact { return max(KeyStyle.compactKeyHeight, min(scaledKeyHeight, KeyStyle.keyHeight)) }
        return max(KeyStyle.keyHeight, min(scaledKeyHeight, KeyStyle.keyHeight * 1.5))
    }

    var body: some View {
        VStack(spacing: KeyStyle.rowGap) {
            toolbar
            #if PENNEL_KEYBOARD_FEASIBILITY
            if Bundle.main.bundleIdentifier == "app.pennel.ios.preview.keyboard" {
                HStack(spacing: Brand.Spacing.s2) {
                    Text("QA · Process routing only")
                        .font(.keyboard(Brand.FontSize.body))
                        .foregroundStyle(KeyStyle.secondaryLabel.color)
                    Spacer()
                    KeyboardRecordingIntentProbeButton()
                        .frame(minHeight: controlWidth)
                        .accessibilityHint("Tests intent routing. No microphone capture is started.")
                }
                .font(.keyboard(Brand.FontSize.content, weight: .semibold))
                .padding(.horizontal, Brand.Spacing.s2)
            }
            #endif
            if showsHelp { guidance }
            if let preview = model.lastResultPreview, model.pad != .listening, model.pad != .processing { recovery(preview) }
            if showsTools {
                KeyboardToolsPanel(typing: typing, tab: $toolTab, query: $toolQuery, insert: insertToolText,
                    requestWriting: requestWriting, applyWriting: applyWriting, restoreWriting: restoreWriting,
                    cancelWriting: cancelWriting, saveSnippet: saveSnippet, deleteSnippet: deleteSnippet, feedback: feedback)
                    .frame(height: keyHeight * (toolTab == .writing ? 5 : 3) + KeyStyle.rowGap * 2)
            }
            if !showsTools || toolTab != .writing {
                LetterPad(layout: layout, typing: typing, insert: insertText, delete: deleteText, feedback: feedback,
                    prepareFeedback: prepareFeedback, deleteWord: deletePreviousWord, swipe: swipe,
                    inputTarget: letterInputTarget, allowSwipe: !searchesTools,
                    recordTouch: { if !searchesTools { recordTouch($0) } },
                    stateOverride: searchesTools ? $toolTypingState : nil)
            }
            bottomRow
        }
        .padding(.horizontal, KeyStyle.sideMargin)
        .padding(.top, Brand.Spacing.s1)
        .padding(.bottom, Brand.Spacing.s1)
        .background(Color.clear)
        .buttonStyle(KeyboardPreparationButtonStyle(prepare: prepareFeedback))
        .fixedSize(horizontal: false, vertical: true)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { heightChanged($0) }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.15), value: showsHelp)
        .onChange(of: model.pad) { _, pad in
            if pad != .sessionOff { launchFailed = false }
            if pad == .ready || pad == .listening || pad == .processing { showsHelp = false }
            if case .failed = pad { showsHelp = true }
        }
    }

    /// The system prediction bar: tools on the left, suggestions or live dictation status in the middle,
    /// and Pennel's dictation button on the right.
    private var toolbar: some View {
        HStack(spacing: Brand.Spacing.s1) {
            barIcon(showsTools ? "keyboard" : "face.smiling", active: showsTools,
                label: typing.copy.language == .dutch ? (showsTools ? "Toon toetsenbord" : "Emoji, tekstfragmenten en schrijven") : (showsTools ? "Show keyboard" : "Emoji, snippets and writing")) {
                showsTools.toggle(); toolQuery = ""
                if !showsTools { cancelWriting() }
                feedback(.selection)
            }
            Group {
                if model.pad == .listening || model.pad == .processing {
                    dictationStatus
                } else if !showsTools, typing.suggestionsEnabled || typing.correctionOriginal != nil {
                    suggestionRow
                } else {
                    Spacer(minLength: 0)
                }
            }
            .frame(maxWidth: .infinity, minHeight: barHeight)
            if model.pad != .listening, model.pad != .processing {
                barIcon(typing.suggestionsEnabled ? "text.bubble.fill" : "text.bubble", active: typing.suggestionsEnabled,
                    label: typing.copy.text(typing.suggestionsEnabled ? .suggestions : .suggestionsOff),
                    hint: typing.copy.text(.suggestionsPrivacy), action: toggleSuggestions)
            }
            PennelDictationButton(model: model, label: typing.copy.text(.startPennel), height: barHeight - Brand.Spacing.s2,
                actionValue: actionValue, hint: model.pad == .sessionOff ? typing.copy.text(.startPennelHint) : "",
                tap: microphoneTapped,
                holdBegan: { showsTools = false; cancelWriting(); model.holdBegan() }, holdEnded: model.holdEnded,
                prepareFeedback: prepareFeedback)
                .padding(.trailing, Brand.Spacing.s1)
        }
        .frame(height: barHeight)
    }

    private var dictationStatus: some View {
        HStack(spacing: Brand.Spacing.s2) {
            Button { model.cancel() } label: {
                Image(systemName: "xmark")
                    .font(.keyboard(KeyStyle.barSize, weight: .semibold))
                    .foregroundStyle(KeyStyle.secondaryLabel.color)
                    .frame(width: controlWidth, height: barHeight)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel(typing.copy.text(.cancel))
            Text(model.pad == .listening ? (model.partialText ?? typing.copy.text(.listening)) : typing.copy.text(.processing))
                .font(.keyboard(KeyStyle.barSize))
                .foregroundStyle(model.partialText == nil ? KeyStyle.secondaryLabel.color : KeyStyle.label.color)
                .lineLimit(1)
                .truncationMode(.head)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityLabel(model.partialText.map { typing.copy.language == .dutch ? "Voorbeeld: \($0)" : "Preview: \($0)" } ?? typing.copy.text(model.pad == .listening ? .listening : .processing))
        }
    }

    private func barIcon(_ systemName: String, active: Bool, label: String, hint: String = "", action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.keyboard(KeyStyle.barSize + 3))
                .foregroundStyle(active ? KeyStyle.accent.color : KeyStyle.secondaryLabel.color)
                .frame(width: controlWidth, height: barHeight)
                .contentShape(Rectangle())
        }
        .accessibilityLabel(label)
        .accessibilityHint(hint)
    }

    private var guidance: some View {
        HStack(alignment: .top, spacing: Brand.Spacing.s2) {
            VStack(alignment: .leading, spacing: Brand.Spacing.s1) {
                switch model.pad {
                case .needsFullAccess:
                    Text(typing.copy.text(.fullAccess))
                    Button(action: openSettings) {
                        Text(typing.copy.text(.settings)).frame(maxWidth: .infinity, minHeight: controlWidth, alignment: .leading).contentShape(Rectangle())
                    }
                case .failed(let message):
                    Text(message)
                    Button {
                        if model.recoveryAction == .recordAgain { model.recordAgain() } else { model.retry() }
                    } label: {
                        Text(model.primaryActionLabel).frame(maxWidth: .infinity, minHeight: controlWidth, alignment: .leading).contentShape(Rectangle())
                    }
                default:
                    Text(typing.copy.text(launchFailed ? .openPennelFallback : .startPennelHint))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button { showsHelp = false } label: {
                Image(systemName: "xmark").frame(width: controlWidth, height: controlWidth).contentShape(Rectangle())
            }
            .accessibilityLabel(typing.copy.text(.hideHelp))
        }
        .font(.keyboard(Brand.FontSize.content))
        .foregroundStyle(KeyStyle.label.color)
        .padding(.horizontal, Brand.Spacing.s2)
    }

    private func recovery(_ preview: String) -> some View {
        VStack(alignment: .leading, spacing: Brand.Spacing.s1) {
            Text(preview)
                .font(.keyboard(Brand.FontSize.content))
                .foregroundStyle(KeyStyle.secondaryLabel.color)
                .lineLimit(2)
            Button { model.insertLast() } label: {
                Text(typing.copy.text(.insertLast))
                    .font(.keyboard(Brand.FontSize.content, weight: .semibold))
                    .foregroundStyle(KeyStyle.accent.color)
                    .frame(maxWidth: .infinity, minHeight: controlWidth)
                    .contentShape(Rectangle())
            }
        }
        .padding(.horizontal, Brand.Spacing.s2)
    }

    private func microphoneTapped() {
        switch model.pad {
        case .sessionOff: launchPennel()
        case .needsFullAccess:
            showsHelp.toggle()
            feedback(.selection)
        default: showsTools = false; cancelWriting(); model.tapPill()
        }
    }

    /// Opens Pennel to start its bounded microphone session; the keyboard starts listening once the user swipes back.
    private func launchPennel() {
        let scheme = Bundle.main.object(forInfoDictionaryKey: "PennelURLScheme") as? String
        guard let url = KeyboardModel.launchURL(scheme: scheme), model.requestLaunch() else { return }
        feedback(.selection)
        let failed = {
            model.cancelLaunch()
            launchFailed = true
            showsHelp = true
            feedback(.error)
        }
        openURL(url) { accepted in
            Task { @MainActor in
                guard !accepted else { return }
                openPennel(url) { opened in if !opened { failed() } }
            }
        }
    }

    private var actionValue: String {
        switch model.pad {
        case .listening: typing.copy.text(.listening)
        case .sessionOff: typing.copy.text(.sessionOff)
        case .needsFullAccess: typing.copy.text(.fullAccess)
        default: ""
        }
    }

    private var suggestionRow: some View {
        HStack(spacing: Brand.Spacing.s2) {
            if let original = typing.correctionOriginal {
                Button(action: undoCorrection) {
                    HStack(spacing: Brand.Spacing.s1) {
                        Image(systemName: "arrow.uturn.backward")
                        Text(original).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, minHeight: controlWidth)
                }
                .accessibilityLabel(typing.copy.language == .dutch ? "Herstel \(original)" : "Restore \(original)")
            }
            if typing.suggestions.isEmpty, typing.correctionOriginal == nil {
                Spacer(minLength: 0)
            } else {
                ForEach(typing.suggestions, id: \.self) { suggestion in
                    Button { insertSuggestion(suggestion) } label: {
                        Text(suggestion)
                            .foregroundStyle(KeyStyle.label.color)
                            .frame(maxWidth: .infinity, minHeight: controlWidth)
                            .lineLimit(2)
                            .contentShape(Rectangle())
                    }
                    .accessibilityHint(typing.copy.text(.insertSuggestion))
                }
            }
        }
        .font(.keyboard(KeyStyle.barSize + 1))
        .frame(minHeight: controlWidth)
    }

    private var bottomRow: some View {
        HStack(spacing: KeyStyle.horizontalGap) {
            KeyboardSymbolLayerKey(label: activeLayer != .letters ? typing.copy.text(.letters) : typing.copy.text(.numbers),
                accessibilityLabel: typing.copy.text(activeLayer != .letters ? .showLetters : .showNumbers),
                inputTarget: letterInputTarget,
                begin: {
                    layerBeforeSlide = activeLayer
                    toolsBeforeSlide = showsTools
                    if showsTools && toolTab == .writing { showsTools = false; return false }
                    let sliding = activeLayer == .letters
                    setActiveLayer(sliding ? .numbers : .letters)
                    return sliding
                }, finish: { selected in
                    if toolsBeforeSlide && !showsTools { cancelWriting() }
                    if selected { setActiveLayer(.letters) } else { feedback(.keyPress) }
                }, cancel: {
                    showsTools = toolsBeforeSlide; setActiveLayer(layerBeforeSlide)
                }, insert: insertText, prepareFeedback: prepareFeedback)
                .frame(width: functionWidth, height: keyHeight)
            if showsGlobe {
                KeyboardGlobeKey(target: inputModeTarget, label: typing.copy.text(.nextKeyboard))
                    .frame(width: functionWidth, height: keyHeight)
            }
            if typing.showsURL {
                key(text: "/", width: functionWidth) { insertText("/") }
            }
            KeyboardSpaceKey(label: typing.copy.text(.space), language: typing.copy.language, action: insertSpace,
                moveCursor: { if !searchesTools { moveCursor($0) } }, prepareFeedback: prepareFeedback, feedback: feedback,
                allowsCursorMovement: !searchesTools)
                .frame(maxWidth: .infinity, minHeight: keyHeight)
            if typing.showsAt {
                key(text: "@", width: functionWidth, letter: true) { insertText("@") }
                key(text: ".", width: functionWidth, letter: true) { insertText(".") }
            } else if typing.showsURL {
                key(text: ".", width: functionWidth, letter: true) { insertText(".") }
                key(text: ".com", width: functionWidth * 1.3, letter: true) { insertText(".com") }
            }
            let returnKey = searchesTools ? KeyboardCopy.Key.returnKey : typing.returnKey
            key(text: typing.copy.text(returnKey), width: functionWidth * 2,
                enabled: searchesTools || typing.returnEnabled, prominent: Self.prominentReturnKeys.contains(returnKey)) { insertText("\n") }
        }
    }

    private static let prominentReturnKeys: Set<KeyboardCopy.Key> = [.go, .search, .send, .join, .route, .done, .continueKey]
    private var functionWidth: CGFloat { keyHeight + Brand.Spacing.s1 }

    private func key(text: String? = nil, systemImage: String? = nil, width: CGFloat? = nil, label: String? = nil, enabled: Bool = true,
                     letter: Bool = false, prominent: Bool = false, action: @escaping () -> Void) -> some View {
        let highlighted = prominent && enabled
        return Button(action: action) {
            Group {
                if let text {
                    Text(text).font(.keyboard(letter ? KeyStyle.letterSize - 2 : KeyStyle.functionSize)).lineLimit(1).minimumScaleFactor(0.7)
                } else if let systemImage {
                    Image(systemName: systemImage)
                }
            }
            .foregroundStyle(highlighted ? KeyStyle.accentText.color : enabled ? KeyStyle.label.color : KeyStyle.tertiaryLabel.color)
            .frame(maxWidth: .infinity, minHeight: keyHeight)
            .background((highlighted ? KeyStyle.accent : letter ? KeyStyle.letterKey : KeyStyle.functionKey).color,
                in: RoundedRectangle(cornerRadius: KeyStyle.keyRadius))
            .shadow(color: KeyStyle.keyShadow.color, radius: 0, x: 0, y: 1)
        }
        .buttonStyle(KeyboardPreparationButtonStyle(prepare: prepareFeedback))
        .frame(width: width)
        .accessibilityLabel(label ?? text ?? "")
        .disabled(!enabled)
    }

    private var searchesTools: Bool { showsTools && toolTab != .writing }
    private var activeLayer: KeyboardLayer { searchesTools ? toolTypingState.layer : typing.state.layer }

    private func setActiveLayer(_ layer: KeyboardLayer) {
        if searchesTools { toolTypingState.layer = layer } else { typing.state.layer = layer }
    }

    private func insertText(_ text: String) {
        if searchesTools {
            toolQuery = String((toolQuery + text).prefix(2_048))
            toolTypingState.insertedCharacter()
            feedback(.keyPress)
        } else { insert(text) }
    }

    private func deleteText() {
        if searchesTools { if !toolQuery.isEmpty { toolQuery.removeLast() } }
        else { delete() }
    }

    private func insertSpace() {
        if searchesTools { insertText(" ") } else { space() }
    }

    private func deletePreviousWord() {
        if searchesTools {
            while toolQuery.last?.isWhitespace == true { toolQuery.removeLast() }
            while let last = toolQuery.last, !last.isWhitespace { toolQuery.removeLast() }
        } else { deleteWord() }
    }
}
