import PennelMobileKit
import SwiftUI
import UIKit

enum KeyboardToolTab: String, CaseIterable, Identifiable {
    case emoji, snippets, writing
    var id: String { rawValue }
    func title(_ language: KeyboardInterfaceLanguage) -> String {
        switch (language, self) {
        case (_, .emoji): "Emoji"
        case (.english, .snippets): "Snippets"
        case (.dutch, .snippets): "Tekstfragmenten"
        case (.english, .writing): "AI writing"
        case (.dutch, .writing): "AI schrijven"
        }
    }
}

struct KeyboardToolsPanel: View {
    let typing: KeyboardTypingModel
    @Binding var tab: KeyboardToolTab
    @Binding var query: String
    let insert: (String) -> Void
    let requestWriting: (KeyboardWritingAction) -> Void
    let applyWriting: () -> Void
    let restoreWriting: () -> Void
    let cancelWriting: () -> Void
    let saveSnippet: (String, String) -> Void
    let deleteSnippet: (KeyboardSnippet) -> Void
    let feedback: (FeedbackEvent) -> Void
    @State private var recentEmoji = KeyboardToolPreferences.recentEmoji(in: UserDefaults.standard)
    @State private var creatingSnippet = false
    @State private var snippetTitle = ""
    @State private var snippetText = ""
    @State private var editsTitle = true
    @State private var showsOriginal = false
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    private var language: KeyboardInterfaceLanguage { typing.copy.language }
    private var controlHeight: CGFloat { verticalSizeClass == .compact ? Brand.Spacing.s8 : Brand.Spacing.s10 + Brand.Spacing.s1 }
    private var isDutch: Bool { language == .dutch }

    var body: some View {
        VStack(spacing: Brand.Spacing.s1) {
            if creatingSnippet {
                snippetEditor
            } else {
                Picker(isDutch ? "Toetsenbordhulpmiddelen" : "Keyboard tools", selection: $tab) {
                    ForEach(KeyboardToolTab.allCases) { value in Text(value.title(language)).tag(value) }
                }
                .pickerStyle(.segmented)
                .onChange(of: tab) { previous, _ in query = ""; if previous == .writing { cancelWriting() } }
                if tab != .writing { searchBar }
                switch tab {
                case .emoji: emojiResults
                case .snippets: snippetResults
                case .writing: writingTools
                }
            }
        }
        .font(.figtree(Brand.FontSize.content))
        .foregroundStyle(Brand.foreground.color)
    }

    private var searchBar: some View {
        HStack(spacing: Brand.Spacing.s2) {
            Image(systemName: "magnifyingglass").accessibilityHidden(true)
            Text(query.isEmpty ? (isDutch ? "Typ om te zoeken…" : "Type to search…") : query)
                .foregroundStyle(query.isEmpty ? Brand.muted.color : Brand.foreground.color)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityLabel(isDutch ? "Zoekopdracht" : "Search query")
                .accessibilityValue(query)
            if !query.isEmpty {
                Button { query = ""; feedback(.selection) } label: {
                    Image(systemName: "xmark.circle.fill").frame(minWidth: controlHeight, minHeight: controlHeight)
                }
                .accessibilityLabel(isDutch ? "Wis zoekopdracht" : "Clear search")
            }
            if tab == .snippets {
                Button { beginSnippet() } label: { Image(systemName: "plus").frame(minWidth: controlHeight, minHeight: controlHeight) }
                    .accessibilityLabel(isDutch ? "Nieuw tekstfragment" : "New snippet")
            }
        }
        .padding(.horizontal, Brand.Spacing.s2)
        .frame(minHeight: controlHeight)
        .background(Brand.subtle.color, in: RoundedRectangle(cornerRadius: Brand.Radius.control))
    }

    private var emojiResults: some View {
        let results = KeyboardEmojiSearch.search(query, recent: recentEmoji)
        return ScrollView {
            if results.isEmpty {
                empty(isDutch ? "Geen emoji gevonden." : "No matching emoji.")
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: controlHeight))], spacing: Brand.Spacing.s1) {
                    ForEach(results) { emoji in
                        Button {
                            KeyboardToolPreferences.recordEmoji(emoji.symbol, in: UserDefaults.standard)
                            recentEmoji = KeyboardToolPreferences.recentEmoji(in: UserDefaults.standard)
                            insert(emoji.symbol)
                        } label: {
                            Text(emoji.symbol).font(.system(size: Brand.FontSize.onboarding))
                                .frame(maxWidth: .infinity, minHeight: controlHeight)
                                .contentShape(Rectangle())
                        }
                        .accessibilityLabel(emoji.label(language: language))
                    }
                }
            }
        }
        .scrollDismissesKeyboard(.never)
    }

    private var snippetResults: some View {
        let results = typing.snippets.filter {
            query.isEmpty || ($0.title + " " + $0.text).localizedCaseInsensitiveContains(query)
        }
        return ScrollView {
            if results.isEmpty {
                empty(typing.snippets.isEmpty ? (isDutch ? "Bewaar tekst die je vaak gebruikt met +." : "Save text you use often with +.") : (isDutch ? "Geen tekstfragmenten gevonden." : "No matching snippets."))
            } else {
                LazyVStack(spacing: Brand.Spacing.s1) {
                    ForEach(results) { snippet in
                        HStack(spacing: Brand.Spacing.s1) {
                            Button { insert(snippet.text) } label: {
                                VStack(alignment: .leading, spacing: Brand.Spacing.s1) {
                                    Text(snippet.title).font(.figtree(Brand.FontSize.content, weight: .semibold)).lineLimit(1)
                                    Text(snippet.text).foregroundStyle(Brand.muted.color).lineLimit(1)
                                }
                                .frame(maxWidth: .infinity, minHeight: controlHeight, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .accessibilityHint(isDutch ? "Voegt dit tekstfragment in." : "Inserts this snippet.")
                            Button { deleteSnippet(snippet) } label: {
                                Image(systemName: "trash").frame(width: controlHeight, height: controlHeight)
                            }
                            .accessibilityLabel(isDutch ? "Verwijder \(snippet.title)" : "Delete \(snippet.title)")
                        }
                    }
                }
                .padding(.horizontal, Brand.Spacing.s2)
            }
        }
    }

    private var snippetEditor: some View {
        VStack(spacing: Brand.Spacing.s1) {
            HStack {
                Button(isDutch ? "Annuleer" : "Cancel") { creatingSnippet = false; query = ""; feedback(.selection) }
                    .frame(minHeight: controlHeight)
                Spacer()
                Button(isDutch ? "Bewaar" : "Save") {
                    let title = editsTitle ? query : snippetTitle
                    let text = editsTitle ? snippetText : query
                    saveSnippet(title.trimmingCharacters(in: .whitespacesAndNewlines), text)
                    creatingSnippet = false; query = ""
                }
                .frame(minHeight: controlHeight)
                .disabled((editsTitle ? query : snippetTitle).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (editsTitle ? snippetText : query).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            editorField(isDutch ? "Naam" : "Name", value: editsTitle ? query : snippetTitle, active: editsTitle) {
                if !editsTitle { snippetText = String(query.prefix(2_048)); query = snippetTitle; editsTitle = true }
            }
            editorField(isDutch ? "Tekst" : "Text", value: editsTitle ? snippetText : query, active: !editsTitle) {
                if editsTitle { snippetTitle = String(query.prefix(64)); query = snippetText; editsTitle = false }
            }
        }
        .onChange(of: query) { _, value in
            let limit = editsTitle ? 64 : 2_048
            let bounded = String(value.prefix(limit))
            let normalized = editsTitle ? bounded.replacingOccurrences(of: "\n", with: " ") : bounded
            if value != normalized { query = normalized }
        }
    }

    private func editorField(_ title: String, value: String, active: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: Brand.Spacing.s2) {
                Text(title).font(.figtree(Brand.FontSize.content, weight: .semibold))
                    .foregroundStyle(active ? Brand.accent.color : Brand.muted.color)
                Text(value.isEmpty ? (isDutch ? "Typ hieronder…" : "Type below…") : value)
                    .lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, Brand.Spacing.s2)
            .frame(minHeight: controlHeight)
            .background(Brand.subtle.color, in: RoundedRectangle(cornerRadius: Brand.Radius.control))
        }
        .accessibilityLabel(title)
        .accessibilityValue(value)
        .accessibilityAddTraits(active ? .isSelected : [])
    }

    private var writingTools: some View {
        VStack(alignment: .leading, spacing: Brand.Spacing.s1) {
            if let preview = typing.writingPreview {
                ScrollView {
                    Text(showsOriginal ? (typing.writingOriginal ?? preview) : preview)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel(isDutch ? (showsOriginal ? "Origineel" : "Voorbeeld") : (showsOriginal ? "Original" : "Preview"))
                }
                HStack {
                    Button(isDutch ? "Vervang selectie" : "Replace selection", action: applyWriting)
                        .frame(minHeight: controlHeight).disabled(!typing.writingCanApply)
                    Spacer()
                    if typing.writingCanRestore {
                        Button(isDutch ? "Herstel origineel" : "Restore original", action: restoreWriting)
                            .frame(minHeight: controlHeight)
                    }
                }
                if let original = typing.writingOriginal {
                    HStack {
                        Button(isDutch ? (showsOriginal ? "Toon resultaat" : "Toon origineel") : (showsOriginal ? "Show result" : "Show original")) {
                            showsOriginal.toggle(); feedback(.selection)
                        }
                        .frame(minHeight: controlHeight)
                        Spacer()
                        Button(isDutch ? "Kopieer origineel" : "Copy original") {
                            UIPasteboard.general.string = original; feedback(.selection)
                        }
                        .frame(minHeight: controlHeight)
                    }
                }
            } else {
                Text(typing.selectedText.map { String($0.prefix(160)) } ?? (isDutch ? "Selecteer tekst in je app om die te bewerken." : "Select text in your app to edit it."))
                    .foregroundStyle(Brand.muted.color).lineLimit(2)
                if typing.writingIsProcessing {
                    HStack {
                        ProgressView()
                        Text(isDutch ? "Tekst wordt verwerkt…" : "Working on your text…")
                        Spacer()
                        Button(isDutch ? "Annuleer" : "Cancel", action: cancelWriting).frame(minHeight: controlHeight)
                    }
                } else {
                    LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: Brand.Spacing.s1) {
                        ForEach(KeyboardWritingAction.allCases, id: \.self) { action in
                            Button(action.title(language: language)) { requestWriting(action) }
                                .frame(maxWidth: .infinity, minHeight: controlHeight)
                                .disabled(!typing.writingAvailable || typing.selectedText?.isEmpty != false)
                        }
                    }
                }
            }
            if let status = typing.writingStatus {
                Text(status).font(.figtree(Brand.FontSize.body)).foregroundStyle(Brand.muted.color).lineLimit(2)
            }
        }
        .padding(.horizontal, Brand.Spacing.s2)
    }

    private func beginSnippet() {
        creatingSnippet = true; editsTitle = true; snippetTitle = ""
        snippetText = String((typing.selectedText ?? "").prefix(2_048)); query = ""
        feedback(.selection)
    }

    private func empty(_ text: String) -> some View {
        Text(text).foregroundStyle(Brand.muted.color).frame(maxWidth: .infinity, minHeight: controlHeight)
    }
}
