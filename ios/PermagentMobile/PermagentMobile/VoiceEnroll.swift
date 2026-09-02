// VoiceEnroll — prompts and quiet re-arm policy for learned speaker identity.
//
// This is a GATE, not a better ear. Kitchen music dies in VAD (VoiceVAD) and
// Voice Isolation; these three sentences train a print on the hub so a radio
// host or another talker does not start an agent turn. The hub stores a JSON
// vector at ~/.permagent/data/voice_print.json, never a WAV.
//
// iOS is the enrollment UI. Setup has its own screen and never occupies the
// conversation Orb; after onboarding it lives under Control > Voice identity.
//
// Strings MUST match crates/goose-server/src/voice/speaker_print.rs PROMPTS.

import Foundation

enum VoiceEnroll {
    static let need = 3
    static let prompts = [
        "What's on my board?",
        "This is the voice I want you to answer.",
        "Tell me something interesting.",
    ]

    static func prompt(have: Int) -> String? {
        guard have >= 0, have < prompts.count else { return nil }
        return prompts[have]
    }

    /// Enrollment is an automatic, pause-ended capture even though the setup
    /// screen disables ordinary hands-free conversation. Gating every VAD
    /// step on `handsFree` left the first enrollment take recording forever:
    /// no silence event could send Stop, so sentence two never arrived.
    ///
    /// `isListening` is what keeps the setup screen a control surface rather
    /// than a conversation one: the VAD runs only INSIDE a take the hub has
    /// asked for, never from `.ready`, so ambient speech still cannot open a
    /// turn there.
    static func shouldDriveVAD(
        handsFree: Bool,
        enrolling: Bool,
        isListening: Bool
    ) -> Bool {
        handsFree || (enrolling && isListening)
    }

    /// Open an enrollment take: stamp the VAD's turn clocks for a turn begun
    /// OUTSIDE the VAD, exactly as push-to-talk's `beginTurn()` does.
    ///
    /// Enrollment takes are opened by hub status (`enroll_status` / `idle`),
    /// never by the VAD's own `.ready` onset detector — the setup screen goes
    /// straight from `.ready` to `.listening` — so nothing else ever stamps
    /// `turnStart`. Left unstamped it holds whatever the last turn left
    /// behind, which on this screen is nothing at all: the max-turn cap then
    /// measures from the 1970 epoch and ends the take on its first frame.
    /// Every enrollment mic-reopen path must come through here.
    static func openTake(_ vad: inout VoiceVAD, now: TimeInterval) {
        vad.noteTurnBegan(at: now)
    }
}

/// A rejected background talker often keeps speaking. Reopening VAD on the
/// next frame immediately starts the same bad turn again, so learned-speaker
/// rejection requires roughly half a second of quiet before hands-free can
/// arm. Push-to-talk remains available.
struct VoiceIdentityQuietGate {
    static let quietRms: Float = 0.0045
    static let quietFramesNeeded = 6

    private(set) var locked = false
    private var quietFrames = 0

    mutating func lock() {
        locked = true
        quietFrames = 0
    }

    /// Returns true while hands-free VAD must remain suppressed.
    mutating func observe(rms: Float) -> Bool {
        guard locked else { return false }
        if rms < Self.quietRms {
            quietFrames += 1
            if quietFrames >= Self.quietFramesNeeded {
                locked = false
                quietFrames = 0
            }
        } else {
            quietFrames = 0
        }
        return locked
    }
}
