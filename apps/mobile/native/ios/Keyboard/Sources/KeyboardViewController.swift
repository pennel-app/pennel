import PennelMobileKit
import SwiftUI
import UIKit

final class KeyboardViewController: UIInputViewController {
    private var host: UIHostingController<KeyboardRoot>?
    private var model: KeyboardModel?
    private var typing: KeyboardTypingModel?
    private var timer: Timer?
    private var observations: [HandoffSignal.Observation] = []
    private let inputModeTarget = KeyboardInputModeTarget()
    private let lifetime = UUID()
    private var generation: UInt64 = 0
    private var isVisible = false
    private var installedFullAccess = false
    private var heightConstraint: NSLayoutConstraint?
    private var contentHeight: CGFloat?
    private var feedbackController: KeyboardFeedbackController?
    private var writingClient: KeyboardWritingClient?
    private var swipeTask: Task<Void, Never>?
    private var correction: KeyboardCorrectionTransaction?
    private var writingSelection: KeyboardSelectionSnapshot?
    private var writingRestoration: KeyboardWritingRestoration?
    private var writingRequest = UUID()
    private var performingEdit = false
    private var permitsWritingInField = false
    private var settingsRevision: String?

    override func viewDidLoad() {
        super.viewDidLoad()
        inputModeTarget.controller = self
        let height = view.heightAnchor.constraint(equalToConstant: Brand.Spacing.s10 * 6 + Brand.Spacing.s5)
        height.priority = .defaultHigh
        height.isActive = true
        heightConstraint = height
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        isVisible = true
        install()
        model?.refresh()
        // Darwin signals are the fast path; the timer also catches a heartbeat that simply stops (5 s) and animates the bars.
        observations = [HandoffSignal.session, .result].map { signal in
            signal.observe { [weak self] in DispatchQueue.main.async { self?.model?.refresh() } }
        }
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                if self.hasFullAccess != self.installedFullAccess { self.install() }
                self.model?.refresh()
                self.writingClient?.refresh()
                self.updateWritingAvailability()
                self.refreshSettingsIfNeeded()
                self.updateHeight()
            }
        }
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        invalidateContext()
        isVisible = false
        typing?.cancelPrediction()
        feedbackController?.stop()
        feedbackController = nil
        writingClient?.cancel()
        writingClient = nil
        swipeTask?.cancel()
        swipeTask = nil
        correction = nil
        writingSelection = nil
        writingRestoration = nil
        writingRequest = UUID()
        timer?.invalidate()
        timer = nil
        observations = []
        host?.willMove(toParent: nil)
        host?.view.removeFromSuperview()
        host?.removeFromParent()
        host = nil
        typing = nil
        model = nil
        contentHeight = nil
    }

    override func textWillChange(_ textInput: (any UITextInput)?) { invalidateContext() }

    override func textDidChange(_ textInput: (any UITextInput)?) {
        invalidateContext()
        if !performingEdit {
            typing?.resetTouches()
            validateTransientEdits()
        }
        updateTypingContext()
    }

    override func selectionWillChange(_ textInput: (any UITextInput)?) { invalidateContext() }

    override func selectionDidChange(_ textInput: (any UITextInput)?) {
        invalidateContext()
        if !performingEdit {
            typing?.resetTouches()
            validateTransientEdits()
        }
        updateTypingContext()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        updateHeight()
    }

    private func install() {
        model?.contextDidChange()
        typing?.cancelPrediction()
        feedbackController?.stop()
        writingClient?.cancel()
        swipeTask?.cancel()
        correction = nil
        writingSelection = nil
        writingRestoration = nil
        writingRequest = UUID()
        installedFullAccess = hasFullAccess
        let defaults = preferenceDefaults
        let language = KeyboardPreferences.interfaceLanguage(in: defaults)
        primaryLanguage = KeyboardPreferences.typingLanguages(in: defaults).first == "nl" ? "nl-NL" : "en-US"
        let copy = KeyboardCopy(language: language)
        KeyboardPreferences.mirrorKeyboardFeedback(from: defaults, to: UserDefaults.standard)
        KeyboardPreferences.mirrorTypingConfiguration(from: defaults, to: UserDefaults.standard)
        if hasFullAccess { KeyboardPreferences.mergeExtensionVocabulary(from: UserDefaults.standard, to: defaults) }
        let typing = KeyboardTypingModel(copy: copy, suggestionsEnabled: suggestionsPreference(in: defaults),
                                         defaults: defaults, learningDefaults: hasFullAccess ? defaults : UserDefaults.standard,
                                         writableDefaults: hasFullAccess ? defaults : UserDefaults.standard)
        self.typing = typing
        feedbackController = KeyboardFeedbackController(view: view, preferences: { [weak self] in
            KeyboardFeedbackPreferences(in: self?.preferenceDefaults ?? UserDefaults.standard)
        })
        if hasFullAccess {
            writingClient = HandoffStore.groupDirectory.map(KeyboardWritingClient.init(directory:))
        } else {
            writingClient = nil
        }
        UserDefaults.standard.set(typing.suggestionsEnabled, forKey: KeyboardPreferences.suggestionsKey)
        KeyboardPreferences.publish(dictionary: [], interfaceLanguage: language.rawValue, in: UserDefaults.standard)
        settingsRevision = defaults?.string(forKey: KeyboardPreferences.typingRevisionKey)
        let directory = hasFullAccess ? HandoffStore.groupDirectory : nil
        let store = HandoffStore(directory: directory ?? FileManager.default.temporaryDirectory.appendingPathComponent("pennel-no-group"))
        let model = KeyboardModel(
            hasFullAccess: hasFullAccess,
            client: HandoffClient(store: store),
            context: { [weak self] in self?.insertionContext }, copy: copy,
            onFeedback: { [weak self] in self?.playFeedback($0) },
            onResult: { [weak self] in self?.insert($0, typingFeedback: false) })
        self.model = model
        let layout = KeyboardLayout.stored(in: defaults)
        layout.store(in: UserDefaults.standard)
        if hasFullAccess { KeyboardFullAccess.markSeen(in: KeyboardLayout.groupDefaults) }
        // iOS 27 can raise an Objective-C exception querying the switch-key requirement during host reconnection.
        // Keep the public globe control available without reading that transitional document interface.
        let root = KeyboardRoot(
            model: model, typing: typing, layout: layout, showsGlobe: true,
            inputModeTarget: inputModeTarget,
            openSettings: { [weak self] in
                guard let self else { return }
                AppOpener.openSettings(from: extensionContext)
            },
            insert: { [weak self] in self?.insert($0) },
            delete: { [weak self] in self?.delete() },
            space: { [weak self] in self?.insertSpace() },
            insertSuggestion: { [weak self] in self?.insertSuggestion($0) },
            toggleSuggestions: { [weak self] in self?.toggleSuggestions() },
            feedback: { [weak self] in self?.playFeedback($0) },
            heightChanged: { [weak self] height in
                guard let self else { return }
                self.contentHeight = height
                self.updateHeight()
            },
            prepareFeedback: { [weak self] in self?.feedbackController?.prepareTyping() },
            moveCursor: { [weak self] in self?.moveCursor($0) },
            deleteWord: { [weak self] in self?.deleteWord() },
            swipe: { [weak self] in self?.insertSwipe($0) },
            insertToolText: { [weak self] in self?.insertToolText($0) },
            requestWriting: { [weak self] in self?.requestWriting($0) },
            applyWriting: { [weak self] in self?.applyWriting() },
            restoreWriting: { [weak self] in self?.restoreWriting() },
            cancelWriting: { [weak self] in self?.cancelWriting() },
            undoCorrection: { [weak self] in _ = self?.undoCorrection(feedback: true) },
            saveSnippet: { [weak self] title, text in self?.typing?.saveSnippet(title: title, text: text) },
            deleteSnippet: { [weak self] in self?.typing?.deleteSnippet($0) },
            recordTouch: { [weak self] in self?.typing?.recordTouch($0) })
        updateTypingContext()
        loadSuggestions()
        if let host {
            host.rootView = root
            updateHeight()
            return
        }
        let host = UIHostingController(rootView: root)
        addChild(host)
        host.view.translatesAutoresizingMaskIntoConstraints = false
        host.view.backgroundColor = .clear
        view.addSubview(host.view)
        NSLayoutConstraint.activate([
            host.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            host.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            host.view.topAnchor.constraint(equalTo: view.topAnchor),
            host.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        host.didMove(toParent: self)
        self.host = host
    }

    private var insertionContext: KeyboardInsertionContext? {
        guard isVisible else { return nil }
        return KeyboardInsertionContext(document: textDocumentProxy.documentIdentifier, lifetime: lifetime, generation: generation)
    }

    private func invalidateContext() {
        generation &+= 1
        model?.contextDidChange()
    }

    private var preferenceDefaults: UserDefaults? {
        guard let shared = KeyboardLayout.groupDefaults else { return UserDefaults.standard }
        let keys = [KeyboardPreferences.languageKey, KeyboardPreferences.typingLanguagesKey,
                    KeyboardPreferences.suggestionsKey, KeyboardPreferences.feedbackKey]
        return keys.contains(where: { shared.object(forKey: $0) != nil }) ? shared : UserDefaults.standard
    }

    private func suggestionsPreference(in defaults: UserDefaults?) -> Bool {
        KeyboardPreferences.effectiveSuggestionsEnabled(shared: defaults, local: UserDefaults.standard)
    }

    private func refreshSettingsIfNeeded() {
        let revision = preferenceDefaults?.string(forKey: KeyboardPreferences.typingRevisionKey)
        guard revision != settingsRevision else { return }
        install()
    }

    private func performEdit(_ action: () -> Void) {
        performingEdit = true
        invalidateContext()
        action()
        performingEdit = false
        validateTransientEdits()
        updateTypingContext()
    }

    private func clearCorrection() {
        correction = nil
        typing?.correctionOriginal = nil
    }

    private func validateTransientEdits() {
        let proxy = textDocumentProxy
        if let correction, !correction.permits(before: proxy.documentContextBeforeInput, after: proxy.documentContextAfterInput,
                                               document: proxy.documentIdentifier, lifetime: lifetime, generation: generation) { clearCorrection() }
        if let selection = writingSelection,
           !selection.permits(selectedText: proxy.selectedText, before: proxy.documentContextBeforeInput,
                              after: proxy.documentContextAfterInput, document: proxy.documentIdentifier, lifetime: lifetime) {
            typing?.writingCanApply = false
            if typing?.writingIsProcessing == true { cancelWriting() }
        }
        if let restoration = writingRestoration {
            typing?.writingCanRestore = restoration.permits(before: proxy.documentContextBeforeInput,
                after: proxy.documentContextAfterInput, selectedText: proxy.selectedText,
                document: proxy.documentIdentifier, lifetime: lifetime)
        }
    }

    private func applyBoundaryReplacement(delimiter: String) -> Bool {
        #if PENNEL_KEYBOARD_FEASIBILITY
        writeContextProbe(stage: "boundary")
        #endif
        guard let typing,
              let replacement = typing.replacementAtBoundary(before: textDocumentProxy.documentContextBeforeInput,
                                                               after: textDocumentProxy.documentContextAfterInput)
        else { return false }
        clearCorrection()
        var outcome = KeyboardBoundaryEditOutcome.notApplicable
        performEdit {
            outcome = KeyboardBoundaryEditor.apply(original: replacement.original, replacement: replacement.text,
                delimiter: delimiter, proxy: textDocumentProxy, lifetime: lifetime)
        }
        if let interrupted = outcome.interruptedTransaction {
            typing.retainInterruptedEdit(interrupted.restoredText)
            return outcome.consumesKey
        }
        guard let transaction = outcome.completedTransaction else { return outcome.consumesKey }
        guard let context = insertionContext, let bound = transaction.bound(to: context) else {
            typing.retainInterruptedEdit(transaction.restoredText)
            return outcome.consumesKey
        }
        typing.resetTouches()
        correction = bound
        typing.correctionOriginal = replacement.original
        playFeedback(.keyPress)
        return true
    }

    @discardableResult
    private func undoCorrection(feedback: Bool) -> Bool {
        #if PENNEL_KEYBOARD_FEASIBILITY
        writeContextProbe(stage: "undo")
        #endif
        guard isVisible, let correction,
              correction.permits(before: textDocumentProxy.documentContextBeforeInput, after: textDocumentProxy.documentContextAfterInput,
                                 document: textDocumentProxy.documentIdentifier, lifetime: lifetime, generation: generation)
        else { clearCorrection(); return false }
        clearCorrection()
        let originGeneration = generation
        var outcome = KeyboardBoundaryEditOutcome.notApplicable
        performEdit {
            outcome = KeyboardBoundaryEditor.undo(correction, proxy: textDocumentProxy, lifetime: lifetime, generation: originGeneration)
        }
        if let interrupted = outcome.interruptedTransaction {
            typing?.retainInterruptedEdit(interrupted.restoredText)
            return outcome.consumesKey
        }
        guard outcome.completedTransaction != nil else { return outcome.consumesKey }
        typing?.reject(original: correction.original, replacement: correction.replacement)
        typing?.resetTouches()
        typing?.learnAccepted(correction.original, previous: KeyboardTypingContext.previousToken(before: textDocumentProxy.documentContextBeforeInput))
        updateTypingContext()
        if feedback { playFeedback(.selection) }
        return true
    }

    private func insert(_ text: String, typingFeedback: Bool = true) {
        guard isVisible else { return }
        if typingFeedback, text.count == 1, let character = text.first,
           ".,!?;:\n".contains(character), applyBoundaryReplacement(delimiter: text) {
            typing?.state.insertedCharacter()
            return
        }
        clearCorrection()
        performEdit {
            typing?.state.insertedCharacter()
            textDocumentProxy.insertText(text)
        }
        if typingFeedback { playFeedback(.keyPress) }
        if text.count != 1 || text.first?.isLetter != true { typing?.resetTouches() }
    }

    private func delete() {
        guard isVisible else { return }
        if undoCorrection(feedback: false) { return }
        clearCorrection()
        typing?.resetTouches()
        typing?.state.resetSpacing()
        performEdit { textDocumentProxy.deleteBackward() }
    }

    private func insertSpace() {
        guard isVisible, let typing else { return }
        let edit = typing.state.spaceEdit(before: textDocumentProxy.documentContextBeforeInput, at: Date(), allowsPeriod: typing.allowsPeriod)
        if edit == .insert(" "), applyBoundaryReplacement(delimiter: " ") { return }
        clearCorrection()
        typing.resetTouches()
        var outcome = KeyboardPrefixEditOutcome.completed
        performEdit {
            switch edit {
            case .insert(let text): textDocumentProxy.insertText(text)
            case .replacePreviousCharacter(let text):
                outcome = KeyboardBoundaryEditor.replacePrefix(" ", with: text, proxy: textDocumentProxy)
            }
        }
        if outcome.isInterrupted { typing.retainInterruptedEdit(" ") }
        if outcome.isCompleted { playFeedback(.keyPress) }
    }

    private func insertSuggestion(_ suggestion: String) {
        guard isVisible, let typing, typing.suggestions.contains(suggestion) else { return }
        let prefix = KeyboardTypingContext.token(before: textDocumentProxy.documentContextBeforeInput)
        guard prefix == typing.currentPrefix,
              KeyboardTypingContext.permitsManualReplacement(token: prefix, before: textDocumentProxy.documentContextBeforeInput,
                                                             after: textDocumentProxy.documentContextAfterInput)
        else { updateTypingContext(); return }
        clearCorrection()
        let transaction = KeyboardCorrectionTransaction(original: prefix, replacement: suggestion, delimiter: " ",
            before: textDocumentProxy.documentContextBeforeInput, after: textDocumentProxy.documentContextAfterInput,
            document: textDocumentProxy.documentIdentifier, lifetime: lifetime)
        let previous = KeyboardTypingContext.previousToken(before: textDocumentProxy.documentContextBeforeInput)
        let originDocument = textDocumentProxy.documentIdentifier
        var outcome = KeyboardPrefixEditOutcome.notApplicable
        performEdit {
            outcome = KeyboardBoundaryEditor.replacePrefix(prefix, with: suggestion + " ", proxy: textDocumentProxy)
        }
        guard outcome.isCompleted else {
            if outcome.isInterrupted { typing.retainInterruptedEdit(prefix + " ") }
            return
        }
        guard textDocumentProxy.documentIdentifier == originDocument else {
            typing.retainInterruptedEdit(prefix + " ")
            return
        }
        typing.state.insertedCharacter()
        typing.learnAccepted(suggestion, previous: previous)
        typing.resetTouches()
        correction = insertionContext.flatMap { transaction?.bound(to: $0) }
        typing.correctionOriginal = correction?.original
        playFeedback(.keyPress)
    }

    private func insertToolText(_ text: String) {
        guard !text.isEmpty, text.count <= 4_096 else { return }
        insert(text)
    }

    private func moveCursor(_ offset: Int) {
        guard isVisible, offset != 0 else { return }
        clearCorrection()
        typing?.resetTouches()
        let before = textDocumentProxy.documentContextBeforeInput
        let after = textDocumentProxy.documentContextAfterInput
        performEdit { textDocumentProxy.adjustTextPosition(byCharacterOffset: max(-64, min(offset, 64))) }
        if before != textDocumentProxy.documentContextBeforeInput || after != textDocumentProxy.documentContextAfterInput {
            playFeedback(.selection)
        }
    }

    private func deleteWord() {
        guard isVisible else { return }
        if textDocumentProxy.selectedText?.isEmpty == false { delete(); return }
        let count = KeyboardTypingContext.wordDeletionCount(before: textDocumentProxy.documentContextBeforeInput)
        guard count > 0 else { return }
        clearCorrection()
        typing?.resetTouches()
        typing?.state.resetSpacing()
        let removed = String((textDocumentProxy.documentContextBeforeInput ?? "").suffix(count))
        var outcome = KeyboardPrefixEditOutcome.notApplicable
        performEdit { outcome = KeyboardBoundaryEditor.replacePrefix(removed, with: "", proxy: textDocumentProxy) }
        if outcome.isInterrupted { typing?.retainInterruptedEdit(removed) }
        if outcome.isCompleted { playFeedback(.selection) }
    }

    private func insertSwipe(_ gesture: KeyboardSwipeGesture) {
        guard isVisible, let typing, typing.swipeEnabled,
              KeyboardTypingContext.token(before: textDocumentProxy.documentContextBeforeInput).isEmpty,
              textDocumentProxy.selectedText?.isEmpty != false, let origin = insertionContext else { return }
        swipeTask?.cancel()
        swipeTask = Task { @MainActor [weak self, weak typing] in
            guard let typing else { return }
            let candidates = await typing.decodeSwipe(gesture)
            guard !Task.isCancelled, let self, self.isVisible, self.typing === typing,
                  self.insertionContext == origin, let first = candidates.first else { return }
            let word: String
            switch typing.state.shift {
            case .off: word = first
            case .once: word = first.prefix(1).uppercased() + first.dropFirst()
            case .locked: word = first.uppercased()
            }
            self.insert(word + " ")
        }
    }

    private func requestWriting(_ action: KeyboardWritingAction) {
        guard isVisible, let typing else { return }
        guard permitsWritingInField else {
            typing.writingStatus = localized("Writing actions are unavailable in this field.", "Schrijfacties zijn niet beschikbaar in dit veld.")
            return
        }
        guard let selected = textDocumentProxy.selectedText.map({ String($0.prefix(4_097)) }), !selected.isEmpty else {
            typing.writingStatus = localized("Select text in the app first.", "Selecteer eerst tekst in de app.")
            return
        }
        guard selected.count <= 4_096 else {
            typing.writingStatus = localized("Select up to 4,096 characters.", "Selecteer maximaal 4.096 tekens.")
            return
        }
        guard let writingClient, writingClient.isAvailable else {
            typing.writingStatus = localized("Open Pennel and choose a writing provider to use writing actions.",
                                            "Open Pennel en kies een schrijfprovider om schrijfacties te gebruiken.")
            return
        }
        cancelWriting()
        let request = UUID()
        writingRequest = request
        let snapshot = KeyboardSelectionSnapshot(text: selected, before: textDocumentProxy.documentContextBeforeInput,
            after: textDocumentProxy.documentContextAfterInput, document: textDocumentProxy.documentIdentifier, lifetime: lifetime)
        writingSelection = snapshot
        writingRestoration = nil
        typing.writingOriginal = selected
        typing.writingPreview = nil
        typing.writingCanApply = false
        typing.writingCanRestore = false
        typing.writingIsProcessing = true
        typing.writingStatus = localized("Preparing a preview…", "Voorbeeld voorbereiden…")
        writingClient.request(action: action, text: selected) { [weak self, weak typing] result in
            guard let self, self.isVisible, let typing, self.typing === typing, self.writingRequest == request else { return }
            typing.writingIsProcessing = false
            switch result {
            case .success(let text):
                typing.writingPreview = text
                typing.writingCanApply = snapshot.permits(selectedText: self.textDocumentProxy.selectedText,
                    before: self.textDocumentProxy.documentContextBeforeInput, after: self.textDocumentProxy.documentContextAfterInput,
                    document: self.textDocumentProxy.documentIdentifier, lifetime: self.lifetime)
                typing.writingStatus = typing.writingCanApply
                    ? self.localized("Review before replacing the selection.", "Controleer voordat je de selectie vervangt.")
                    : self.localized("The selection changed. Your preview is retained.", "De selectie is veranderd. Je voorbeeld blijft bewaard.")
            case .failure:
                typing.writingStatus = self.localized("Writing action unavailable. Check the provider in Pennel.",
                                                      "Schrijfactie niet beschikbaar. Controleer de provider in Pennel.")
            }
        }
    }

    private func applyWriting() {
        guard isVisible, let typing, let text = typing.writingPreview, !text.isEmpty, text.count <= 4_096,
              let selection = writingSelection,
              selection.permits(selectedText: textDocumentProxy.selectedText, before: textDocumentProxy.documentContextBeforeInput,
                                after: textDocumentProxy.documentContextAfterInput, document: textDocumentProxy.documentIdentifier, lifetime: lifetime)
        else { typing?.writingCanApply = false; return }
        clearCorrection()
        performEdit { textDocumentProxy.insertText(text) }
        writingRestoration = selection.restoration(replacement: text)
        writingSelection = nil
        typing.writingCanApply = false
        typing.writingCanRestore = writingRestoration?.permits(before: textDocumentProxy.documentContextBeforeInput,
            after: textDocumentProxy.documentContextAfterInput, selectedText: textDocumentProxy.selectedText,
            document: textDocumentProxy.documentIdentifier, lifetime: lifetime) == true
        typing.writingStatus = localized("Selection replaced.", "Selectie vervangen.")
        playFeedback(.selection)
    }

    private func restoreWriting() {
        guard isVisible, let restoration = writingRestoration,
              restoration.permits(before: textDocumentProxy.documentContextBeforeInput, after: textDocumentProxy.documentContextAfterInput,
                                   selectedText: textDocumentProxy.selectedText, document: textDocumentProxy.documentIdentifier, lifetime: lifetime)
        else { typing?.writingCanRestore = false; return }
        writingRestoration = nil
        var outcome = KeyboardPrefixEditOutcome.notApplicable
        performEdit {
            outcome = KeyboardBoundaryEditor.replacePrefix(restoration.replacement, with: restoration.original,
                                                           proxy: textDocumentProxy)
        }
        typing?.writingCanRestore = false
        guard outcome.isCompleted else {
            if outcome.isInterrupted { typing?.retainInterruptedEdit(restoration.original) }
            typing?.writingStatus = localized("The insertion point changed. The original is in temporary snippets.",
                                              "Het invoegpunt is veranderd. Het origineel staat bij tijdelijke fragmenten.")
            return
        }
        typing?.writingStatus = localized("Original text restored.", "Oorspronkelijke tekst hersteld.")
        playFeedback(.selection)
    }

    private func cancelWriting() {
        writingRequest = UUID()
        writingClient?.cancel()
        typing?.writingIsProcessing = false
        typing?.writingCanApply = false
        typing?.writingStatus = nil
    }

    private func localized(_ english: String, _ dutch: String) -> String {
        typing?.copy.language == .dutch ? dutch : english
    }

    #if PENNEL_KEYBOARD_FEASIBILITY
    private func writeContextProbe(stage: String) {
        guard Bundle.main.bundleIdentifier == "app.pennel.ios.preview.keyboard", hasFullAccess,
              let directory = HandoffStore.groupDirectory, let typing else { return }
        let proxy = textDocumentProxy
        let before = proxy.documentContextBeforeInput
        let after = proxy.documentContextAfterInput
        var value = typing.boundaryProbe(before: before, after: after)
        value["stage"] = stage
        value["at"] = Date().timeIntervalSince1970
        value["generation"] = generation
        value["fullAccess"] = hasFullAccess
        value["hostExplicitlyDisablesAutocorrection"] = proxy.autocorrectionType == .no
        value["selectionLength"] = proxy.selectedText.map { $0.prefix(4_097).count } ?? 0
        value["undoAvailable"] = correction != nil
        value["undoPermitted"] = correction?.permits(before: before, after: after,
            document: proxy.documentIdentifier, lifetime: lifetime, generation: generation) == true
        do {
            let folder = directory.deletingLastPathComponent().appendingPathComponent("Library/Caches/PennelKeyboardQA", isDirectory: true)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
            try data.write(to: folder.appendingPathComponent("keyboard-context-probe.json"), options: .atomic)
        } catch {}
    }
    #endif

    private func updateWritingAvailability() {
        guard let typing else { return }
        typing.writingAvailable = permitsWritingInField && hasFullAccess && writingClient?.isAvailable == true
        guard !typing.writingIsProcessing, typing.writingPreview == nil else { return }
        if !hasFullAccess {
            typing.writingStatus = localized("Enable Full Access for writing actions. Typing works without it.",
                                            "Zet Volledige toegang aan voor schrijfacties. Typen werkt ook zonder.")
        } else if !permitsWritingInField {
            typing.writingStatus = localized("Writing actions are unavailable in this field.", "Schrijfacties zijn niet beschikbaar in dit veld.")
        } else if !typing.writingAvailable {
            typing.writingStatus = localized("Open Pennel and choose a writing provider to use writing actions.",
                                            "Open Pennel en kies een schrijfprovider om schrijfacties te gebruiken.")
        } else if typing.selectedText?.isEmpty != false {
            typing.writingStatus = localized("Select text in the app first.", "Selecteer eerst tekst in de app.")
        } else { typing.writingStatus = nil }
    }

    private func toggleSuggestions() {
        guard let typing else { return }
        typing.suggestionsEnabled.toggle()
        playFeedback(.selection)
        let defaults = preferenceDefaults
        KeyboardPreferences.setSuggestionsEnabled(typing.suggestionsEnabled, shared: defaults, local: UserDefaults.standard, canWriteShared: hasFullAccess)
        loadSuggestions()
        updateTypingContext()
        updateHeight()
    }

    private func loadSuggestions() {
        guard let typing else { return }
        guard typing.suggestionsEnabled else {
            typing.setLexicon([], words: [])
            return
        }
        let words = KeyboardPreferences.dictionary(in: preferenceDefaults)
        requestSupplementaryLexicon(completion: KeyboardLexiconCompletion.make { [weak self, weak typing] entries in
            guard let self, self.isVisible, let typing, self.typing === typing, typing.suggestionsEnabled else { return }
            typing.setLexicon(entries, words: words)
            self.updateTypingContext()
        })
    }

    private func updateTypingContext() {
        guard let typing else { return }
        let kind = textDocumentProxy.keyboardType ?? .default
        typing.showsAt = kind == .emailAddress
        typing.showsURL = kind == .URL
        typing.returnEnabled = textDocumentProxy.enablesReturnKeyAutomatically != true || textDocumentProxy.hasText
        typing.allowsPeriod = ![UIKeyboardType.emailAddress, .URL, .numberPad, .decimalPad, .phonePad, .asciiCapableNumberPad].contains(kind)
        let capitalization: KeyboardCapitalization
        switch textDocumentProxy.autocapitalizationType ?? .sentences {
        case .none: capitalization = .none
        case .words: capitalization = .words
        case .allCharacters: capitalization = .allCharacters
        case .sentences: capitalization = .sentences
        @unknown default: capitalization = .none
        }
        switch textDocumentProxy.returnKeyType ?? .default {
        case .go: typing.returnKey = .go
        case .search, .google, .yahoo: typing.returnKey = .search
        case .send: typing.returnKey = .send
        case .next: typing.returnKey = .next
        case .done: typing.returnKey = .done
        case .join: typing.returnKey = .join
        case .route: typing.returnKey = .route
        case .continue: typing.returnKey = .continueKey
        default: typing.returnKey = .returnKey
        }
        if [.numberPad, .decimalPad, .asciiCapableNumberPad, .numbersAndPunctuation].contains(kind), typing.state.layer == .letters {
            typing.state.layer = .numbers
        }
        let protectedTypes: [UITextContentType] = [.password, .newPassword, .oneTimeCode, .creditCardNumber, .telephoneNumber, .postalCode]
        let nameTypes: [UITextContentType] = [.name, .givenName, .middleName, .familyName, .nickname, .username, .organizationName]
        let contentType = textDocumentProxy.textContentType
        let allowsAssistance = !protectedTypes.contains(where: { $0 == contentType })
            && ![UIKeyboardType.numberPad, .decimalPad, .phonePad, .asciiCapableNumberPad].contains(kind)
        let prose = ![UIKeyboardType.emailAddress, .URL, .webSearch].contains(kind)
            && !nameTypes.contains(where: { $0 == contentType })
        permitsWritingInField = allowsAssistance
        typing.selectedText = textDocumentProxy.selectedText.map { String($0.prefix(4_097)) }
        updateWritingAvailability()
        typing.update(before: textDocumentProxy.documentContextBeforeInput, capitalization: capitalization,
                      assistanceAllowed: allowsAssistance,
                      autocorrectionAllowed: prose && textDocumentProxy.autocorrectionType != .no,
                      selectionIsEmpty: textDocumentProxy.selectedText?.isEmpty != false)
        if textDocumentProxy.documentContextAfterInput?.first.map({ $0.isLetter || $0.isNumber }) == true { typing.suggestions = [] }
    }

    private func updateHeight() {
        let row = max(Brand.Spacing.s10 + Brand.Spacing.s1,
                      min(UIFontMetrics.default.scaledValue(for: Brand.Spacing.s10 + Brand.Spacing.s1), Brand.Spacing.s10 + Brand.Spacing.s8))
        let suggestions = typing?.suggestionsEnabled == true ? Brand.Spacing.s10 + Brand.Spacing.s1 : 0
        let recovery = model?.lastResult == nil ? 0 : Brand.Spacing.s5
        let desired = contentHeight ?? (row * 5 + Brand.Spacing.s2 * 4 + suggestions + recovery)
        if abs((heightConstraint?.constant ?? 0) - desired) > 0.5 { heightConstraint?.constant = desired }
    }

    private func playFeedback(_ event: FeedbackEvent) {
        guard isVisible else { return }
        feedbackController?.play(event)
    }
}
