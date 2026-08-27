import XCTest

final class VoiceEnrollTests: XCTestCase {
    func testThreeKitchenSentencesMatchTheHub() {
        XCTAssertEqual(VoiceEnroll.need, 3)
        XCTAssertEqual(VoiceEnroll.prompts.count, 3)
        XCTAssertEqual(VoiceEnroll.prompt(have: 0), "What's on my board?")
        XCTAssertEqual(VoiceEnroll.prompt(have: 1), "Henry, I'm in the kitchen.")
        XCTAssertEqual(VoiceEnroll.prompt(have: 2), "Tell me something interesting.")
        XCTAssertNil(VoiceEnroll.prompt(have: 3))
        XCTAssertNil(VoiceEnroll.prompt(have: -1))
    }

    func testOrbTextPrefersPronunciationTeachOverEnroll() {
        XCTAssertEqual(
            VoiceEnroll.orbText(teachWord: "Elspeth", enrollPrompt: "What's on my board?"),
            "Elspeth"
        )
        XCTAssertEqual(
            VoiceEnroll.orbText(teachWord: nil, enrollPrompt: "What's on my board?"),
            "What's on my board?"
        )
        XCTAssertNil(VoiceEnroll.orbText(teachWord: "", enrollPrompt: nil))
        XCTAssertNil(VoiceEnroll.orbText(teachWord: nil, enrollPrompt: nil))
    }
}
