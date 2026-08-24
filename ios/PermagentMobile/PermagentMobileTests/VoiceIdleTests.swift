import XCTest

final class VoiceIdleTests: XCTestCase {
    func testLastNightEmptySttToastsAreTransient() {
        XCTAssertTrue(VoiceIdle.isTransientEmptyTurn("No speech detected — try again"))
        XCTAssertTrue(VoiceIdle.isTransientEmptyTurn("Recording too short — hold longer to speak"))
        XCTAssertFalse(VoiceIdle.isTransientEmptyTurn("STT failed: model missing"))
        XCTAssertFalse(VoiceIdle.isTransientEmptyTurn("Voice reply failed: timeout"))
        XCTAssertFalse(VoiceIdle.isTransientEmptyTurn(nil))
    }
}
