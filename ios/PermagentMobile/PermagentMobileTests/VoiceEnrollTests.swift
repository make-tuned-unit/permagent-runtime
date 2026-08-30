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
