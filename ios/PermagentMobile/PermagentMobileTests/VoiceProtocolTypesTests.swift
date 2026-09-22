import XCTest

final class VoiceProtocolTypesTests: XCTestCase {
    func testEnrollmentActionsMapToInterruptionEndReason() {
        XCTAssertEqual(voiceEndReasonForEnrollmentAction(.beginEnroll), .interruption)
        XCTAssertEqual(voiceEndReasonForEnrollmentAction(.skipEnroll), .interruption)
        XCTAssertEqual(voiceEndReasonForEnrollmentAction(.clearEnroll), .interruption)
    }

    func testFeedbackFramesDecodeThroughServerWireType() throws {
        let decoder = JSONDecoder()
        let start = try decoder.decode(
            VoiceServerWireMessage.self,
            from: Data(#"{"type":"feedback_start","turn_id":"ios-7"}"#.utf8)
        )
        let end = try decoder.decode(
            VoiceServerWireMessage.self,
            from: Data(#"{"type":"feedback_end","turn_id":"ios-7","sample_rate":24000,"cancelled":true}"#.utf8)
        )
        XCTAssertEqual(start, .feedbackStart(VoiceFeedbackStartMessage(turn_id: "ios-7")))
        XCTAssertEqual(
            end,
            .feedbackEnd(VoiceFeedbackEndMessage(turn_id: "ios-7", sample_rate: 24000, cancelled: true))
        )
    }

    func testFeedbackCancelDecodesThroughClientWireType() throws {
        let decoder = JSONDecoder()
        let cancel = try decoder.decode(
            VoiceClientWireMessage.self,
            from: Data(#"{"type":"feedback_cancel","turn_id":"ios-8"}"#.utf8)
        )
        XCTAssertEqual(cancel, .feedbackCancel(VoiceFeedbackCancelMessage(turn_id: "ios-8")))
    }

    func testCapturePolicyUsesTriggerScopedCaps() {
        XCTAssertEqual(
            VoiceCapturePolicy.maxTurnMS(trigger: .vad),
            VoiceCapturePolicy.automaticMaxTurnMS
        )
        XCTAssertEqual(
            VoiceCapturePolicy.maxTurnMS(trigger: .barge),
            VoiceCapturePolicy.automaticMaxTurnMS
        )
        XCTAssertEqual(
            VoiceCapturePolicy.maxTurnMS(trigger: .userTap),
            VoiceCapturePolicy.userMaxTurnMS
        )
        XCTAssertEqual(
            VoiceCapturePolicy.maxTurnMS(trigger: nil),
            VoiceCapturePolicy.userMaxTurnMS
        )
    }

    func testFeedbackStartDispositionRejectsLateOrMismatchedTurns() {
        XCTAssertEqual(
            voiceFeedbackStartDisposition(
                feedbackTurnID: "ios-3",
                expectedTurnID: "ios-3",
                activeListeningTurnID: nil
            ),
            .accept
        )
        XCTAssertEqual(
            voiceFeedbackStartDisposition(
                feedbackTurnID: "ios-3",
                expectedTurnID: "ios-2",
                activeListeningTurnID: nil
            ),
            .cancel
        )
        XCTAssertEqual(
            voiceFeedbackStartDisposition(
                feedbackTurnID: "ios-3",
                expectedTurnID: "ios-3",
                activeListeningTurnID: "ios-4"
            ),
            .cancel
        )
    }

    func testAutomaticQuietRearmerArmsFromMonotonicClockWithoutFrames() {
        XCTAssertFalse(
            VoiceAutomaticQuietRearmer.shouldRearm(
                waiting: true,
                lastVoiceLikeAt: 10.0,
                now: 10.3,
                quietMS: 500
            )
        )
        XCTAssertTrue(
            VoiceAutomaticQuietRearmer.shouldRearm(
                waiting: true,
                lastVoiceLikeAt: 10.0,
                now: 10.6,
                quietMS: 500
            )
        )
    }
}
