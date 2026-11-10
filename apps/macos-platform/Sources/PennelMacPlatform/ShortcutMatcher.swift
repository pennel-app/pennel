import Foundation

struct NativeShortcut: Codable, Equatable {
    var action: UInt32
    var code: Int64?
    var mods: [UInt64]
}
struct ShortcutTable: Codable, Equatable {
    var hold: NativeShortcut?
    var always: [NativeShortcut]
    var listening: [NativeShortcut]
}
enum ShortcutInput { case down, up, flags, mouse }
enum ShortcutSignal: Equatable { case down, up, interrupt, shift(Bool), action(UInt32) }
struct ShortcutDecision { var signals: [ShortcutSignal] = []; var withhold = false }

/// A pure matcher shared by the listen-only monitor and the swallowing tap. No product policy lives here.
struct ShortcutMatcher {
    private(set) var table: ShortcutTable
    private(set) var isHeld = false
    private(set) var listening = false
    private(set) var capturing = false
    private var flags: UInt64 = 0
    private var keys: Set<Int64> = []
    private var withheld: Set<Int64> = []
    private var captureDraining: Set<Int64> = []
    private var armed: Set<UInt32> = []
    init(table: ShortcutTable) { self.table = table }
    var needsSwallowingTap: Bool {
        capturing || !withheld.isEmpty || table.hold?.code != nil || !table.always.isEmpty ||
            (listening && table.listening.contains { $0.code != nil })
    }
    mutating func setTable(_ table: ShortcutTable) {
        self.table = table; isHeld = false; keys.removeAll(); armed.removeAll()
    }
    mutating func setListening(_ value: Bool) {
        if value != listening { armed.removeAll() }; listening = value
    }
    mutating func setCapture(_ value: Bool) {
        if capturing && !value { captureDraining = withheld }
        capturing = value; isHeld = false; keys.removeAll(); armed.removeAll()
    }
    private func family(_ mask: UInt64) -> UInt64 {
        switch mask {
        case 0x1, 0x2000: return 0x40000
        case 0x20, 0x40: return 0x80000
        case 0x8, 0x10: return 0x100000
        case 0x2, 0x4: return 0x20000
        default: return mask
        }
    }
    static func modifierMask(_ code: Int64) -> UInt64? {
        switch code {
        case 59: return 0x1; case 62: return 0x2000
        case 58: return 0x20; case 61: return 0x40
        case 55: return 0x8; case 54: return 0x10
        case 56: return 0x2; case 60: return 0x4
        case 63: return 0x800000
        default: return nil
        }
    }
    private func matches(_ binding: NativeShortcut, exact: Bool) -> Bool {
        guard binding.mods.allSatisfy({ flags & $0 == $0 }), binding.code.map({ keys.contains($0) }) ?? true else { return false }
        guard exact else { return true }
        let required = binding.mods.reduce(UInt64(0)) { $0 | family($1) }
        let held = isHeld ? table.hold?.mods.reduce(UInt64(0)) { $0 | family($1) } ?? 0 : 0
        // macOS synthesizes fn on arrows/function keys; it is meaningful only on modifier bindings.
        let relevant: UInt64 = binding.code == nil ? 0x9E0000 : 0x1E0000
        let allowed = required | held | (isHeld ? 0x20000 : 0)
        return flags & relevant & ~allowed == 0
    }
    mutating func route(_ input: ShortcutInput, code: Int64, flags nextFlags: UInt64, repeating: Bool = false) -> ShortcutDecision {
        if input == .mouse { armed.removeAll(); return ShortcutDecision() }
        let before = Set(table.listening.filter { $0.code == nil && matches($0, exact: true) }.map(\.action))
        flags = nextFlags
        let modifier = Self.modifierMask(code)
        let down = input == .down || (input == .flags && modifier.map { flags & $0 != 0 } == true)
        if input == .down { keys.insert(code) }
        if input == .up { keys.remove(code) }
        var result = ShortcutDecision(withhold: withheld.contains(code))
        if !down { withheld.remove(code); captureDraining.remove(code) }
        if capturing {
            if down { withheld.insert(code) }
            result.withhold = true; return result
        }
        if !captureDraining.isEmpty { return result }
        let actions = table.always + (listening ? table.listening : [])
        if input == .down { armed.removeAll() }
        var actionKey = false
        for binding in actions {
            if let key = binding.code {
                if input == .down && code == key && matches(binding, exact: true) {
                    result.withhold = true; withheld.insert(code); actionKey = true
                    if !repeating { result.signals.append(.action(binding.action)) }
                }
            } else if listening {
                let engaged = matches(binding, exact: true)
                if engaged && !before.contains(binding.action) { armed.insert(binding.action) }
                if !engaged && before.contains(binding.action) {
                    if armed.remove(binding.action) != nil && !down { result.signals.append(.action(binding.action)) }
                }
            }
        }
        if input == .flags && (code == 56 || code == 60) { result.signals.append(.shift(flags & 0x20000 != 0)) }
        if let hold = table.hold {
            if input == .down && code == hold.code && matches(hold, exact: false) {
                result.withhold = true; withheld.insert(code)
            }
            if input == .down && isHeld && hold.code == nil && !actionKey { result.signals.append(.interrupt) }
        }
        let engaged = table.hold.map { matches($0, exact: false) } ?? false
        if engaged != isHeld { isHeld = engaged; result.signals.append(engaged ? .down : .up) }
        return result
    }
}
