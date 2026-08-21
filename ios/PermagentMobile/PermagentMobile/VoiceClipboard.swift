// Voice clipboard — pasteboard write for a voice `{"type":"clipboard"}` frame.
//
// Why this file exists (2026-08-21): asking Henry to copy a land
// acknowledgement, then switching to Notes, pasted nothing. Two causes:
// the hub deferred the clipboard WebSocket frame until after TTS (so the
// write landed after Permagent was already backgrounded), and
// `UIPasteboard.general.string =` from the background is ignored. The
// daemon now sends the frame as soon as the tool runs; this helper is the
// phone side — a same-device write with an expiration, wrapped in a
// background task so a race to Notes still has a chance.

import UIKit

@MainActor
enum VoiceClipboard {
    /// How long Notes (and anything else) can paste after we copy. Ten
    /// minutes covers "I heard it, I switched, I pasted" without leaving
    /// the acknowledgement on the pasteboard all day.
    static let persistence: TimeInterval = 10 * 60

    /// Write `body` onto this phone's general pasteboard. Returns whether
    /// `changeCount` bumped — i.e. whether iOS actually accepted the write
    /// (backgrounded calls often don't). Callers show "Copied" only then.
    @discardableResult
    static func write(_ body: String) -> Bool {
        guard !body.isEmpty else { return false }

        let token = BackgroundToken()
        token.id = UIApplication.shared.beginBackgroundTask(withName: "voice-clipboard") {
            UIApplication.shared.endBackgroundTask(token.id)
            token.id = .invalid
        }

        let before = UIPasteboard.general.changeCount
        UIPasteboard.general.setItems(
            [[
                "public.utf8-plain-text": body,
                "public.plain-text": body,
            ]],
            options: [
                .localOnly: true,
                .expirationDate: Date().addingTimeInterval(persistence),
            ]
        )

        if token.id != .invalid {
            UIApplication.shared.endBackgroundTask(token.id)
            token.id = .invalid
        }

        // Don't read `.string` back — iOS 16+ treats that as a paste and can
        // return nil without a user gesture. changeCount bumps only when a
        // write actually landed, including from a brief background task.
        return UIPasteboard.general.changeCount > before
    }
}

/// UIKit's expiration handler is @Sendable; a local `var` cannot ride it.
private final class BackgroundToken: @unchecked Sendable {
    var id = UIBackgroundTaskIdentifier.invalid
}
