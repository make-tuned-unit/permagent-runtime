// Standalone watch voice — the facts the phone hands the watch once, so the
// watch can hold a spoken conversation with the hub's `/voice` socket over
// home Wi-Fi without the phone (Astra ruling 2026-10-13, "Wi-Fi wrist
// conversation").
//
// The phone mints a watch-scoped credential from the hub and pushes this
// payload through WCSession application context; the watch keeps it in its
// Keychain and never shows it. Foundation-only so both targets (and the
// logic tests) compile it.
//
// The hub mints it at `POST /api/devices/{phone_id}/watch-credential`
// (voice-gateway lane, wq-reports/build/watch-voice-gateway-20261013.md):
//   {"hub_url_lan":"wss://192.168.1.20:3443","voice_token":"…",
//    "conversation_id":"…","expires_at":"…","cert_sha256":"AA:BB:…"}
// The phone re-encodes it in the handoff shape below (cert_fingerprint).

import Foundation

struct WatchVoiceCredential: Codable, Equatable, Sendable {
    /// Base URL of the LAN voice gateway, e.g. `wss://192.168.1.20:3443`.
    var hubURL: String
    /// Watch-scoped bearer for `/voice`. Never logged, never shown.
    var voiceToken: String
    /// The shared chat session — phone, Mac and watch are one conversation.
    var conversationId: String
    /// SHA-256 of the gateway's leaf certificate (hex, colons optional).
    /// Nil only for a plain-HTTP loopback rehearsal hub.
    var certFingerprint: String?
    /// ISO-8601, informational; the hub enforces expiry.
    var expiresAt: String?
    /// DEBUG rehearsal against today's `/voice`, which reads the token from
    /// the query string only. Production credentials always use the header.
    var queryTokenAuth: Bool?

    enum CodingKeys: String, CodingKey {
        case hubURL = "hub_url_lan"
        case voiceToken = "voice_token"
        case conversationId = "conversation_id"
        case certFingerprint = "cert_fingerprint"
        case expiresAt = "expires_at"
        case queryTokenAuth = "query_token_auth"
    }

    /// WCSession application-context key. A context carrying
    /// `clearKey: true` (and no credential) tells the watch to forget it.
    static let contextKey = "watchVoice.v1"
    static let clearKey = "watchVoice.clear"

    var isUsable: Bool {
        guard !voiceToken.isEmpty, !conversationId.isEmpty,
              let url = URL(string: hubURL), let scheme = url.scheme?.lowercased(),
              url.host != nil else { return false }
        if scheme == "https" || scheme == "wss" {
            return Self.normalizedFingerprint(certFingerprint) != nil
        }
        // Plain ws/http only for loopback rehearsal; LAN traffic must be TLS.
        return (scheme == "http" || scheme == "ws") && Self.isLoopback(url.host ?? "")
    }

    /// `ws(s)://<hub>/voice?session_id=…&client=watch_voice` — the token
    /// rides the Authorization header unless `queryTokenAuth` (rehearsal).
    func voiceURL() -> URL? {
        guard var comps = URLComponents(string: hubURL) else { return nil }
        let scheme = comps.scheme?.lowercased()
        comps.scheme = (scheme == "https" || scheme == "wss") ? "wss" : "ws"
        comps.path = "/voice"
        var items = [
            URLQueryItem(name: "session_id", value: conversationId),
            URLQueryItem(name: "client", value: "watch_voice"),
        ]
        if queryTokenAuth == true {
            items.append(URLQueryItem(name: "token", value: voiceToken))
        }
        comps.queryItems = items
        return comps.url
    }

    /// Lowercase hex with separators removed; nil unless it is 32 bytes.
    static func normalizedFingerprint(_ raw: String?) -> String? {
        guard let raw else { return nil }
        let hex = raw.lowercased().filter { $0.isHexDigit }
        let stripped = raw.filter { !$0.isWhitespace && $0 != ":" }
        guard hex.count == 64, stripped.count == 64 else { return nil }
        return hex
    }

    static func isLoopback(_ host: String) -> Bool {
        host == "127.0.0.1" || host == "localhost" || host == "::1"
    }
}

/// The watch's conversation states, in the order the ruling names them.
enum WatchVoicePhase: Equatable, Sendable {
    case idle
    case connecting
    case ready
    case listening
    case transcribing
    case thinking
    case speaking
    case failed(String)

    var label: String {
        switch self {
        case .idle: return "Ready"
        case .connecting: return "Connecting…"
        case .ready: return "Ready"
        case .listening: return "Listening…"
        case .transcribing: return "Transcribing…"
        case .thinking: return "Thinking…"
        case .speaking: return "Speaking…"
        case .failed: return "Not connected"
        }
    }

    /// A turn is in flight on the hub: the stop was sent and no terminal
    /// frame has arrived. A disconnect here must NOT resubmit audio.
    var awaitingHub: Bool {
        self == .transcribing || self == .thinking || self == .speaking
    }
}

/// Bounds from the ruling. Pure so tests can read them.
enum WatchVoiceLimits {
    static let connect: TimeInterval = 8
    static let recording: TimeInterval = 45
    static let noProgress: TimeInterval = 30
    /// From `stop` to reply_end + playback drained.
    static let turn: TimeInterval = 120
    /// Silence that submits once speech was heard (WatchEndpoint.chat).
    static let endpoint = WatchEndpoint.chat
    /// Automatic reconnects before the watch shows a failure and offers
    /// the phone relay.
    static let reconnectAttempts = 2
}

/// Reply audio may reopen the mic only when the hub said the reply is over
/// AND the speaker has rendered every buffer (ruling decision 2).
struct WatchReplyGate: Equatable, Sendable {
    private(set) var replyEnded = false
    private(set) var pendingBuffers = 0

    mutating func reset() { replyEnded = false; pendingBuffers = 0 }
    mutating func scheduled() { pendingBuffers += 1 }
    mutating func rendered() { pendingBuffers = max(0, pendingBuffers - 1) }
    mutating func endReceived() { replyEnded = true }
    mutating func dropAudio() { pendingBuffers = 0 }
    var complete: Bool { replyEnded && pendingBuffers == 0 }
}
