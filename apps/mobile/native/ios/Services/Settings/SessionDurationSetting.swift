import Foundation
import PennelMobileKit

extension SessionClock.Duration {
    static let storageKey = "sessionDurationSeconds"

    static func stored(in defaults: UserDefaults = AppPaths.defaults) -> SessionClock.Duration {
        SessionClock.Duration(rawValue: defaults.double(forKey: storageKey)) ?? .fiveMinutes
    }

    func store(in defaults: UserDefaults = AppPaths.defaults) {
        defaults.set(rawValue, forKey: Self.storageKey)
    }

    var label: String {
        switch self {
        case .fiveMinutes: Copy.Settings.sessionDuration5
        case .fifteenMinutes: Copy.Settings.sessionDuration15
        case .oneHour: Copy.Settings.sessionDuration60
        }
    }
}
