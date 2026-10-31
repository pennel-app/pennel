import PennelMobileKit
import SwiftUI

struct KeyboardRoot: View {
    let model: KeyboardModel
    let typing: KeyboardTypingModel
    let layout: KeyboardLayout
    let showsGlobe: Bool
    let inputModeTarget: KeyboardInputModeTarget
    let openSettings: () -> Void
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
    @State private var showsLetters = true
    @State private var showsHelp = false
    @State private var showsTools = false
    @State private var toolTab = KeyboardToolTab.emoji
    @State private var toolQuery = ""
    @State private var toolTypingState = KeyboardTypingState()
    @State private var layerBeforeSlide = KeyboardLayer.letters
    @State private var lettersBeforeSlide = true
    @State private var toolsBeforeSlide = false
    @State private var letterInputTarget = KeyboardLetterInputTarget()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    @ScaledMetric(relativeTo: .body) private var scaledKeyHeight = Brand.Spacing.s10 + Brand.Spacing.s1
    private let controlWidth = Brand.Spacing.s10 + Brand.Spacing.s1
    private var keyHeight: CGFloat {
        if verticalSizeClass == .compact { return max(Brand.Spacing.s8, min(scaledKeyHeight, Brand.Spacing.s10)) }
        return max(controlWidth, min(scaledKeyHeight, Brand.Spacing.s10 + Brand.Spacing.s8))
    }

    var body: some View {
        VStack(spacing: Brand.Spacing.s1) {
            if showsLetters { toolbar }
            #if PENNEL_KEYBOARD_FEASIBILITY
            if Bundle.main.bundleIdentifier == "app.pennel.ios.preview.keyboard" {
                HStack(spacing: Brand.Spacing.s2) {
                    Text("QA · Process routing only")
                        .font(.figtree(Brand.FontSize.body))
                        .foregroundStyle(Brand.muted.color)
                    Spacer()
                    KeyboardRecordingIntentProbeButton()
                        .frame(minHeight: controlWidth)
                        .accessibilityHint("Tests intent routing. No microphone capture is started.")
                }
                .font(.figtree(Brand.FontSize.content, weight: .semibold))
                .padding(.horizontal, Brand.Spacing.s2)
            }
            #endif
            if showsLetters {
                if showsHelp { guidance }
                if let preview = model.lastResultPreview, model.pad != .listening, model.pad != .processing { recovery(preview) }
                if model.pad == .listening {
                    Text(model.partialText ?? " ")
                        .font(.figtree(Brand.FontSize.content))
                        .foregroundStyle(Brand.muted.color)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, minHeight: controlWidth, maxHeight: controlWidth, alignment: .leading)
                        .accessibilityLabel(typing.copy.language == .dutch ? "Voorbeeld: \(model.partialText ?? "")" : "Preview: \(model.partialText ?? "")")
                        .accessibilityHidden(model.partialText == nil)
                }
                if showsTools {
                    KeyboardToolsPanel(typing: typing, tab: $toolTab, query: $toolQuery, insert: insertToolText,
                        requestWriting: requestWriting, applyWriting: applyWriting, restoreWriting: restoreWriting,
                        cancelWriting: cancelWriting, saveSnippet: saveSnippet, deleteSnippet: deleteSnippet, feedback: feedback)
                        .frame(height: keyHeight * (toolTab == .writing ? 5 : 3) + Brand.Spacing.s1 * 2)
                } else if typing.suggestionsEnabled || typing.correctionOriginal != nil { suggestionRow }
                if !showsTools || toolTab != .writing {
                    LetterPad(layout: layout, typing: typing, insert: insertText, delete: deleteText, feedback: feedback,
                        prepareFeedback: prepareFeedback, deleteWord: deletePreviousWord, swipe: swipe,
                        inputTarget: letterInputTarget, allowSwipe: !searchesTools,
                        recordTouch: { if !searchesTools { recordTouch($0) } },
                        stateOverride: searchesTools ? $toolTypingState : nil)
                }
            } else {
                DictationPad(model: model, openSettings: openSettings, prepareFeedback: prepareFeedback)
            }
            bottomRow
        }
        .padding(.horizontal, Brand.Spacing.s2)
        .padding(.vertical, Brand.Spacing.s2)
        .background(Brand.surface.color)
        .buttonStyle(KeyboardPreparationButtonStyle(prepare: prepareFeedback))
        .fixedSize(horizontal: false, vertical: true)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { heightChanged($0) }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.15), value: showsHelp)
        .onChange(of: model.pad) { _, pad in
            if pad == .ready || pad == .listening || pad == .processing { showsHelp = false }
            if case .failed = pad { showsHelp = true }
        }
    }

    private var toolbar: some View {
        HStack(spacing: Brand.Spacing.s2) {
            Button { showsLetters = false; showsTools = false; cancelWriting(); feedback(.selection) } label: {
                Text("Pennel").font(.figtree(Brand.FontSize.content, weight: .semibold))
                    .foregroundStyle(Brand.foreground.color)
                    .frame(minWidth: controlWidth, minHeight: keyHeight)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel(typing.copy.text(.showDictation))
            HStack(spacing: Brand.Spacing.s2) {
                Image(systemName: model.pad == .listening ? "stop.fill" : model.pad == .processing ? "ellipsis" : "mic.fill")
                Text(status).lineLimit(1)
            }
            .font(.figtree(Brand.FontSize.body, weight: .medium))
            .foregroundStyle(Brand.pillText.color)
            .frame(maxWidth: .infinity, minHeight: keyHeight)
            .padding(.horizontal, Brand.Spacing.s3)
            .background(Brand.pillBackground.color, in: Capsule())
            .accessibilityHidden(true)
            .overlay {
                KeyboardDictationGesture(label: model.primaryActionLabel,
                    hint: model.pad == .sessionOff ? typing.copy.text(.manualStart) : "",
                    value: actionValue, enabled: model.pad != .processing, holdEnabled: model.pad == .ready,
                    tap: microphoneTapped, holdBegan: { showsTools = false; cancelWriting(); model.holdBegan() }, holdEnded: model.holdEnded,
                    prepareFeedback: prepareFeedback)
            }
            if model.pad == .listening || model.pad == .processing {
                Button { model.cancel() } label: {
                    Image(systemName: "xmark.circle.fill").frame(width: controlWidth, height: keyHeight).contentShape(Rectangle())
                }
                .accessibilityLabel(typing.copy.text(.cancel))
            }
            Button(action: toggleSuggestions) {
                Image(systemName: typing.suggestionsEnabled ? "textformat.abc" : "textformat.abc.dottedunderline")
                    .foregroundStyle(typing.suggestionsEnabled ? Brand.accent.color : Brand.muted.color)
                    .frame(width: controlWidth, height: keyHeight)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel(typing.copy.text(typing.suggestionsEnabled ? .suggestions : .suggestionsOff))
            .accessibilityHint(typing.copy.text(.suggestionsPrivacy))
            Button {
                showsTools.toggle(); toolQuery = ""
                if !showsTools { cancelWriting() }
                feedback(.selection)
            } label: {
                Image(systemName: showsTools ? "keyboard" : "square.grid.2x2")
                    .foregroundStyle(showsTools ? Brand.accent.color : Brand.muted.color)
                    .frame(width: controlWidth, height: keyHeight)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel(typing.copy.language == .dutch ? (showsTools ? "Toon toetsenbord" : "Emoji, tekstfragmenten en schrijven") : (showsTools ? "Show keyboard" : "Emoji, snippets and writing"))
        }
        .frame(minHeight: keyHeight)
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
                    Text(typing.copy.text(.manualStart))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button { showsHelp = false } label: {
                Image(systemName: "xmark").frame(width: controlWidth, height: controlWidth).contentShape(Rectangle())
            }
            .accessibilityLabel(typing.copy.text(.hideHelp))
        }
        .font(.figtree(Brand.FontSize.content))
        .foregroundStyle(Brand.foreground.color)
        .padding(.horizontal, Brand.Spacing.s2)
    }

    private func recovery(_ preview: String) -> some View {
        VStack(alignment: .leading, spacing: Brand.Spacing.s1) {
            Text(preview)
                .font(.figtree(Brand.FontSize.content))
                .foregroundStyle(Brand.muted.color)
                .lineLimit(2)
            Button { model.insertLast() } label: {
                Text(typing.copy.text(.insertLast))
                    .font(.figtree(Brand.FontSize.content, weight: .semibold))
                    .foregroundStyle(Brand.accent.color)
                    .frame(maxWidth: .infinity, minHeight: controlWidth)
                    .contentShape(Rectangle())
            }
        }
        .padding(.horizontal, Brand.Spacing.s2)
    }

    private func microphoneTapped() {
        switch model.pad {
        case .sessionOff, .needsFullAccess:
            showsHelp.toggle()
            feedback(.selection)
        default: showsTools = false; cancelWriting(); model.tapPill()
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

    private var status: String {
        switch model.pad {
        case .ready: typing.copy.text(.ready)
        case .listening: typing.copy.text(.listening)
        case .processing: typing.copy.text(.processing)
        case .needsFullAccess: typing.copy.text(.setupDictation)
        case .sessionOff: typing.copy.text(.sessionOff)
        case .failed: model.primaryActionLabel
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
                Text(typing.copy.text(.suggestionsPrivacy))
                    .foregroundStyle(Brand.muted.color)
                    .frame(maxWidth: .infinity)
            } else {
                ForEach(typing.suggestions, id: \.self) { suggestion in
                    Button { insertSuggestion(suggestion) } label: {
                        Text(suggestion)
                            .foregroundStyle(Brand.foreground.color)
                            .frame(maxWidth: .infinity, minHeight: controlWidth)
                            .lineLimit(2)
                            .contentShape(Rectangle())
                    }
                    .accessibilityHint(typing.copy.text(.insertSuggestion))
                }
            }
        }
        .font(.figtree(Brand.FontSize.content, weight: .medium))
        .frame(minHeight: controlWidth)
    }

    private var bottomRow: some View {
        HStack(spacing: 5) {
            if showsGlobe {
                KeyboardGlobeKey(target: inputModeTarget, label: typing.copy.text(.nextKeyboard))
                    .frame(width: controlWidth, height: keyHeight)
            }
            KeyboardSymbolLayerKey(label: !showsLetters || activeLayer != .letters ? typing.copy.text(.letters) : typing.copy.text(.numbers),
                accessibilityLabel: typing.copy.text(!showsLetters ? .showTyping : activeLayer != .letters ? .showLetters : .showNumbers),
                inputTarget: letterInputTarget,
                begin: {
                    layerBeforeSlide = activeLayer
                    lettersBeforeSlide = showsLetters; toolsBeforeSlide = showsTools
                    if !showsLetters || (showsTools && toolTab == .writing) {
                        showsLetters = true; showsTools = false; return false
                    }
                    let sliding = activeLayer == .letters
                    setActiveLayer(sliding ? .numbers : .letters)
                    return sliding
                }, finish: { selected in
                    if toolsBeforeSlide && !showsTools { cancelWriting() }
                    if selected { setActiveLayer(.letters) } else { feedback(.keyPress) }
                }, cancel: {
                    showsLetters = lettersBeforeSlide; showsTools = toolsBeforeSlide; setActiveLayer(layerBeforeSlide)
                }, insert: insertText, prepareFeedback: prepareFeedback)
                .frame(width: controlWidth + Brand.Spacing.s3, height: keyHeight)
            if showsLetters { key(text: typing.showsAt ? "@" : ",", width: controlWidth - Brand.Spacing.s2) { insertText(typing.showsAt ? "@" : ",") } }
            KeyboardSpaceKey(label: typing.copy.text(.space), language: typing.copy.language, action: insertSpace,
                moveCursor: { if !searchesTools { moveCursor($0) } }, prepareFeedback: prepareFeedback, feedback: feedback,
                allowsCursorMovement: !searchesTools)
                .frame(maxWidth: .infinity, minHeight: keyHeight)
            if showsLetters { key(text: typing.showsURL ? ".com" : ".", width: typing.showsURL ? controlWidth + Brand.Spacing.s2 : controlWidth - Brand.Spacing.s2) { insertText(typing.showsURL ? ".com" : ".") } }
            if !showsLetters {
                KeyboardDeleteKey(label: typing.copy.text(.delete), action: deleteText, feedback: { feedback(.keyPress) },
                    prepareFeedback: prepareFeedback, deleteWord: deletePreviousWord).frame(width: controlWidth, height: keyHeight)
            }
            key(text: typing.copy.text(searchesTools ? .returnKey : typing.returnKey), width: controlWidth + Brand.Spacing.s5,
                enabled: searchesTools || typing.returnEnabled) { insertText("\n") }
        }
    }

    private func key(text: String? = nil, systemImage: String? = nil, width: CGFloat? = nil, label: String? = nil, enabled: Bool = true, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Group {
                if let text {
                    Text(text).font(.figtree(Brand.FontSize.body, weight: .medium)).lineLimit(1).minimumScaleFactor(0.7)
                } else if let systemImage {
                    Image(systemName: systemImage)
                }
            }
            .foregroundStyle(enabled ? Brand.foreground.color : Brand.disabledText.color)
            .frame(maxWidth: .infinity, minHeight: keyHeight)
            .background(Brand.subtle.color, in: RoundedRectangle(cornerRadius: Brand.Radius.keycap))
        }
        .buttonStyle(KeyboardPreparationButtonStyle(prepare: prepareFeedback))
        .frame(width: width)
        .accessibilityLabel(label ?? text ?? "")
        .disabled(!enabled)
    }

    private var searchesTools: Bool { showsLetters && showsTools && toolTab != .writing }
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
