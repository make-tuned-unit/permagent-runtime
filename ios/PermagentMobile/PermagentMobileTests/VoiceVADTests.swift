// VoiceVADTests — drives simulated conversations through the hands-free VAD
// frame by frame, at the real mic tap's ~85 ms cadence, with an injected
// clock. The headline tests reproduce the reported bug: the listening orb
// ended a turn 5-10 seconds in, while the user was still speaking, because
// the keepalive floor read iOS's quiet voiceChat-processed speech as silence.

import AVFoundation
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

    /// Open a turn the way the live machine does: two consecutive onset frames.
    @discardableResult
    private func beginTurn(
        _ vad: inout VoiceVAD,
        rms: Float = 0.03,
        at now: inout TimeInterval
    ) -> VoiceVAD.Action {
        XCTAssertEqual(vad.step(rms: rms, phase: .ready, now: now), .none,
                       "first onset frame must not open a turn")
        now += dt
        let action = vad.step(rms: rms, phase: .ready, now: now)
        XCTAssertEqual(action, .beginTurn, "second onset frame must open the turn")
        return action
    }

    /// Speech onset from ready opens a turn — but only after two frames.
    func testOnsetBeginsTurn() {
        var vad = VoiceVAD()
        XCTAssertEqual(vad.step(rms: 0.001, phase: .ready, now: 100), .none)
        XCTAssertEqual(vad.step(rms: 0.02, phase: .ready, now: 100.085), .none)
        XCTAssertEqual(vad.step(rms: 0.02, phase: .ready, now: 100.170), .beginTurn)
    }

    /// A single room-noise spike must not open a turn (20260821_14 empty STT).
    func testSingleOnsetSpikeDoesNotBeginTurn() {
        var vad = VoiceVAD()
        XCTAssertEqual(vad.step(rms: 0.02, phase: .ready, now: 1), .none)
        XCTAssertEqual(vad.step(rms: 0.001, phase: .ready, now: 1.085), .none)
        XCTAssertEqual(vad.step(rms: 0.02, phase: .ready, now: 1.17), .none)
    }

    /// THE REGRESSION: soft-but-real speech (RMS 0.008 — under the old 0.010
    /// keepalive that shipped, above the retuned floor) must not read as
    /// silence. With the old floor this turn ended ~1.5 s after onset; the
    /// reported symptom was the orb cutting out 5-10 s into an utterance
    /// whenever it softened. Twenty seconds of it must now ride through.
    func testSoftSpeechIsNotCutAsSilence() {
        var vad = VoiceVAD()
        var now: TimeInterval = 1_000
        beginTurn(&vad, rms: 0.02, at: &now)
        let cut = run(&vad, rms: 0.008, phase: .listening, duration: 20, from: &now)
        XCTAssertNil(cut, "soft speech was treated as silence and ended the turn at +\(cut?.at ?? 0)s")
    }

    /// Sustained speech runs the FULL minute: nothing may end the turn before
    /// the 60 s cap, and the cap itself must end it (acceptance: listening
    /// stays active for a full 60 seconds, then stops).
    func testContinuousSpeechListensForFullMinuteThenCaps() {
        var vad = VoiceVAD()
        var now: TimeInterval = 5_000
        beginTurn(&vad, rms: 0.03, at: &now)
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
        beginTurn(&vad, rms: 0.03, at: &now)
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

    /// A QUICK ask (short voiced duration) hands over fast: the adaptive
    /// endpoint ends it on the tight window, not the dictation-patient one.
    /// This is the "still a bit of lag" fix (2026-08-06) — every one-line
    /// command used to pay the full 1.8 s.
    func testQuickAskEndsOnTheTightWindow() {
        var vad = VoiceVAD()
        var now: TimeInterval = 2_000
        beginTurn(&vad, rms: 0.03, at: &now)
        XCTAssertNil(run(&vad, rms: 0.03, phase: .listening, duration: 2, from: &now))
        guard let end = run(&vad, rms: 0.0005, phase: .listening, duration: 5, from: &now) else {
            return XCTFail("trailing silence never ended the turn")
        }
        XCTAssertEqual(end.action, .endTurn)
        XCTAssertGreaterThan(end.at, 0.4, "quick-ask endpoint got tighter than the 300-800ms band")
        XCTAssertLessThan(end.at, 0.75, "quick-ask endpoint drifted — lag is back")
    }

    /// The quick window is the one that moved on 2026-08-25 (800 -> 500 ms).
    /// Pinned against the research band so a future tweak has to be deliberate.
    func testQuickWindowSitsInsideThePublishedBand() {
        let c = VoiceVAD.Config()
        XCTAssertGreaterThanOrEqual(c.quickSilenceMs, 300)
        XCTAssertLessThanOrEqual(c.quickSilenceMs, 800)
        XCTAssertLessThan(c.quickSilenceMs, c.silenceMs,
                          "quick window must stay tighter than the dictation window")
        XCTAssertEqual(c.quickTurnSpeechMs, 3_500,
                       "raising the classifier hands mid-thought pauses to the tight window")
    }

    /// The endpoint windows are tunable by ear without a rebuild, and a bad
    /// value must clamp rather than wedge a turn open or cut every sentence.
    func testDefaultsOverrideAppliesAndClamps() {
        let suite = "voice.vad.tests.\(UUID().uuidString)"
        guard let d = UserDefaults(suiteName: suite) else { return XCTFail("no suite") }
        defer { UserDefaults().removePersistentDomain(forName: suite) }

        // Absent keys leave the compiled defaults alone.
        XCTAssertEqual(VoiceVAD.applyingDefaults(.init(), defaults: d).silenceMs, 1_400)

        d.set(700.0, forKey: VoiceVAD.DefaultsKey.silenceMs)
        d.set(350.0, forKey: VoiceVAD.DefaultsKey.quickSilenceMs)
        let tuned = VoiceVAD.applyingDefaults(.init(), defaults: d)
        XCTAssertEqual(tuned.silenceMs, 700)
        XCTAssertEqual(tuned.quickSilenceMs, 350)

        // Absurd values clamp to the accepted range.
        d.set(99_000.0, forKey: VoiceVAD.DefaultsKey.silenceMs)
        d.set(1.0, forKey: VoiceVAD.DefaultsKey.quickSilenceMs)
        let clamped = VoiceVAD.applyingDefaults(.init(), defaults: d)
        XCTAssertEqual(clamped.silenceMs, VoiceVAD.silenceOverrideRange.upperBound)
        XCTAssertEqual(clamped.quickSilenceMs, VoiceVAD.silenceOverrideRange.lowerBound)

        // The two-tier invariant holds even if the knob inverts it.
        d.set(600.0, forKey: VoiceVAD.DefaultsKey.silenceMs)
        d.set(1_500.0, forKey: VoiceVAD.DefaultsKey.quickSilenceMs)
        let inverted = VoiceVAD.applyingDefaults(.init(), defaults: d)
        XCTAssertLessThanOrEqual(inverted.quickSilenceMs, inverted.silenceMs)
    }

    /// A tuned window must reach the live VAD through the route chooser, not
    /// just through `applyingDefaults` — the engine only ever calls the former.
    func testRouteChooserCarriesTheOverride() {
        let suite = "voice.vad.tests.\(UUID().uuidString)"
        guard let d = UserDefaults(suiteName: suite) else { return XCTFail("no suite") }
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        d.set(650.0, forKey: VoiceVAD.DefaultsKey.silenceMs)
        let c = VoiceVAD.configForRoute(inputPortTypes: [VoiceVAD.builtInMicPort], defaults: d)
        XCTAssertEqual(c.silenceMs, 650)
        XCTAssertEqual(c.onset, VoiceVAD.builtInMicConfig.onset,
                       "the override must not disturb the route preset's thresholds")
    }

    /// A LONG turn (dictation-length voiced duration) keeps the patient
    /// window: once the speaker actually stops, it completes promptly — but
    /// never on the tight window that would cut a mid-thought pause.
    func testLongTurnKeepsThePatientWindow() {
        var vad = VoiceVAD()
        var now: TimeInterval = 2_000
        beginTurn(&vad, rms: 0.03, at: &now)
        XCTAssertNil(run(&vad, rms: 0.03, phase: .listening, duration: 5, from: &now))
        guard let end = run(&vad, rms: 0.0005, phase: .listening, duration: 5, from: &now) else {
            return XCTFail("trailing silence never ended the turn")
        }
        XCTAssertEqual(end.action, .endTurn)
        XCTAssertGreaterThan(end.at, 1.2, "a long turn ended on the quick window — dictation pauses would cut again")
        XCTAssertLessThan(end.at, 1.65, "silence window drifted — turn-taking would feel sluggish")
    }

    /// A push-to-talk turn stamps its own clocks. Before the fix it inherited
    /// the previous hands-free turn's epoch, so the max-turn clause could end
    /// it on the very first frame.
    func testExternallyBegunTurnDoesNotInheritStaleClocks() {
        var vad = VoiceVAD()
        var now: TimeInterval = 0
        // A full hands-free turn, long ago.
        beginTurn(&vad, rms: 0.03, at: &now)
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

    // ── Route-aware thresholds ──────────────────────────────────────────────

    /// The regression (2026-08-06): "Listening" heard a headset but not the
    /// bare iPhone. Ordinary speech through the built-in mic under voiceChat
    /// AGC lands around RMS 0.010 — under the headset onset (0.015), above
    /// the built-in preset's (0.008). The built-in preset must open the turn.
    func testBuiltInMicPresetHearsQuietOnset() {
        var headset = VoiceVAD(config: VoiceVAD.headsetConfig)
        XCTAssertEqual(headset.step(rms: 0.010, phase: .ready, now: 1), .none,
                       "headset calibration unexpectedly loosened")
        var builtIn = VoiceVAD(config: VoiceVAD.builtInMicConfig)
        XCTAssertEqual(builtIn.step(rms: 0.010, phase: .ready, now: 1), .none)
        XCTAssertEqual(builtIn.step(rms: 0.010, phase: .ready, now: 1.085), .beginTurn,
                       "the bare iPhone mic must hear ordinary speech")
    }

    func testBuiltInMicPortMatchesSDK() {
        XCTAssertEqual(
            VoiceVAD.builtInMicPort,
            AVAudioSession.Port.builtInMic.rawValue,
            "route chooser would miss the phone mic and use headset thresholds"
        )
    }

    /// The chooser keys on the built-in port type; any headset/BT route keeps
    /// the calibrated defaults. Speakerphone is a separate preset (higher barge).
    func testRouteChooserPicksPresetByPortType() {
        let builtIn = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.builtInMic.rawValue]
        )
        XCTAssertEqual(builtIn.onset, VoiceVAD.builtInMicConfig.onset)
        let bt = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.bluetoothHFP.rawValue]
        )
        XCTAssertEqual(bt.onset, VoiceVAD.headsetConfig.onset)
        let wired = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.headsetMic.rawValue]
        )
        XCTAssertEqual(wired.onset, VoiceVAD.headsetConfig.onset)
        XCTAssertEqual(VoiceVAD.configForRoute(inputPortTypes: []).onset,
                       VoiceVAD.headsetConfig.onset)
        let speaker = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.builtInMic.rawValue], speakerphone: true
        )
        XCTAssertEqual(speaker.onset, VoiceVAD.builtInMicConfig.onset)
        XCTAssertEqual(speaker.barge, VoiceVAD.speakerphoneConfig.barge)
        XCTAssertGreaterThan(speaker.barge, VoiceVAD.builtInMicConfig.barge)
        let btOnSpeakerFlagIgnored = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.bluetoothHFP.rawValue], speakerphone: false
        )
        XCTAssertEqual(btOnSpeakerFlagIgnored.barge, VoiceVAD.headsetConfig.barge)
    }

    /// Headphones must not inherit the speakerphone barge floor — AEC is on
    /// and the mic is at the mouth, so interrupting should stay easy.
    func testHeadphonesKeepHeadsetBargeNotSpeakerphone() {
        let hp = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.bluetoothHFP.rawValue], speakerphone: false
        )
        XCTAssertEqual(hp.barge, VoiceVAD.headsetConfig.barge)
        XCTAssertLessThan(hp.barge, VoiceVAD.speakerphoneConfig.barge)
    }

    /// Speakerphone (AEC off) must still hear arm's-length speech, but TTS
    /// bleed from the same speaker (~0.035 RMS) must not barge-in. Headset
    /// barge at that level is correct; speakerphone barge is raised.
    func testSpeakerphoneHearsQuietSpeechButNotTtsBleed() {
        var onset = VoiceVAD(config: VoiceVAD.speakerphoneConfig)
        XCTAssertEqual(onset.step(rms: 0.010, phase: .ready, now: 1), .none)
        XCTAssertEqual(onset.step(rms: 0.010, phase: .ready, now: 1.085), .beginTurn,
                       "speakerphone must hear the same quiet speech as the built-in preset")
        var bleed = VoiceVAD(config: VoiceVAD.speakerphoneConfig)
        XCTAssertEqual(bleed.step(rms: 0.035, phase: .speaking, now: 1), .none)
        XCTAssertEqual(bleed.step(rms: 0.035, phase: .speaking, now: 1.085), .none,
                       "speaker TTS bleed barged in — interrupting himself")
        XCTAssertEqual(bleed.step(rms: 0.09, phase: .speaking, now: 2), .none)
        XCTAssertEqual(bleed.step(rms: 0.09, phase: .speaking, now: 2.085), .none,
                       "speakerphone barge needs three frames so echo cannot cut him")
        XCTAssertEqual(bleed.step(rms: 0.09, phase: .speaking, now: 2.17), .interrupt)
    }

    /// The built-in preset preserves the calibrated ratios: keepalive at
    /// 0.4 × onset (soft speech is not silence) and barge at 2 × onset
    /// (interrupting must not take a shout).
    func testBuiltInPresetPreservesThresholdRatios() {
        let c = VoiceVAD.builtInMicConfig
        XCTAssertEqual(c.keepalive / c.onset, 0.4, accuracy: 0.01)
        XCTAssertEqual(c.barge / c.onset, 2.0, accuracy: 0.01)
    }

    /// 20260821_14 23:47:10 → 23:47:51: speakerphone room hiss (~0.0032)
    /// sat on the built-in keepalive and held Listening for ~41 s after
    /// the user stopped. Hiss must now read as silence.
    func testSpeakerphoneRoomHissDoesNotHoldTheTurn() {
        var vad = VoiceVAD(config: VoiceVAD.speakerphoneConfig)
        var now: TimeInterval = 23_470
        beginTurn(&vad, rms: 0.012, at: &now)
        XCTAssertNil(run(&vad, rms: 0.012, phase: .listening, duration: 1.2, from: &now),
                     "real speech ended the turn early")
        guard let end = run(&vad, rms: 0.0035, phase: .listening, duration: 8, from: &now) else {
            return XCTFail("room hiss held the turn — the 41s Listening hang is back")
        }
        XCTAssertEqual(end.action, .endTurn)
        XCTAssertLessThan(end.at, 1.2, "hiss-held endpoint at +\(end.at)s — Listening hang is back")
    }

    /// A noise-only open (two onset frames, then silence) must abort before
    /// the full quick window — those were the 1.5–2 s empty-STT flashes.
    func testUncommittedNoiseTurnAbortsQuickly() {
        var vad = VoiceVAD()
        var now: TimeInterval = 100
        beginTurn(&vad, rms: 0.02, at: &now)
        guard let end = run(&vad, rms: 0.0005, phase: .listening, duration: 3, from: &now) else {
            return XCTFail("noise-only turn never ended")
        }
        XCTAssertEqual(end.action, .endTurn)
        XCTAssertLessThan(end.at, 0.85, "uncommitted abort drifted to +\(end.at)s")
    }

    /// 2026-08-27 07:44–07:52 ADT: six speakerphone turns rode kitchen
    /// music/hiss at *just above* keepalive (0.0055) all the way to maxTurnMs
    /// 60 s, then STT came back empty. `testSpeakerphoneRoomHissDoesNotHoldTheTurn`
    /// uses 0.0035 — *below* keepalive — and already passes. Music at cooking
    /// volume sits *on* the floor, refreshes lastVoice, commits via
    /// voicedAccumMs, and never looks like silence. Uncommitted keepalive-hiss
    /// must abort on abortSilenceMs (~500 ms), not ride the minute cap.
    func testSpeakerphoneKeepaliveHissAbortsUncommittedTurn() {
        var vad = VoiceVAD(config: VoiceVAD.speakerphoneConfig)
        var now: TimeInterval = 10_440
        beginTurn(&vad, rms: 0.012, at: &now)
        let hiss = VoiceVAD.speakerphoneConfig.keepalive + 0.0002
        guard let end = run(&vad, rms: hiss, phase: .listening, duration: 3, from: &now) else {
            return XCTFail("keepalive hiss rode the turn — kitchen music would sit to the 60 s cap")
        }
        XCTAssertEqual(end.action, .endTurn)
        XCTAssertLessThan(end.at, 1.0, "hiss-held endpoint at +\(end.at)s — must abort uncommitted, not ride")
    }
}
