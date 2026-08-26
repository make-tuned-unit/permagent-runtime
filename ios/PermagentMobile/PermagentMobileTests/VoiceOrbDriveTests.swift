import XCTest

/// The orb's contract (user report, 2026-08-25): while THEY speak it pulses with
/// their voice; while the AGENT speaks it changes shape with the speech; and
/// the wait in between must not look like either of those. These tests pin
/// each of those three claims, and the level scales they have to hold at:
/// ordinary speech reaches `bands` as roughly 0.10-0.60 on both the mic path
/// (`min(1, rms * 12)`) and the playback tap (`min(1, rms * 2)`).
final class VoiceOrbDriveTests: XCTestCase {
    /// Mid-range speech levels used throughout — the band where the old
    /// synthetic breath swallowed the real signal.
    private let quiet = 0.12
    private let loud = 0.45

    // ── LISTENING: the pulse is the microphone ──────────────────────────────

    /// The regression this whole change exists for. The old listening floor
    /// was a 0.16-0.27 sine, sitting ON TOP of ordinary speech, so
    /// `max(breath, level)` returned the breath and the orb pulsed to a
    /// metronome. Two different speech levels must now produce two different
    /// orbs, at every phase of the floor's cycle.
    func testListeningTracksTheVoiceRatherThanASine() {
        for t in stride(from: 0.0, through: 3.0, by: 0.25) {
            let soft = VoiceOrbDrive.bands(
                thinking: false, listening: true, speaking: false, level: quiet, t: t)
            let hard = VoiceOrbDrive.bands(
                thinking: false, listening: true, speaking: false, level: loud, t: t)
            XCTAssertGreaterThan(
                hard.low, soft.low + 0.1,
                "at t=\(t) the listening orb did not distinguish soft speech from loud — "
                    + "the synthetic floor is back on top of the voice")
        }
    }

    /// The floor must stay under ordinary speech, or it swallows it again.
    func testListeningFloorSitsBelowOrdinarySpeech() {
        for t in stride(from: 0.0, through: 3.0, by: 0.25) {
            let silent = VoiceOrbDrive.bands(
                thinking: false, listening: true, speaking: false, level: 0, t: t)
            XCTAssertLessThanOrEqual(
                silent.low, VoiceOrbDrive.floorCeiling,
                "listening floor at t=\(t) is loud enough to mask real speech")
        }
    }

    /// …but silence still breathes. A dead sphere under a LISTENING label was
    /// its own reported bug.
    func testListeningStillBreathesAtTrueSilence() {
        let a = VoiceOrbDrive.bands(thinking: false, listening: true, speaking: false, level: 0, t: 0)
        let b = VoiceOrbDrive.bands(thinking: false, listening: true, speaking: false, level: 0, t: 0.7)
        XCTAssertGreaterThan(a.low, 0.03, "listening orb was static at silence")
        XCTAssertNotEqual(a.low, b.low, accuracy: 0.001, "listening breath did not move")
    }

    // ── SPEAKING: shape follows the TTS envelope ────────────────────────────

    /// Speaking must swell with the playback tap and keep a residual so a
    /// quiet syllable does not kill the orb — but the residual is a FLOOR,
    /// not a blend, so the shape stays the agent's voice.
    func testSpeakingMovesWithVoiceAndKeepsAResidualPulse() {
        let hush = VoiceOrbDrive.bands(thinking: false, listening: false, speaking: true, level: 0, t: 0.2)
        XCTAssertGreaterThan(hush.low, 0.08, "speaking orb died on a quiet syllable")
        let full = VoiceOrbDrive.bands(thinking: false, listening: false, speaking: true, level: 0.6, t: 0.2)
        XCTAssertGreaterThan(full.low, hush.low)
        XCTAssertGreaterThan(
            VoiceOrbDrive.amp(low: 0.6, speaking: true),
            VoiceOrbDrive.amp(low: 0.6, speaking: false)
        )
        XCTAssertGreaterThan(
            VoiceOrbDrive.spin(mid: 0.6, speaking: true),
            VoiceOrbDrive.spin(mid: 0.6, speaking: false)
        )
    }

    /// `low` drives `amp`, which is the magnitude of the noise-field
    /// displacement — i.e. the SHAPE. A rising envelope must move it
    /// monotonically, at every phase, or "changes shape with the speech" is
    /// decoration rather than a signal.
    func testSpeakingShapeIsMonotonicInTheEnvelope() {
        for t in stride(from: 0.0, through: 2.0, by: 0.2) {
            var previous = -1.0
            for level in stride(from: 0.1, through: 0.9, by: 0.1) {
                let amp = VoiceOrbDrive.amp(
                    low: VoiceOrbDrive.bands(
                        thinking: false, listening: false, speaking: true, level: level, t: t
                    ).low,
                    speaking: true
                )
                XCTAssertGreaterThan(amp, previous, "shape stalled at level=\(level), t=\(t)")
                previous = amp
            }
        }
    }

    // ── THINKING: a different KIND of motion ────────────────────────────────

    /// With multiple seconds of model thinking in front of every reply, this
    /// is the state the user stares at. It must not read as LISTENING: it
    /// turns instead of pulsing, so `low` (shape) stays near-flat while `mid`
    /// (spin) stays high.
    func testThinkingTurnsInsteadOfPulsing() {
        let samples = stride(from: 0.0, through: 6.0, by: 0.2).map {
            VoiceOrbDrive.bands(thinking: true, listening: false, speaking: false, level: 0, t: $0)
        }
        let lows = samples.map(\.low)
        let swing = (lows.max() ?? 0) - (lows.min() ?? 0)
        XCTAssertLessThan(swing, 0.03, "thinking pulsed like listening — the wait reads as a mic")

        let listeningSwing = stride(from: 0.0, through: 6.0, by: 0.2)
            .map { VoiceOrbDrive.bands(
                thinking: false, listening: true, speaking: false, level: 0, t: $0).low }
        XCTAssertGreaterThan(
            (listeningSwing.max() ?? 0) - (listeningSwing.min() ?? 0), swing,
            "thinking and listening breathe identically — the two states are indistinguishable")

        let mids = samples.map(\.mid)
        XCTAssertGreaterThan(mids.min() ?? 0, 0.3, "thinking stopped turning")
        XCTAssertGreaterThan(
            VoiceOrbDrive.spin(mid: mids.min() ?? 0, speaking: false),
            VoiceOrbDrive.spin(mid: 0.27, speaking: false),
            "thinking does not spin faster than a loud listening frame")
    }

    /// Nothing on screen should be perfectly frozen.
    func testThinkingStillHasItsOwnBreath() {
        let a = VoiceOrbDrive.bands(thinking: true, listening: false, speaking: false, level: 0, t: 0)
        let b = VoiceOrbDrive.bands(thinking: true, listening: false, speaking: false, level: 0, t: 1.0)
        XCTAssertNotEqual(a.low, b.low, accuracy: 0.0005)
    }

    /// The mic tap can hand over NaN after a route change; a poisoned band
    /// reaches CoreGraphics and is fatal.
    func testNonFiniteLevelIsSanitised() {
        for state in [(true, false, false), (false, true, false), (false, false, true)] {
            let b = VoiceOrbDrive.bands(
                thinking: state.0, listening: state.1, speaking: state.2, level: .nan, t: 0.4)
            XCTAssertTrue(b.low.isFinite && b.mid.isFinite && b.high.isFinite)
        }
    }
}
