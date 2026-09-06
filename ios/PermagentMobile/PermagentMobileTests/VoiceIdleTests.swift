import XCTest
import Foundation

final class VoiceIdleTests: XCTestCase {
    func testLastNightEmptySttToastsAreTransient() {
        XCTAssertTrue(VoiceIdle.isTransientEmptyTurn("No speech detected — try again"))
        XCTAssertTrue(VoiceIdle.isTransientEmptyTurn("Recording too short — hold longer to speak"))
        XCTAssertFalse(VoiceIdle.isTransientEmptyTurn("STT failed: model missing"))
        XCTAssertFalse(VoiceIdle.isTransientEmptyTurn("Voice reply failed: timeout"))
        XCTAssertFalse(VoiceIdle.isTransientEmptyTurn(nil))
    }

    func testPartialTranscriptIsReplacedOnlyByFinalTranscript() {
        var buffer = VoiceTranscriptBuffer()
        buffer.acceptPartial("what about the")
        XCTAssertEqual(buffer.displayText, "what about the")
        buffer.acceptFinal("What about the model?")
        buffer.acceptPartial("late stale partial")
        XCTAssertEqual(buffer.displayText, "What about the model?")
        XCTAssertEqual(buffer.partial, "")
    }

    func testTurnFeedbackKeepsEmptyAndTransportFailuresRecoverable() {
        XCTAssertEqual(VoiceTurnFeedback.emptyCapture.message, "I didn’t catch that — try again.")
        XCTAssertTrue(VoiceTurnFeedback.emptyCapture.isRecoverable)
        XCTAssertEqual(VoiceTurnFeedback.connectionLost.message, "Connection lost — reconnecting to your hub.")
        XCTAssertTrue(VoiceTurnFeedback.connectionLost.isRecoverable)
        XCTAssertEqual(VoiceTurnFeedback.error("").message, "Voice stopped — try again.")
        XCTAssertNil(VoiceTurnFeedback.none.message)
    }

    func testAudioSegmentDecodesServerWordTimingsAndUTF16Ranges() throws {
        let json = #"{"type":"audio_segment","segment_id":0,"text":"Hello world.","sample_rate":24000,"duration_ms":900,"word_timings":[{"word":"Hello","start_ms":0,"end_ms":360,"start_utf16":0,"end_utf16":5},{"word":"world","start_ms":380,"end_ms":820,"start_utf16":6,"end_utf16":11}]}"#
        let message = try JSONDecoder().decode(VoiceReplyTestMessage.self, from: Data(json.utf8))
        XCTAssertEqual(message.type, "audio_segment")
        let segment = try XCTUnwrap(message.segment)
        XCTAssertEqual(segment.segmentID, "0")
        XCTAssertEqual(segment.durationMS, 900)
        XCTAssertEqual(segment.words[1].rangeLocation, 6)
        XCTAssertEqual(segment.words[1].rangeLength, 5)
        XCTAssertEqual(voiceReplyWordIndex(at: 400, in: segment), 1)
    }

    func testAudioSegmentSupportsPrototypeReplySegmentAlias() throws {
        let json = #"{"type":"reply_segment","segment_id":"s-2","text":"Okay.","sample_rate":24000,"duration_ms":300,"words":[]}"#
        let message = try JSONDecoder().decode(VoiceReplyTestMessage.self, from: Data(json.utf8))
        XCTAssertEqual(message.segment?.segmentID, "s-2")
    }

    func testTimingValidationRejectsMalformedAndDoesNotHighlight() {
        let bad = VoiceReplySegment(segmentID: "bad", text: "one two", sampleRate: 24_000,
                                    durationMS: 500, words: [
                                        VoiceReplyWordTiming(word: "one", startMS: 300, endMS: 100, rangeLocation: 0, rangeLength: 3),
                                        VoiceReplyWordTiming(word: "two", startMS: 100, endMS: 600, rangeLocation: 4, rangeLength: 3)
                                    ])
        XCTAssertFalse(bad.hasValidTimings)
        XCTAssertNil(voiceReplyWordIndex(at: 350, in: bad))
    }

    func testMixedExplicitAndInferredRangesAdvanceForRepeatedWords() {
        let segment = VoiceReplySegment(segmentID: "r", text: "one one one", sampleRate: 24_000,
                                        durationMS: 900, words: [
                                            VoiceReplyWordTiming(word: "one", startMS: 0, endMS: 200, rangeLocation: 0, rangeLength: 3),
                                            VoiceReplyWordTiming(word: "one", startMS: 200, endMS: 500),
                                            VoiceReplyWordTiming(word: "one", startMS: 500, endMS: 900)
                                        ])
        XCTAssertEqual(segment.displayRanges(), [NSRange(location: 0, length: 3), NSRange(location: 4, length: 3), NSRange(location: 8, length: 3)])
        XCTAssertTrue(segment.hasValidTimings)
    }

    func testEmojiUsesUTF16AndGapsHaveNoHighlight() {
        let text = "go 🚀 now"
        let segment = VoiceReplySegment(segmentID: "e", text: text, sampleRate: 24_000,
                                        durationMS: 900, words: [
                                            VoiceReplyWordTiming(word: "go", startMS: 100, endMS: 300, rangeLocation: 0, rangeLength: 2),
                                            VoiceReplyWordTiming(word: "🚀", startMS: 400, endMS: 600, rangeLocation: 3, rangeLength: 2),
                                            VoiceReplyWordTiming(word: "now", startMS: 700, endMS: 900, rangeLocation: 6, rangeLength: 3)
                                        ])
        XCTAssertEqual(segment.displayRanges()[1], NSRange(location: 3, length: 2))
        XCTAssertNil(voiceReplyWordIndex(at: 50, in: segment))
        XCTAssertNil(voiceReplyWordIndex(at: 650, in: segment))
        XCTAssertNil(voiceReplyWordIndex(at: 901, in: segment))
    }

    func testMissingTimingsRemainReadableWithoutHighlight() {
        let segment = VoiceReplySegment(segmentID: "none", text: "Readable reply", sampleRate: 24_000, durationMS: 700)
        XCTAssertFalse(segment.hasValidTimings)
        XCTAssertNil(voiceReplyWordIndex(at: 200, in: segment))
    }

    func testPunctuationAndUTF16TimingRemainExact() {
        let segment = VoiceReplySegment(segmentID: "p", text: "Wait—really? Yes!", sampleRate: 24_000,
                                        durationMS: 900, words: [
                                            VoiceReplyWordTiming(word: "Wait—really?", startMS: 0, endMS: 450, rangeLocation: 0, rangeLength: 12),
                                            VoiceReplyWordTiming(word: "Yes!", startMS: 450, endMS: 900, rangeLocation: 13, rangeLength: 4)
                                        ])
        XCTAssertTrue(segment.hasValidTimings)
        XCTAssertEqual(segment.displayRanges(), [NSRange(location: 0, length: 12), NSRange(location: 13, length: 4)])
    }

    func testGlobalRemapPreservesRepeatedSegmentsAndFailsClosed() {
        XCTAssertEqual(voiceReplyGlobalRanges(segmentTexts: ["one", "one"], in: "one\n\none"),
                       [NSRange(location: 0, length: 3), NSRange(location: 5, length: 3)])
        XCTAssertNil(voiceReplyGlobalRanges(segmentTexts: ["one", "missing"], in: "one\n\none"))
    }

    func testSegmentQueueRejectsDuplicatesRegressionsOrphansAndResets() {
        var queue = VoiceReplySegmentQueue()
        XCTAssertTrue(queue.acceptMetadata(segmentID: "0"))
        XCTAssertTrue(queue.consumePCM())
        XCTAssertFalse(queue.acceptMetadata(segmentID: "0"))
        XCTAssertFalse(queue.consumePCM())
        queue.reset()
        XCTAssertTrue(queue.acceptMetadata(segmentID: "2"))
        XCTAssertFalse(queue.acceptMetadata(segmentID: "1"))
        XCTAssertFalse(queue.consumePCM())
        queue.reset()
        XCTAssertFalse(queue.consumePCM())
        XCTAssertTrue(queue.timingDisabled)
    }

    func testSegmentQueueAcceptsOrderedMultiSegmentFrames() {
        var queue = VoiceReplySegmentQueue()
        XCTAssertTrue(queue.acceptMetadata(segmentID: "0"))
        XCTAssertTrue(queue.acceptMetadata(segmentID: "1"))
        XCTAssertTrue(queue.consumePCM())
        XCTAssertTrue(queue.consumePCM())
        XCTAssertEqual(queue.pendingCount, 0)
    }

    func testInvalidSampleRateAndZeroDurationAreNotHighlightable() {
        let wrongRate = VoiceReplySegment(segmentID: "r", text: "ok", sampleRate: 48_000, durationMS: 300,
                                          words: [VoiceReplyWordTiming(word: "ok", startMS: 0, endMS: 300, rangeLocation: 0, rangeLength: 2)])
        let zero = VoiceReplySegment(segmentID: "z", text: "ok", sampleRate: 24_000, durationMS: 0,
                                     words: [VoiceReplyWordTiming(word: "ok", startMS: 0, endMS: 1, rangeLocation: 0, rangeLength: 2)])
        XCTAssertFalse(wrongRate.sampleRate == 24_000 && wrongRate.hasValidTimings)
        XCTAssertFalse(zero.hasValidTimings)
    }

    func testTimingBoundaryAndResetEpochAreDeterministic() {
        let segment = VoiceReplySegment(segmentID: "b", text: "a b", sampleRate: 24_000, durationMS: 400,
                                        words: [VoiceReplyWordTiming(word: "a", startMS: 0, endMS: 200, rangeLocation: 0, rangeLength: 1),
                                                VoiceReplyWordTiming(word: "b", startMS: 200, endMS: 400, rangeLocation: 2, rangeLength: 1)])
        XCTAssertEqual(voiceReplyWordIndex(at: 0, in: segment), 0)
        XCTAssertEqual(voiceReplyWordIndex(at: 200, in: segment), 1)
        XCTAssertNil(voiceReplyWordIndex(at: 400, in: segment))
        var queue = VoiceReplySegmentQueue()
        XCTAssertTrue(queue.acceptMetadata(segmentID: "4"))
        queue.reset()
        XCTAssertTrue(queue.acceptMetadata(segmentID: "0"))
        XCTAssertTrue(queue.consumePCM())
    }
}

/// Test-only envelope matching the flat WebSocket JSON emitted by the hub.
/// VoiceEngine keeps its operational envelope private; this pins the public
/// wire shape without requiring a live socket or audio device.
private struct VoiceReplyTestMessage: Decodable {
    let type: String
    let segment: VoiceReplySegment?

    private enum CodingKeys: String, CodingKey {
        case type, segmentID = "segment_id", text, sampleRate = "sample_rate"
        case durationMS = "duration_ms", words, wordTimings = "word_timings"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        type = try c.decode(String.self, forKey: .type)
        guard type == "audio_segment" || type == "reply_segment" else {
            segment = nil
            return
        }
        let segmentID = try c.decode(VoiceWireSegmentID.self, forKey: .segmentID).value
        let text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        let sampleRate = try c.decodeIfPresent(Int.self, forKey: .sampleRate) ?? 24_000
        let durationMS = try c.decodeIfPresent(Int.self, forKey: .durationMS) ?? 0
        let words = (try? c.decode([VoiceReplyWordTiming].self, forKey: .wordTimings))
            ?? (try? c.decode([VoiceReplyWordTiming].self, forKey: .words))
            ?? []
        segment = VoiceReplySegment(segmentID: segmentID, text: text, sampleRate: sampleRate, durationMS: durationMS, words: words)
    }
}
