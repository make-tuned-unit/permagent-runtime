// VoiceVADTests — drives simulated conversations through the hands-free VAD
// frame by frame, at the real mic tap's ~85 ms cadence, with an injected
// clock. The headline tests reproduce the reported bug: the listening orb
// ended a turn 5-10 seconds in, while the user was still speaking, because
// the keepalive floor read iOS's quiet voiceChat-processed speech as silence.

import XCTest

final class VoiceVADTests: XCTestCase {
    /// The real tap delivers ~85 ms buffers (4096 frames @ 48 kHz).
    private let dt: TimeInterval = 0.085

    /// Feed constant-RMS frames for `duration`, returning the first non-`.none`
    /// action and the offset (from `now` at entry) at which it fired.
    private func run(
        _ vad: inout VoiceVAD,
        rms: Float,
        phase: VoiceVAD.Phase,
        duration: TimeInterval,
        from now: inout TimeInterval
    ) -> (action: VoiceVAD.Action, at: TimeInterval)? {
        let start = now
        while now - start < duration {
            now += dt
            let action = vad.step(rms: rms, phase: phase, now: now)
            if action != .none { return (action, now - start) }
        }
        return nil
    }

    /// Speech onset from ready opens a turn.
    func testOnsetBeginsTurn() {
        var vad = VoiceVAD()
        XCTAssertEqual(vad.step(rms: 0.001, phase: .ready, now: 100), .none)
        XCTAssertEqual(vad.step(rms: 0.02, phase: .ready, now: 100.085), .beginTurn)
    }

    /// THE REGRESSION: soft-but-real speech (RMS 0.008 — under the old 0.010
    /// keepalive that shipped, above the retuned floor) must not read as
    /// silence. With the old floor this turn ended ~1.5 s after onset; the
    /// reported symptom was the orb cutting out 5-10 s into an utterance
    /// whenever it softened. Twenty seconds of it must now ride through.
    func testSoftSpeechIsNotCutAsSilence() {
        var vad = VoiceVAD()
        var now: TimeInterval = 1_000
        XCTAssertEqual(vad.step(rms: 0.02, phase: .ready, now: now), .beginTurn)
        let cut = run(&vad, rms: 0.008, phase: .listening, duration: 20, from: &now)
        XCTAssertNil(cut, "soft speech was treated as silence and ended the turn at +\(cut?.at ?? 0)s")
    }

    /// Sustained speech runs the FULL minute: nothing may end the turn before
    /// the 60 s cap, and the cap itself must end it (acceptance: listening
    /// stays active for a full 60 seconds, then stops).
    func testContinuousSpeechListensForFullMinuteThenCaps() {
        var vad = VoiceVAD()
        var now: TimeInterval = 5_000
        XCTAssertEqual(vad.step(rms: 0.03, phase: .ready, now: now), .beginTurn)
        guard let end = run(&vad, rms: 0.03, phase: .listening, duration: 70, from: &now) else {
            return XCTFail("the 60 s turn cap never fired")
        }
        XCTAssertEqual(end.action, .endTurn)
        XCTAssertGreaterThan(end.at, 59.9, "turn ended early, at +\(end.at)s — premature cutoff")
        XCTAssertLessThan(end.at, 60.2, "turn cap fired late, at +\(end.at)s")
    }

    /// Dictation-paced speech — bursts separated by ~1.2 s thinking pauses —
    /// must also survive to the cap. Under the old 900 ms window the FIRST
    /// pause ended the turn (~5 s in, exactly the reported cutoff).
    func testNaturalPausesDoNotEndTheTurn() {
        var vad = VoiceVAD()
        var now: TimeInterval = 9_000
        XCTAssertEqual(vad.step(rms: 0.03, phase: .ready, now: now), .beginTurn)
        var elapsed: TimeInterval = 0
        while elapsed < 55 {
            if let hit = run(&vad, rms: 0.03, phase: .listening, duration: 4, from: &now) {
                return XCTFail("cut during speech at ~+\(elapsed + hit.at)s")
            }
            elapsed += 4
            if let hit = run(&vad, rms: 0.001, phase: .listening, duration: 1.2, from: &now) {
                return XCTFail("cut during a natural pause at ~+\(elapsed + hit.at)s")
            }
            elapsed += 1.2
        }
    }

    /// Turn-taking still works: once the speaker actually stops, the trailing
    /// -silence window completes the turn promptly — not at the 60 s cap.
    func testTrailingSilenceEndsTurn() {
        var vad = VoiceVAD()
        var now: TimeInterval = 2_000
        XCTAssertEqual(vad.step(rms: 0.03, phase: .ready, now: now), .beginTurn)
        XCTAssertNil(run(&vad, rms: 0.03, phase: .listening, duration: 3, from: &now))
        guard let end = run(&vad, rms: 0.0005, phase: .listening, duration: 5, from: &now) else {
            return XCTFail("trailing silence never ended the turn")
        }
        XCTAssertEqual(end.action, .endTurn)
        XCTAssertGreaterThan(end.at, 1.4)
        XCTAssertLessThan(end.at, 2.0, "silence window drifted — turn-taking would feel sluggish")
    }

    /// A push-to-talk turn stamps its own clocks. Before the fix it inherited
    /// the previous hands-free turn's epoch, so the max-turn clause could end
    /// it on the very first frame.
    func testExternallyBegunTurnDoesNotInheritStaleClocks() {
        var vad = VoiceVAD()
        var now: TimeInterval = 0
        // A full hands-free turn, long ago.
        XCTAssertEqual(vad.step(rms: 0.03, phase: .ready, now: now), .beginTurn)
        XCTAssertNil(run(&vad, rms: 0.03, phase: .listening, duration: 2, from: &now))
        XCTAssertNotNil(run(&vad, rms: 0.0005, phase: .listening, duration: 5, from: &now))
        vad.noteTurnEnded()
        // Much later: a push-to-talk turn begins without a VAD onset.
        now += 300
        vad.noteTurnBegan(at: now)
        let hit = run(&vad, rms: 0.001, phase: .listening, duration: 5, from: &now)
        XCTAssertNil(hit, "externally-begun turn ended at +\(hit?.at ?? 0)s off stale clocks")
    }

    /// Barge-in still demands two consecutive loud frames, and only while the
    /// agent is speaking — a single transient or thinking-phase noise is inert.
    func testBargeInRequiresSustainedLoudSpeechWhileSpeaking() {
        var vad = VoiceVAD()
        XCTAssertEqual(vad.step(rms: 0.09, phase: .speaking, now: 1), .none)
        XCTAssertEqual(vad.step(rms: 0.001, phase: .speaking, now: 1.085), .none)  // streak broken
        XCTAssertEqual(vad.step(rms: 0.09, phase: .speaking, now: 1.17), .none)
        XCTAssertEqual(vad.step(rms: 0.09, phase: .speaking, now: 1.255), .interrupt)
        // Thinking never interrupts, no matter how loud or sustained.
        XCTAssertEqual(vad.step(rms: 0.09, phase: .thinking, now: 2), .none)
        XCTAssertEqual(vad.step(rms: 0.09, phase: .thinking, now: 2.085), .none)
        XCTAssertEqual(vad.step(rms: 0.09, phase: .thinking, now: 2.17), .none)
    }

    /// Barge-in must fire at ORDINARY speaking volume, not a raised voice.
    /// The threshold was copied verbatim from the web (0.05) while onset was
    /// lowered for iOS's AGC, leaving barge at 3.3× onset instead of the web's
    /// 2× — interrupting took a shout.
    func testBargeInFiresAtOrdinarySpeechLevel() {
        var vad = VoiceVAD()
        let ordinary: Float = 0.035   // just above conversational onset (0.015)
        XCTAssertEqual(vad.step(rms: ordinary, phase: .speaking, now: 1), .none)
        XCTAssertEqual(vad.step(rms: ordinary, phase: .speaking, now: 1.085), .interrupt)
        // Residual TTS bleed below the barge floor still must not interrupt.
        var quiet = VoiceVAD()
        for i in 0..<6 {
            XCTAssertEqual(
                quiet.step(rms: 0.02, phase: .speaking, now: 1 + Double(i) * 0.085),
                .none
            )
        }
    }

    /// The listening cap is the required one minute.
    func testListeningCapIsSixtySeconds() {
        XCTAssertEqual(VoiceVAD.Config().maxTurnMs, 60_000)
    }
}
