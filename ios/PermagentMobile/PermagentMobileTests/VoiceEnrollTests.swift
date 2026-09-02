import XCTest

final class VoiceEnrollTests: XCTestCase {
    func testThreeKitchenSentencesMatchTheHub() {
        XCTAssertEqual(VoiceEnroll.need, 3)
        XCTAssertEqual(VoiceEnroll.prompts.count, 3)
        XCTAssertEqual(VoiceEnroll.prompt(have: 0), "What's on my board?")
        XCTAssertEqual(VoiceEnroll.prompt(have: 1), "This is the voice I want you to answer.")
        XCTAssertEqual(VoiceEnroll.prompt(have: 2), "Tell me something interesting.")
        XCTAssertNil(VoiceEnroll.prompt(have: 3))
        XCTAssertNil(VoiceEnroll.prompt(have: -1))
    }

    func testPromptsNeverHardcodeTheAgentName() {
        XCTAssertFalse(VoiceEnroll.prompts.contains { $0.lowercased().contains("henry") })
    }

    /// BUG A. The "Voice identity" setup screen turns hands-free OFF (it is a
    /// control surface, not a conversation surface), and `handleMicFrame` used
    /// to gate every VAD step on `handsFree`. An enrollment take has no
    /// push-to-talk control — trailing silence is the ONLY thing that can end
    /// it — so the first take recorded forever, no Stop ever reached the hub,
    /// and the orb never advanced past sentence one.
    func testEnrollmentListeningKeepsAutomaticSilenceEndpointWhenHandsFreeIsOff() {
        XCTAssertTrue(VoiceEnroll.shouldDriveVAD(
            handsFree: false,
            enrolling: true,
            isListening: true
        ))
        // Still no ambient onset detection on the setup screen: the VAD runs
        // only inside a take the hub asked for, never from `.ready`.
        XCTAssertFalse(VoiceEnroll.shouldDriveVAD(
            handsFree: false,
            enrolling: true,
            isListening: false
        ))
        XCTAssertFalse(VoiceEnroll.shouldDriveVAD(
            handsFree: false,
            enrolling: false,
            isListening: true
        ))
        // Ordinary conversation is unchanged in every combination.
        XCTAssertTrue(VoiceEnroll.shouldDriveVAD(
            handsFree: true,
            enrolling: false,
            isListening: false
        ))
        XCTAssertTrue(VoiceEnroll.shouldDriveVAD(
            handsFree: true,
            enrolling: true,
            isListening: true
        ))
    }

    /// BUG B. Enrollment takes are opened by hub status (`enroll_status` /
    /// `idle`), never by the VAD's own `.ready` onset detector — the setup
    /// screen goes straight from `.ready` to `.listening`. So nothing stamps
    /// the VAD's turn clocks unless the open path does it itself, exactly as
    /// push-to-talk's `beginTurn()` already does. Take one leaves its clocks
    /// behind and take two inherits them.
    ///
    /// This drives the real two-take sequence — take, hub round trip, next
    /// take — through the production open path and the real `VoiceVAD`, and
    /// requires that the second take does not endpoint before a word is said.
    /// Gut the stamp in `VoiceEnroll.openTake` and take ONE already fails: an
    /// unstamped `turnStart` puts the max-turn cap at the 1970 epoch.
    func testEnrollmentTakeAdvanceStampsFreshClocksSoTheNextTakeSurvives() {
        // A real wall clock, as VoiceEngine passes Date().timeIntervalSince1970.
        var t: TimeInterval = 1_756_600_000
        let dt: TimeInterval = 0.085  // the mic tap's ~85 ms cadence
        var vad = VoiceVAD()

        // ── Take 1, opened the way beginEnrollmentTake() opens it ──────────
        VoiceEnroll.openTake(&vad, now: t)
        for _ in 0..<12 {
            t += dt
            XCTAssertEqual(vad.step(rms: 0.05, phase: .listening, now: t), .none,
                           "take 1 was cut off while the sentence was being read")
        }
        var ended = false
        for _ in 0..<12 where !ended {
            t += dt
            ended = vad.step(rms: 0.001, phase: .listening, now: t) == .endTurn
        }
        XCTAssertTrue(ended, "take 1 never endpointed on trailing silence")

        // The engine's endTurn(): Stop goes to the hub, per-turn state clears.
        vad.noteTurnEnded()

        // The hub round trip — STT, speaker print, enroll_status, next prompt —
        // with the mic still delivering frames while the client sits thinking.
        for _ in 0..<50 {
            t += dt
            XCTAssertEqual(vad.step(rms: 0.001, phase: .thinking, now: t), .none)
        }

        // ── Take 2. It must survive the pause before the user starts reading ─
        VoiceEnroll.openTake(&vad, now: t)
        for _ in 0..<20 {
            t += dt
            XCTAssertEqual(vad.step(rms: 0.001, phase: .listening, now: t), .none,
                           "take 2 endpointed on stale clocks before a word was said")
        }
        for _ in 0..<12 {
            t += dt
            XCTAssertEqual(vad.step(rms: 0.05, phase: .listening, now: t), .none,
                           "take 2 was cut off while the sentence was being read")
        }
        ended = false
        for _ in 0..<12 where !ended {
            t += dt
            ended = vad.step(rms: 0.001, phase: .listening, now: t) == .endTurn
        }
        XCTAssertTrue(ended, "take 2 never endpointed on trailing silence")
    }

    /// The mechanism Bug B's fix defends against, pinned directly: a take
    /// opened by bare `enterListening()` — no onset, no stamp — measures the
    /// max-turn cap from whatever `turnStart` holds, which for enrollment is
    /// never anything at all. The very first frame ends the take.
    func testUnstampedTakeEndsOnItsFirstFrame() {
        var vad = VoiceVAD()
        XCTAssertEqual(
            vad.step(rms: 0.05, phase: .listening, now: 1_756_600_000),
            .endTurn,
            "an unstamped take is expected to die instantly — that is the bug"
        )
    }

    func testRejectedSpeakerRequiresSustainedQuietBeforeRearming() {
        var gate = VoiceIdentityQuietGate()
        gate.lock()
        XCTAssertTrue(gate.locked)
        for _ in 0..<(VoiceIdentityQuietGate.quietFramesNeeded - 1) {
            XCTAssertTrue(gate.observe(rms: 0.001))
        }
        XCTAssertFalse(gate.observe(rms: 0.001))
    }

    func testBackgroundSpeechResetsTheQuietWindow() {
        var gate = VoiceIdentityQuietGate()
        gate.lock()
        for _ in 0..<4 { _ = gate.observe(rms: 0.001) }
        XCTAssertTrue(gate.observe(rms: 0.02))
        for _ in 0..<5 { XCTAssertTrue(gate.observe(rms: 0.001)) }
        XCTAssertFalse(gate.observe(rms: 0.001))
    }
}
