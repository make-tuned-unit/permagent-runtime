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
