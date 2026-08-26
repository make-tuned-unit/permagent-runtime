// VoiceOrbDrive — the orb's motion targets, extracted so they can be tested
// without a Canvas.
//
// The contract the orb is meant to keep (user report, 2026-08-25): while THEY
// speak the orb pulses with their voice; while the AGENT speaks it goes dynamic
// and changes shape with the speech. The three states must read as three
// different KINDS of motion, not one shape at three speeds — the convention every
// shipped voice orb follows, and it is what makes a long think tolerable.
//
// Bands map onto geometry in VoiceOrbView: `low` → `amp`, the magnitude of the
// noise-field displacement (the SHAPE change); `mid` → `spin`, the rotation
// rate; `high` → surface shimmer/brightness.
//
// LEVEL SCALES. Both callers hand in a 0…1 number, but they are not the same
// number. The mic path is `min(1, rms * 12)` and the playback tap is
// `min(1, rms * 2)` (VoiceView.swift:427, :584); ordinary speech on either path
// lands around 0.10–0.60. That is why the old listening floor was wrong: its
// synthetic breath sat at 0.16–0.27, i.e. ON TOP of ordinary speech, so
// `max(breath, level)` returned the breath almost every frame and the orb
// pulsed to a metronome rather than to the speaker. The floor now sits at
// 0.05–0.08, safely BELOW speech, so the pulse is the voice and the floor only
// catches true silence.

import Foundation

enum VoiceOrbDrive {
    struct Bands: Equatable {
        var low: Double
        var mid: Double
        var high: Double
    }

    /// Ceiling for a listening/speaking floor. Any synthetic idle motion must
    /// stay under this so real speech always wins the `max`. Ordinary speech
    /// on both level scales starts around 0.10.
    static let floorCeiling = 0.10

    /// Band targets for one frame. `level` is 0…1 (mic RMS while listening,
    /// TTS playback RMS while speaking).
    static func bands(
        thinking: Bool,
        listening: Bool,
        speaking: Bool,
        level: Double,
        t: Double
    ) -> Bands {
        let level = level.isFinite ? min(1.5, max(0, level)) : 0

        if thinking {
            // Turning, not breathing. `low` is held almost flat so the surface
            // does not pulse — the noise field still drifts on its own clock —
            // while `mid` is pinned high so the orb visibly ROTATES. With a
            // multi-second wait in front of every reply this is the state the
            // user stares at, and it has to say "working", not "idle" and not
            // "listening".
            let drift = 0.005 * sin(t * 0.9)
            return Bands(low: 0.085 + drift, mid: 0.42 + 0.05 * sin(t * 0.7), high: 0.02)
        }

        if listening {
            // The pulse IS the microphone. The floor only shows through in
            // real silence.
            let floor = 0.05 + 0.03 * (0.5 + 0.5 * sin(t * 2.2))
            return Bands(
                low: max(floor, level * 1.35),
                mid: max(floor * 0.7, level * 0.9 * (0.7 + 0.3 * sin(t * 11))),
                high: max(floor * 0.4, level * 0.7 * (0.5 + 0.5 * sin(t * 19 + 1.7)))
            )
        }

        if speaking {
            // Shape follows the TTS envelope. The residual is a floor, not a
            // blend: a quiet syllable must not be papered over by a sine, or
            // the orb stops being the agent's voice and becomes decoration.
            let residual = 0.09
            let drive = max(level * 1.55, residual)
            return Bands(
                low: drive,
                mid: drive * (0.55 + 0.45 * sin(t * 11)),
                high: drive * (0.45 + 0.55 * sin(t * 19 + 1.7))
            )
        }

        let breath = 0.09 + 0.06 * (0.5 + 0.5 * sin(t * 1.5))
        return Bands(
            low: max(breath, level),
            mid: max(breath * 0.65, level * 0.7),
            high: max(breath * 0.35, level * 0.5)
        )
    }

    static func amp(low: Double, speaking: Bool) -> Double {
        speaking ? 0.055 + low * 0.50 : 0.045 + low * 0.34
    }

    static func spin(mid: Double, speaking: Bool) -> Double {
        speaking ? 0.28 + mid * 1.25 : 0.20 + mid * 0.9
    }
}
