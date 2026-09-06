import Foundation

/// Shared, testable layout/accessibility rules, not a second theme.
enum DesignPolicy {
    static let controlSize: CGFloat = 44
    static let pageInset: CGFloat = 20
    /// The compact in-page title band used by the tab surfaces. Keep this
    /// aligned with the Decisions header so titles do not jump between tabs.
    static let pageHeaderHorizontalPadding: CGFloat = 18
    static let pageHeaderTopPadding: CGFloat = 6
    static let pageHeaderBottomPadding: CGFloat = 8
    static let cardRadius: CGFloat = 24
    static let composerRadius: CGFloat = 28
    static let voiceIdentitySymbol = "person.crop.circle"
    static func voiceCaptionHeight(accessibilitySize: Bool) -> CGFloat {
        accessibilitySize ? 156 : 88
    }
    static func hubStatus(_ healthy: Bool?) -> String {
        switch healthy {
        case true?: "Hub online"
        case false?: "Hub unreachable"
        case nil: "Connecting to your hub"
        }
    }

    static func opaqueChrome(reduceTransparency: Bool, increasedContrast: Bool) -> Bool {
        reduceTransparency || increasedContrast
    }
}

#if DEBUG
/// Isolated visual QA, explicitly selected at process launch. Never paired or
/// shipped in release; screenshots are labeled and cannot perform actions.
enum DesignPreview {
    static func screen(arguments: [String]) -> String? {
        guard let value = arguments.first(where: { $0.hasPrefix("--design-preview=") })?
            .split(separator: "=", maxSplits: 1).last.map(String.init),
              ["chat", "voice", "models", "control"].contains(value) else { return nil }
        return value
    }
    static var screen: String? { screen(arguments: ProcessInfo.processInfo.arguments) }
    static var enabled: Bool { screen != nil }
}
#endif
