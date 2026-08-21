// "Hey Siri, talk to Henry" — App Shortcut that opens a live conversation.
//
// Phrases are registered by `PermagentShortcuts`. The hub's default persona
// is Henry; `\(.applicationName)` also matches "Talk to Permagent". Watch
// and iPhone both compile this file and each registers the shortcut for
// its own Siri.

import AppIntents

struct TalkToAgentIntent: AppIntent {
    static let title: LocalizedStringResource = "Talk to Henry"
    static var description: IntentDescription {
        IntentDescription("Start a live conversation with your agent.")
    }
    static var openAppWhenRun: Bool { true }

    func perform() async throws -> some IntentResult {
        await MainActor.run { AppRoute.shared.talk() }
        return .result()
    }
}

struct PermagentShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: TalkToAgentIntent(),
            phrases: [
                "Talk to \(.applicationName)",
                "Talk to Henry with \(.applicationName)",
                "Ask \(.applicationName)",
            ],
            shortTitle: "Talk to Henry",
            systemImageName: "waveform"
        )
    }
}
