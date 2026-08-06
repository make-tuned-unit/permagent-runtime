// Regression tests for the chat stream + transcript logic (ChatStream.swift).
//
// These pin the behaviors that shipped broken once and must stay fixed:
//   1. Segment spacing — tool activity between text segments produces a
//      paragraph break, never "…harness works.Let me dig deeper…".
//   2. The per-channel break — a boundary announced by a thinking-only delta
//      still separates the NEXT text from the previous text.
//   3. Restored history joins stored segments with the same paragraph break.
//   4. Locking the phone mid-stream classifies as "stopped watching", never
//      as a failed reply.
//
// The target compiles ChatStream.swift directly (no app dependency) — keep
// everything under test Foundation-only.

import XCTest

final class ReplyStreamParserTests: XCTestCase {
    private func frame(_ json: String) -> String { "data: \(json)" }

    private func assistantText(_ text: String) -> String {
        frame(#"{"type":"Message","message":{"role":"assistant","created":0,"content":[{"type":"text","text":"\#(text)"}]}}"#)
    }

    private func assistantThinking(_ thinking: String) -> String {
        frame(#"{"type":"Message","message":{"role":"assistant","created":0,"content":[{"type":"thinking","thinking":"\#(thinking)"}]}}"#)
    }

    private func assistantToolRequest() -> String {
        frame(#"{"type":"Message","message":{"role":"assistant","created":0,"content":[{"type":"toolRequest","id":"t1"}]}}"#)
    }

    private func userToolResponse() -> String {
        frame(#"{"type":"Message","message":{"role":"user","created":0,"content":[{"type":"toolResponse","id":"t1"}]}}"#)
    }

    private func delta(_ outcome: ReplyStreamParser.Outcome) -> ReplyDelta? {
        if case .delta(let d) = outcome { return d }
        return nil
    }

    func testDeltasWithinOneSegmentCarryNoBreak() {
        var p = ReplyStreamParser()
        XCTAssertEqual(delta(p.consume(line: assistantText("I need")))?.segmentBreak, false)
        XCTAssertEqual(delta(p.consume(line: assistantText(" to see")))?.segmentBreak, false)
    }

    func testToolRequestArmsABreakForTheNextText() {
        var p = ReplyStreamParser()
        _ = p.consume(line: assistantText("compare it to how the harness works."))
        XCTAssertEqual(p.consume(line: assistantToolRequest()), .none)
        let next = delta(p.consume(line: assistantText("Let me dig deeper")))
        XCTAssertEqual(next?.segmentBreak, true, "the first text after tool activity owes the reader a paragraph")
        // …and the break is consumed, not sticky.
        XCTAssertEqual(delta(p.consume(line: assistantText(" into Prime")))?.segmentBreak, false)
    }

    func testUserRoleToolResponseAlsoArmsABreak() {
        var p = ReplyStreamParser()
        _ = p.consume(line: assistantText("first segment"))
        XCTAssertEqual(p.consume(line: userToolResponse()), .none)
        XCTAssertEqual(delta(p.consume(line: assistantText("second segment")))?.segmentBreak, true)
    }

    func testMixedFrameYieldsTextThenArmsBreak() {
        // One frame carrying trailing text AND a tool request: the text belongs
        // to the CURRENT segment; the break applies only after.
        var p = ReplyStreamParser()
        let mixed = frame(#"{"type":"Message","message":{"role":"assistant","created":0,"content":[{"type":"text","text":"done."},{"type":"toolRequest","id":"t1"}]}}"#)
        XCTAssertEqual(delta(p.consume(line: mixed))?.segmentBreak, false)
        XCTAssertEqual(delta(p.consume(line: assistantText("Next.")))?.segmentBreak, true)
    }

    func testActionRequiredMapsToAwaitingApproval() {
        var p = ReplyStreamParser()
        let action = frame(#"{"type":"Message","message":{"role":"assistant","created":0,"content":[{"type":"actionRequired","id":"a1","toolName":"shell"}]}}"#)
        let d = delta(p.consume(line: action))
        XCTAssertEqual(d?.awaitingApproval?.toolName, "shell")
        XCTAssertEqual(d?.awaitingApproval?.actionId, "a1")
    }

    func testTerminalFrames() {
        var p = ReplyStreamParser()
        XCTAssertEqual(p.consume(line: frame(#"{"type":"Finish","reason":"stop"}"#)), .finish)
        XCTAssertEqual(
            p.consume(line: frame(#"{"type":"Error","error":"boom"}"#)),
            .error("boom")
        )
    }

    func testJunkAndNonDataLinesAreIgnored() {
        var p = ReplyStreamParser()
        XCTAssertEqual(p.consume(line: ""), .none)
        XCTAssertEqual(p.consume(line: "event: message"), .none)
        XCTAssertEqual(p.consume(line: "data: not json"), .none)
        XCTAssertEqual(p.consume(line: frame(#"{"type":"Ping"}"#)), .none)
    }
}

final class AssistantAccumulatorTests: XCTestCase {
    private func d(text: String = "", thinking: String = "", brk: Bool = false) -> ReplyDelta {
        ReplyDelta(text: text, thinking: thinking, awaitingApproval: nil, segmentBreak: brk)
    }

    func testSegmentsJoinWithAParagraphNeverGlued() {
        var acc = AssistantAccumulator()
        acc.apply(d(text: "compare it to how the harness works."))
        acc.apply(d(text: "Let me dig deeper", brk: true))
        XCTAssertEqual(acc.text, "compare it to how the harness works.\n\nLet me dig deeper")
    }

    func testNoLeadingBreakOnTheFirstText() {
        var acc = AssistantAccumulator()
        acc.apply(d(text: "hello", brk: true))
        XCTAssertEqual(acc.text, "hello")
    }

    func testBreakSurvivesAThinkingOnlyDelta() {
        // The regression this type exists for: text → tool → thinking → text
        // must still break the two texts apart, even though the thinking delta
        // consumed the flag first.
        var acc = AssistantAccumulator()
        acc.apply(d(text: "first segment."))
        acc.apply(d(thinking: "let me check something", brk: true))
        acc.apply(d(text: "Second segment."))
        XCTAssertEqual(acc.text, "first segment.\n\nSecond segment.")
        XCTAssertEqual(acc.thinking, "let me check something")
    }

    func testThinkingChannelBreaksIndependently() {
        var acc = AssistantAccumulator()
        acc.apply(d(thinking: "step one"))
        acc.apply(d(thinking: "step two", brk: true))
        XCTAssertEqual(acc.thinking, "step one\n\nstep two")
    }

    func testPlainDeltasStillConcatenateExactly() {
        // Token-level deltas within a segment must never gain separators.
        var acc = AssistantAccumulator()
        acc.apply(d(text: "I need"))
        acc.apply(d(text: " to see"))
        acc.apply(d(text: " what you're looking at."))
        XCTAssertEqual(acc.text, "I need to see what you're looking at.")
    }

    func testResetClearsEverything() {
        var acc = AssistantAccumulator()
        acc.apply(d(text: "notice", brk: true))
        acc.reset()
        acc.apply(d(text: "fresh"))
        XCTAssertEqual(acc.text, "fresh")
        XCTAssertEqual(acc.thinking, "")
    }
}

final class ChatTranscriptTests: XCTestCase {
    func testStoredSegmentsJoinWithParagraphBreaks() {
        let bubbles = ChatTranscript.bubbles(from: [
            StoredMessage(role: "assistant", content: [
                StoredContent(type: "text", text: "harness works."),
                StoredContent(type: "toolRequest"),
                StoredContent(type: "text", text: "Let me dig deeper"),
            ]),
        ])
        XCTAssertEqual(bubbles.count, 1)
        XCTAssertEqual(bubbles[0].text, "harness works.\n\nLet me dig deeper")
    }

    func testToolAndSystemRolesAreDropped() {
        let bubbles = ChatTranscript.bubbles(from: [
            StoredMessage(role: "user", content: [StoredContent(type: "text", text: "hi")]),
            StoredMessage(role: "tool", content: [StoredContent(type: "text", text: "raw tool output")]),
            StoredMessage(role: "assistant", content: [StoredContent(type: "text", text: "hello")]),
        ])
        XCTAssertEqual(bubbles.map(\.role), ["user", "assistant"])
    }

    func testContentlessMessagesAreDropped() {
        let bubbles = ChatTranscript.bubbles(from: [
            StoredMessage(role: "assistant", content: [StoredContent(type: "toolRequest")]),
            StoredMessage(role: "assistant", content: [StoredContent(type: "text", text: "")]),
        ])
        XCTAssertTrue(bubbles.isEmpty)
    }

    func testThinkingBlocksJoinLikeTextBlocks() {
        let bubbles = ChatTranscript.bubbles(from: [
            StoredMessage(role: "assistant", content: [
                StoredContent(type: "thinking", thinking: "first"),
                StoredContent(type: "thinking", thinking: "second"),
            ]),
        ])
        XCTAssertEqual(bubbles.count, 1)
        XCTAssertEqual(bubbles[0].thinking, "first\n\nsecond")
    }
}

final class ChatConnectionTests: XCTestCase {
    func testLockScreenTeardownErrorsAreStoppedWatchingNotFailure() {
        // Every way iOS actually severs the socket when the phone locks or the
        // app backgrounds. A new teardown mode regressing to a scary error is
        // exactly what this list guards against.
        let losses: [URLError.Code] = [
            .networkConnectionLost, .notConnectedToInternet, .timedOut, .cancelled,
            .backgroundSessionWasDisconnected, .dataNotAllowed, .internationalRoamingOff,
            .callIsActive,
        ]
        for code in losses {
            XCTAssertTrue(ChatConnection.isLoss(URLError(code)), "\(code) must route to catch-up")
        }
    }

    func testHubSideFailuresAreRealErrors() {
        // These mean the hub was never reached or answered wrongly — the user
        // must see them, not a silent catch-up that swallows their message.
        let failures: [URLError.Code] = [
            .cannotFindHost, .dnsLookupFailed, .cannotConnectToHost, .badServerResponse,
        ]
        for code in failures {
            XCTAssertFalse(ChatConnection.isLoss(URLError(code)), "\(code) must surface")
        }
        struct Other: Error {}
        XCTAssertFalse(ChatConnection.isLoss(Other()))
    }
}

final class ChatGreetingTests: XCTestCase {
    func testGreetingBuckets() {
        XCTAssertEqual(ChatGreeting.forHour(5), "Morning thoughts?")
        XCTAssertEqual(ChatGreeting.forHour(11), "Morning thoughts?")
        XCTAssertEqual(ChatGreeting.forHour(12), "What's on deck?")
        XCTAssertEqual(ChatGreeting.forHour(16), "What's on deck?")
        XCTAssertEqual(ChatGreeting.forHour(17), "Evening plans?")
        XCTAssertEqual(ChatGreeting.forHour(21), "Evening plans?")
        XCTAssertEqual(ChatGreeting.forHour(22), "Moonlit chat?")
        XCTAssertEqual(ChatGreeting.forHour(0), "Moonlit chat?")
        XCTAssertEqual(ChatGreeting.forHour(4), "Moonlit chat?")
    }
}
