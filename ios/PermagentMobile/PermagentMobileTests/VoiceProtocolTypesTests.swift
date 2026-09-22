import XCTest

final class VoiceProtocolTypesTests: XCTestCase {
    func testEnrollmentActionsMapToInterruptionEndReason() {
        XCTAssertEqual(voiceEndReasonForEnrollmentAction(.beginEnroll), .interruption)
        XCTAssertEqual(voiceEndReasonForEnrollmentAction(.skipEnroll), .interruption)
        XCTAssertEqual(voiceEndReasonForEnrollmentAction(.clearEnroll), .interruption)
    }
}
