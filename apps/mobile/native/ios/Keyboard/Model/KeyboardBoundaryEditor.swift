import PennelMobileKit
import UIKit

enum KeyboardBoundaryEditOutcome {
    case notApplicable
    case completed(KeyboardCorrectionTransaction)
    case interrupted(KeyboardCorrectionTransaction)

    var consumesKey: Bool {
        if case .notApplicable = self { return false }
        return true
    }

    var completedTransaction: KeyboardCorrectionTransaction? {
        if case .completed(let transaction) = self { return transaction }
        return nil
    }

    var interruptedTransaction: KeyboardCorrectionTransaction? {
        if case .interrupted(let transaction) = self { return transaction }
        return nil
    }
}

enum KeyboardPrefixEditOutcome {
    case notApplicable
    case completed
    case interrupted

    var isCompleted: Bool { if case .completed = self { return true }; return false }
    var isInterrupted: Bool { if case .interrupted = self { return true }; return false }
}

@MainActor
enum KeyboardBoundaryEditor {
    static func apply(original: String, replacement: String, delimiter: String,
                      proxy: any UITextDocumentProxy, lifetime: UUID) -> KeyboardBoundaryEditOutcome {
        let document = proxy.documentIdentifier
        let before = proxy.documentContextBeforeInput
        let after = proxy.documentContextAfterInput
        guard proxy.selectedText?.isEmpty != false,
              after?.first.map({ !$0.isLetter && !$0.isNumber }) ?? true,
              let transaction = KeyboardCorrectionTransaction(original: original, replacement: replacement, delimiter: delimiter,
                  before: before, after: after, document: document, lifetime: lifetime) else { return .notApplicable }
        switch replacePrefix(original, with: replacement + delimiter, proxy: proxy) {
        case .notApplicable: return .notApplicable
        case .completed: return .completed(transaction)
        case .interrupted: return .interrupted(transaction)
        }
    }

    static func undo(_ transaction: KeyboardCorrectionTransaction, proxy: any UITextDocumentProxy,
                     lifetime: UUID, generation: UInt64) -> KeyboardBoundaryEditOutcome {
        guard proxy.selectedText?.isEmpty != false,
              transaction.permits(before: proxy.documentContextBeforeInput, after: proxy.documentContextAfterInput,
                                  document: proxy.documentIdentifier, lifetime: lifetime, generation: generation) else { return .notApplicable }
        switch replacePrefix(transaction.replacement + transaction.delimiter, with: transaction.restoredText, proxy: proxy) {
        case .notApplicable: return .notApplicable
        case .completed: return .completed(transaction)
        case .interrupted: return .interrupted(transaction)
        }
    }

    static func replacePrefix(_ original: String, with replacement: String,
                              proxy: any UITextDocumentProxy) -> KeyboardPrefixEditOutcome {
        guard original.count <= 4_098, replacement.count <= 4_098,
              proxy.selectedText?.isEmpty != false,
              let before = proxy.documentContextBeforeInput, before.hasSuffix(original) else { return .notApplicable }
        let document = proxy.documentIdentifier
        let capturedBefore = String(before.suffix(4_224))
        let capturedAfter = proxy.documentContextAfterInput.map { String($0.prefix(96)) }
        func remainsAtTarget(deleted: Int) -> Bool {
            guard proxy.documentIdentifier == document, proxy.selectedText?.isEmpty != false,
                  let currentBefore = proxy.documentContextBeforeInput,
                  proxy.documentContextAfterInput.map({ String($0.prefix(96)) }) == capturedAfter else { return false }
            let visible = String(currentBefore.suffix(4_224))
            let expected = String(capturedBefore.dropLast(deleted))
            // A bounded host window may refill to its original length, but must retain every proven character.
            // Missing following context stays unknown; the visible prefix still anchors each mutation.
            guard visible.count <= capturedBefore.count, visible.count >= expected.count else { return false }
            return expected.isEmpty ? visible.isEmpty : visible.hasSuffix(expected)
        }
        for deleted in 0..<original.count {
            guard remainsAtTarget(deleted: deleted) else { return .interrupted }
            proxy.deleteBackward()
        }
        guard remainsAtTarget(deleted: original.count) else { return .interrupted }
        if !replacement.isEmpty { proxy.insertText(replacement) }
        return .completed
    }
}
