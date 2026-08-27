// VoiceEnroll — orb prompts and chrome for hub speaker-print enrollment (N3).
//
// This is a GATE, not a better ear. Kitchen music dies in VAD (VoiceVAD) and
// Voice Isolation; these three sentences train a print on the hub so a radio
// host or another talker does not start an agent turn. The hub stores a JSON
// vector at ~/.permagent/data/voice_print.json, never a WAV.
//
// iOS is the enrollment UI. Watch has no /voice socket of its own (the phone
// hops). Desktop Command Center shares the same hub print and fails OPEN when
// none exists — there is no enroll UI there.
//
// Strings MUST match crates/goose-server/src/voice/speaker_print.rs PROMPTS.

import Foundation

enum VoiceEnroll {
    static let need = 3
    static let prompts = [
        "What's on my board?",
        "Henry, I'm in the kitchen.",
        "Tell me something interesting.",
    ]

    static func prompt(have: Int) -> String? {
        guard have >= 0, have < prompts.count else { return nil }
        return prompts[have]
    }

    /// Pronunciation teach wins the orb slot if both are somehow set.
    static func orbText(teachWord: String?, enrollPrompt: String?) -> String? {
        if let word = teachWord, !word.isEmpty { return word }
        if let prompt = enrollPrompt, !prompt.isEmpty { return prompt }
        return nil
    }
}
