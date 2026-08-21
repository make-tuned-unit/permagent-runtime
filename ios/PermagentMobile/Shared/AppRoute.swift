// Deep links inside the installed apps — Siri, widgets, and in-app chrome
// share one place to say "open a live conversation".
//
// Phone: full-screen VoiceView (hands-free, already listening).
// Watch: the orb chat screen (the Watch has no /voice socket of its own).

import SwiftUI

@MainActor
final class AppRoute: ObservableObject {
    static let shared = AppRoute()

    /// iPhone / iPad — present VoiceView.
    @Published var showVoice = false
    /// Watch — push WatchChatView.
    @Published var showWatchChat = false

    func talk() {
        #if os(watchOS)
        showWatchChat = true
        #else
        showVoice = true
        #endif
    }
}
