// Chat stream + transcript logic, pure and UI-free.
//
// Everything here is deliberately Foundation-only so the PermagentTests
// target can compile this file directly (it depends on no app target — see
// project.yml). These units carry the chat behaviors that regressed once and
// must not regress again:
//
//   * segment spacing — tool activity between text segments is a PARAGRAPH,
//     never a glue joint ("…works.Let me dig deeper…", reported 2026-08-06);
//   * connection-loss classification — locking the phone mid-reply is
//     "stopped watching", never "reply failed";
//   * transcript restoration — stored multi-segment messages rehydrate with
//     the same paragraph breaks the live stream draws.

import Foundation

// ── The transcript's atoms ───────────────────────────────────────────────────

struct ChatBubble: Identifiable {
    let id = UUID()
    let role: String   // "user" | "assistant"
    var text: String
    var thinking: String = ""
}

/// One streamed slice of the reply: answer `text`, reasoning `thinking`,
/// and/or an approval request that has parked the agent. `segmentBreak` is
/// true when tool activity separated this slice from the previous one — the
/// renderer owes the reader a paragraph break there.
struct ReplyDelta: Equatable {
    let text: String
    let thinking: String
    let awaitingApproval: AwaitingApproval?
    var segmentBreak: Bool = false
}

struct AwaitingApproval: Equatable { let toolName: String; let actionId: String? }

// ── Wire shapes (mirror the daemon's serde) ──────────────────────────────────

/// Thinking blocks carry `thinking`; answer blocks carry `text`; approval
/// blocks carry their action details in `data` (and older hubs may put them
/// directly on the block).
struct ReplyContent: Codable {
    let type: String
    let text: String?
    let thinking: String?
    let id: String?
    let toolName: String?
    let data: ReplyAction?

    init(
        type: String,
        text: String? = nil,
        thinking: String? = nil,
        id: String? = nil,
        toolName: String? = nil,
        data: ReplyAction? = nil
    ) {
        self.type = type
        self.text = text
        self.thinking = thinking
        self.id = id
        self.toolName = toolName
        self.data = data
    }
}

struct ReplyAction: Codable {
    let id: String?
    let toolName: String?
}

struct ReplyMeta: Codable { let userVisible: Bool; let agentVisible: Bool }

struct ReplyMessage: Codable {
    let role: String
    let created: Int
    let content: [ReplyContent]
    let metadata: ReplyMeta?
}

struct ReplyRequest: Encodable {
    let user_message: ReplyMessage
    let session_id: String
}

struct ReplyEvent: Decodable {
    let type: String
    let message: ReplyMessage?
    let error: String?
}

// ── The SSE reply parser ─────────────────────────────────────────────────────

/// Consumes raw SSE lines from POST /reply and produces deltas, arming a
/// segment break whenever tool activity separates one text segment from the
/// next. Tool REQUESTS ride assistant frames; tool RESULTS come back as
/// user-role frames — both end a segment.
struct ReplyStreamParser {
    enum Outcome: Equatable {
        case none
        case delta(ReplyDelta)
        case finish
        case error(String)
    }

    private var segmentBreakPending = false

    mutating func consume(line: String) -> Outcome {
        guard line.hasPrefix("data: ") else { return .none }
        guard let data = String(line.dropFirst(6)).data(using: .utf8),
              let event = try? JSONDecoder().decode(ReplyEvent.self, from: data)
        else { return .none }

        switch event.type {
        case "Message":
            guard let m = event.message else { return .none }
            if m.role == "assistant" {
                let t = m.content.compactMap(\.text).joined()
                let th = m.content.compactMap(\.thinking).joined()
                let hasToolActivity = m.content.contains {
                    $0.type == "toolRequest" || $0.type == "toolResponse"
                        || $0.type == "toolConfirmationRequest"
                }
                let action = m.content.first { $0.type == "actionRequired" }
                let approval = action.map {
                    AwaitingApproval(
                        toolName: $0.toolName ?? $0.data?.toolName ?? "a tool",
                        actionId: $0.id ?? $0.data?.id
                    )
                }
                var out = Outcome.none
                if !t.isEmpty || !th.isEmpty || approval != nil {
                    out = .delta(ReplyDelta(
                        text: t,
                        thinking: th,
                        awaitingApproval: approval,
                        segmentBreak: segmentBreakPending
                    ))
                    segmentBreakPending = false
                }
                if hasToolActivity { segmentBreakPending = true }
                return out
            }
            if m.role == "user",
               m.content.contains(where: { $0.type == "toolResponse" }) {
                segmentBreakPending = true
            }
            return .none
        case "Finish":
            return .finish
        case "Error":
            return .error(event.error ?? "The hub reported an unknown error.")
        default:
            return .none
        }
    }
}

// ── The streaming accumulator ────────────────────────────────────────────────

/// Applies streamed deltas to one assistant turn, inserting the paragraph
/// break the segment boundary owes the reader.
///
/// The break is tracked PER CHANNEL: a boundary announced by a thinking-only
/// delta must still separate the NEXT text from the previous text. (A single
/// pending flag consumed by whichever delta arrived first was the bug: after
/// text → tool → thinking → text, the second text glued straight onto the
/// first.)
struct AssistantAccumulator {
    private(set) var text = ""
    private(set) var thinking = ""
    private var textBreakPending = false
    private var thinkingBreakPending = false

    mutating func apply(_ delta: ReplyDelta) {
        if delta.segmentBreak {
            textBreakPending = true
            thinkingBreakPending = true
        }
        if !delta.text.isEmpty {
            if textBreakPending && !text.isEmpty { text += "\n\n" }
            text += delta.text
            textBreakPending = false
        }
        if !delta.thinking.isEmpty {
            if thinkingBreakPending && !thinking.isEmpty { thinking += "\n\n" }
            thinking += delta.thinking
            thinkingBreakPending = false
        }
    }

    /// The approval notice replaces whatever was accumulating; a later real
    /// delta starts the answer fresh.
    mutating func reset() {
        text = ""
        thinking = ""
        textBreakPending = false
        thinkingBreakPending = false
    }
}

// ── Stored-transcript restoration ────────────────────────────────────────────

struct StoredContent: Decodable {
    let type: String
    let text: String?
    let thinking: String?

    init(type: String, text: String? = nil, thinking: String? = nil) {
        self.type = type
        self.text = text
        self.thinking = thinking
    }
}

struct StoredMessage: Decodable {
    let role: String
    let content: [StoredContent]

    init(role: String, content: [StoredContent]) {
        self.role = role
        self.content = content
    }
}

enum ChatTranscript {
    /// Stored conversation → chat bubbles. Tool traffic is dropped — a
    /// resumed thread reads the way it read when it was live. Distinct text
    /// blocks within one stored message are distinct segments (the model
    /// spoke, used a tool, spoke again) — joined with a paragraph break,
    /// never glued.
    static func bubbles(from conversation: [StoredMessage]) -> [ChatBubble] {
        conversation.compactMap { m in
            guard m.role == "user" || m.role == "assistant" else { return nil }
            let text = m.content.compactMap(\.text)
                .filter { !$0.isEmpty }
                .joined(separator: "\n\n")
            let thinking = m.content.compactMap(\.thinking)
                .filter { !$0.isEmpty }
                .joined(separator: "\n\n")
            guard !text.isEmpty || !thinking.isEmpty else { return nil }
            return ChatBubble(role: m.role, text: text, thinking: thinking)
        }
    }
}

// ── Connection-loss classification ───────────────────────────────────────────

enum ChatConnection {
    /// The network failures that mean "this device stopped watching", not
    /// "the hub failed". Locking the phone or switching apps mid-stream
    /// surfaces as any of these depending on how iOS tore the socket down —
    /// all of them get the quiet catch-up path, never a scary error.
    static func isLoss(_ error: Error) -> Bool {
        guard let urlError = error as? URLError else { return false }
        switch urlError.code {
        case .networkConnectionLost, .notConnectedToInternet, .timedOut, .cancelled,
             .backgroundSessionWasDisconnected, .dataNotAllowed, .internationalRoamingOff,
             .callIsActive:
            return true
        default:
            return false
        }
    }
}

// ── The empty-state greeting ─────────────────────────────────────────────────

enum ChatGreeting {
    static func forHour(_ hour: Int) -> String {
        switch hour {
        case 5..<12: return "Morning thoughts?"
        case 12..<17: return "What's on deck?"
        case 17..<22: return "Evening plans?"
        default: return "Moonlit chat?"
        }
    }
}
