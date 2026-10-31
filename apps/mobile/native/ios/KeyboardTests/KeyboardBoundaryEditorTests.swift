import Foundation
import PennelMobileKit
import Testing
import UIKit

@MainActor
private final class BoundaryTestDocument: NSObject, UITextDocumentProxy {
    var text: String
    var cursor: Int
    var followingContextIsUnavailable: Bool
    var beforeContextIsUnavailable = false
    var beforeContextLimit: Int?
    var selectedText: String?
    var documentIdentifier = UUID()
    var documentInputMode: UITextInputMode? { nil }
    var deletionCount = 0
    var afterDeletion: ((BoundaryTestDocument) -> Void)?

    init(_ text: String, followingContextIsUnavailable: Bool = true) {
        self.text = text
        cursor = text.count
        self.followingContextIsUnavailable = followingContextIsUnavailable
    }

    var documentContextBeforeInput: String? {
        guard !beforeContextIsUnavailable else { return nil }
        let visible = String(text.prefix(cursor))
        return beforeContextLimit.map { String(visible.suffix($0)) } ?? visible
    }

    var documentContextAfterInput: String? {
        followingContextIsUnavailable ? nil : String(text.dropFirst(cursor))
    }

    var hasText: Bool { !text.isEmpty }

    func insertText(_ text: String) {
        let position = self.text.index(self.text.startIndex, offsetBy: cursor)
        self.text.insert(contentsOf: text, at: position)
        cursor += text.count
    }

    func deleteBackward() {
        guard cursor > 0 else { return }
        let position = text.index(text.startIndex, offsetBy: cursor - 1)
        text.remove(at: position)
        cursor -= 1
        deletionCount += 1
        afterDeletion?(self)
    }

    func adjustTextPosition(byCharacterOffset offset: Int) { cursor = max(0, min(text.count, cursor + offset)) }
    func setMarkedText(_ markedText: String, selectedRange: NSRange) {}
    func unmarkText() {}
}

@MainActor
struct KeyboardBoundaryEditorTests {
    @Test func actual_controller_edit_path_corrects_and_undoes_with_nil_after_context() throws {
        let proxy = BoundaryTestDocument("Teh teh")
        let lifetime = UUID()
        let transaction = try #require(KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
            proxy: proxy, lifetime: lifetime).completedTransaction)
        #expect(proxy.text == "Teh the ")
        #expect(proxy.deletionCount == 3)
        let bound = try #require(transaction.bound(to: KeyboardInsertionContext(
            document: proxy.documentIdentifier, lifetime: lifetime, generation: 9)))
        #expect(KeyboardBoundaryEditor.undo(bound, proxy: proxy, lifetime: lifetime, generation: 9).completedTransaction != nil)
        #expect(proxy.text == "Teh teh ")
    }

    @Test func nil_after_is_not_assumed_to_mean_eof_and_following_text_is_not_deleted() throws {
        let proxy = BoundaryTestDocument("teh following")
        proxy.cursor = 3
        let transaction = try #require(KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
            proxy: proxy, lifetime: UUID()).completedTransaction)
        #expect(proxy.text == "the  following")
        #expect(transaction.original == "teh")
        #expect(proxy.deletionCount == 3)
    }

    @Test func known_following_letters_selection_and_unproven_prefixes_fail_without_deletion() {
        let proxy = BoundaryTestDocument("tehnical", followingContextIsUnavailable: false)
        proxy.cursor = 3
        #expect(!KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ", proxy: proxy, lifetime: UUID()).consumesKey)
        #expect(proxy.text == "tehnical")
        proxy.followingContextIsUnavailable = true
        proxy.cursor = proxy.text.count
        #expect(!KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ", proxy: proxy, lifetime: UUID()).consumesKey)
        proxy.text = "teh"
        proxy.cursor = 3
        proxy.selectedText = "teh"
        #expect(!KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ", proxy: proxy, lifetime: UUID()).consumesKey)
        proxy.selectedText = nil
        proxy.beforeContextIsUnavailable = true
        #expect(!KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ", proxy: proxy, lifetime: UUID()).consumesKey)
        #expect(proxy.deletionCount == 0)
    }

    @Test func undo_rejects_generation_document_and_context_availability_changes() throws {
        let proxy = BoundaryTestDocument("teh")
        let lifetime = UUID()
        let transaction = try #require(KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
            proxy: proxy, lifetime: lifetime).completedTransaction)
        let bound = try #require(transaction.bound(to: KeyboardInsertionContext(
            document: proxy.documentIdentifier, lifetime: lifetime, generation: 7)))
        let deletions = proxy.deletionCount
        #expect(!KeyboardBoundaryEditor.undo(bound, proxy: proxy, lifetime: lifetime, generation: 8).consumesKey)
        proxy.followingContextIsUnavailable = false
        #expect(!KeyboardBoundaryEditor.undo(bound, proxy: proxy, lifetime: lifetime, generation: 7).consumesKey)
        proxy.followingContextIsUnavailable = true
        proxy.documentIdentifier = UUID()
        #expect(!KeyboardBoundaryEditor.undo(bound, proxy: proxy, lifetime: lifetime, generation: 7).consumesKey)
        #expect(proxy.deletionCount == deletions)
        #expect(proxy.text == "the ")
    }

    @Test(arguments: [false, true])
    func interrupted_correction_consumes_space_without_writing_into_the_new_target(changeDocument: Bool) throws {
        let proxy = BoundaryTestDocument("teh")
        proxy.afterDeletion = { proxy in
            if changeDocument { proxy.documentIdentifier = UUID() }
            else { proxy.selectedText = "new" }
            proxy.text = "new target"
            proxy.cursor = 3
        }
        let outcome = KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ", proxy: proxy, lifetime: UUID())
        if !outcome.consumesKey { proxy.insertText(" ") }
        #expect(outcome.completedTransaction == nil)
        #expect(try #require(outcome.interruptedTransaction).restoredText == "teh ")
        #expect(proxy.text == "new target")
        #expect(proxy.deletionCount == 1)
    }

    @Test(arguments: [false, true])
    func interrupted_undo_consumes_delete_without_deleting_from_the_new_target(changeDocument: Bool) throws {
        let proxy = BoundaryTestDocument("teh")
        let lifetime = UUID()
        let transaction = try #require(KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
            proxy: proxy, lifetime: lifetime).completedTransaction)
        let bound = try #require(transaction.bound(to: KeyboardInsertionContext(
            document: proxy.documentIdentifier, lifetime: lifetime, generation: 5)))
        proxy.afterDeletion = { proxy in
            if changeDocument { proxy.documentIdentifier = UUID() }
            else { proxy.selectedText = "new" }
            proxy.text = "new target"
            proxy.cursor = 3
        }
        let outcome = KeyboardBoundaryEditor.undo(bound, proxy: proxy, lifetime: lifetime, generation: 5)
        if !outcome.consumesKey { proxy.deleteBackward() }
        #expect(outcome.completedTransaction == nil)
        #expect(try #require(outcome.interruptedTransaction).restoredText == "teh ")
        #expect(proxy.text == "new target")
        #expect(proxy.deletionCount == 4)
    }

    @Test func correction_rejects_same_document_cursor_move_to_an_identical_suffix() {
        let proxy = BoundaryTestDocument("teh xx te")
        proxy.cursor = 3
        proxy.afterDeletion = { proxy in if proxy.deletionCount == 1 { proxy.cursor = proxy.text.count } }
        let outcome = KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
                                                    proxy: proxy, lifetime: UUID())
        #expect(outcome.interruptedTransaction != nil)
        #expect(proxy.text == "te xx te")
        #expect(proxy.deletionCount == 1)
    }

    @Test func correction_rechecks_the_anchor_after_its_final_deletion() {
        let proxy = BoundaryTestDocument("teh xx")
        proxy.cursor = 3
        proxy.afterDeletion = { proxy in if proxy.deletionCount == 3 { proxy.cursor = proxy.text.count } }
        let outcome = KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
                                                    proxy: proxy, lifetime: UUID())
        #expect(outcome.interruptedTransaction != nil)
        #expect(proxy.text == " xx")
        #expect(proxy.deletionCount == 3)
    }

    @Test func undo_rejects_same_document_cursor_move_to_an_identical_suffix() throws {
        let proxy = BoundaryTestDocument("teh xx the")
        proxy.cursor = 3
        let lifetime = UUID()
        let transaction = try #require(KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
            proxy: proxy, lifetime: lifetime).completedTransaction)
        let bound = try #require(transaction.bound(to: KeyboardInsertionContext(
            document: proxy.documentIdentifier, lifetime: lifetime, generation: 5)))
        proxy.afterDeletion = { proxy in if proxy.deletionCount == 4 { proxy.cursor = proxy.text.count } }
        let outcome = KeyboardBoundaryEditor.undo(bound, proxy: proxy, lifetime: lifetime, generation: 5)
        #expect(outcome.interruptedTransaction != nil)
        #expect(proxy.text == "the xx the")
        #expect(proxy.deletionCount == 4)
    }

    @Test(arguments: [("te", "test "), ("replacement", "original"), ("word ", "")], [false, true])
    func manual_suggestion_writing_restore_and_word_delete_stop_on_target_change(
        edit: (String, String), changeDocument: Bool
    ) {
        let proxy = BoundaryTestDocument("prefix " + edit.0)
        proxy.afterDeletion = { proxy in
            if changeDocument { proxy.documentIdentifier = UUID() }
            else { proxy.selectedText = "new" }
            proxy.text = "new target"
            proxy.cursor = 3
        }
        let outcome = KeyboardBoundaryEditor.replacePrefix(edit.0, with: edit.1, proxy: proxy)
        #expect(outcome.isInterrupted)
        #expect(!outcome.isCompleted)
        #expect(proxy.text == "new target")
        #expect(proxy.deletionCount == 1)
    }

    @Test func prefix_edit_rejects_following_context_availability_changes_mid_deletion() {
        let proxy = BoundaryTestDocument("word")
        proxy.afterDeletion = { $0.followingContextIsUnavailable = false }
        #expect(KeyboardBoundaryEditor.replacePrefix("word", with: "", proxy: proxy).isInterrupted)
        #expect(proxy.text == "wor")
        #expect(proxy.deletionCount == 1)
    }

    @Test func bounded_context_refill_still_supports_correction_and_undo() throws {
        let prefix = String(repeating: "word ", count: 30)
        let proxy = BoundaryTestDocument(prefix + "teh")
        proxy.beforeContextLimit = 50
        let lifetime = UUID()
        let transaction = try #require(KeyboardBoundaryEditor.apply(original: "teh", replacement: "the", delimiter: " ",
            proxy: proxy, lifetime: lifetime).completedTransaction)
        #expect(proxy.text == prefix + "the ")
        let bound = try #require(transaction.bound(to: KeyboardInsertionContext(
            document: proxy.documentIdentifier, lifetime: lifetime, generation: 2)))
        #expect(KeyboardBoundaryEditor.undo(bound, proxy: proxy, lifetime: lifetime, generation: 2).completedTransaction != nil)
        #expect(proxy.text == prefix + "teh ")
    }

    @Test func next_word_insertion_and_prefix_only_deletion_use_the_shared_guard() {
        let proxy = BoundaryTestDocument("hello ")
        #expect(KeyboardBoundaryEditor.replacePrefix("", with: "world ", proxy: proxy).isCompleted)
        #expect(proxy.text == "hello world ")
        #expect(KeyboardBoundaryEditor.replacePrefix("world ", with: "", proxy: proxy).isCompleted)
        #expect(proxy.text == "hello ")
        #expect(proxy.deletionCount == 6)
    }

}
