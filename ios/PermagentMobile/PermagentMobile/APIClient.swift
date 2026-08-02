// Daemon API client — the phone is a thin client of the hub (MULTI_DEVICE.md).
// Same bearer pairing model as the web: one token, stored in the Keychain.

import Foundation

struct HubConfig: Codable, Equatable {
    var baseURL: URL       // http://<magicdns>:3001
    var token: String      // daemon_token (the pairing secret)
}

enum APIError: Error { case unauthorized, badStatus(Int), notPaired, dictationUnavailable }

actor APIClient {
    static let shared = APIClient()
    private var config: HubConfig?

    func pair(_ config: HubConfig) {
        self.config = config
        KeychainStore.save(config)
    }

    func loadSavedPairing() {
        if config == nil { config = KeychainStore.load() }
    }

    var isPaired: Bool { config != nil }

    /// The stored pairing (base URL + bearer token) — for consumers that open
    /// their own connections (the /voice WebSocket does query-param auth).
    func currentConfig() -> HubConfig? {
        loadSavedPairing()
        return config
    }

    func get<T: Decodable>(_ path: String, as type: T.Type) async throws -> T {
        try await request(path, method: "GET", body: Optional<Int>.none)
    }

    func post<T: Decodable, B: Encodable>(_ path: String, body: B, as type: T.Type) async throws -> T {
        try await request(path, method: "POST", body: body)
    }

    /// Fire-and-refresh command POST for endpoints that answer 204/No Content or
    /// a body we don't need (schedule run_now / pause / unpause / kill). Sends no
    /// request body and discards the response; the caller reloads to reflect the
    /// new state. Using `post(...)` here would throw — it tries to JSON-decode an
    /// empty 204 body.
    func send(_ path: String, method: String = "POST") async throws {
        guard let config else { throw APIError.notPaired }
        var req = URLRequest(url: config.baseURL.appendingPathComponent(path))
        req.httpMethod = method
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        let (_, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
        if http.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }
    }

    /// Multipart upload of a recorded clip to the local dictation model
    /// (POST /api/dictation/transcribe → `{ text }`). The daemon reads the first
    /// multipart field regardless of name; we send it as `audio`. A 503 surfaces
    /// as `APIError.badStatus(503)` — "no local dictation model configured" — so
    /// the Notes composer can explain the setup gap rather than read as a crash.
    func transcribe(_ audio: Data, filename: String = "dictation.wav", mimeType: String = "audio/wav") async throws -> String {
        guard let config else { throw APIError.notPaired }
        let boundary = "PermagentBoundary-\(UUID().uuidString)"
        var req = URLRequest(url: config.baseURL.appendingPathComponent("/api/dictation/transcribe"))
        req.httpMethod = "POST"
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        req.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")
        var body = Data()
        body.appendString("--\(boundary)\r\n")
        body.appendString("Content-Disposition: form-data; name=\"audio\"; filename=\"\(filename)\"\r\n")
        body.appendString("Content-Type: \(mimeType)\r\n\r\n")
        body.append(audio)
        body.appendString("\r\n--\(boundary)--\r\n")
        req.httpBody = body
        let (data, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
        if http.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }
        struct TranscribeResponse: Decodable { let text: String }
        return try JSONDecoder().decode(TranscribeResponse.self, from: data).text
    }

    private func request<T: Decodable, B: Encodable>(
        _ path: String, method: String, body: B?
    ) async throws -> T {
        guard let config else { throw APIError.notPaired }
        var req = URLRequest(url: config.baseURL.appendingPathComponent(path))
        req.httpMethod = method
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try JSONEncoder().encode(body)
        }
        let (data, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
        if http.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }
        return try JSONDecoder().decode(T.self, from: data)
    }

    /// Live events over the hub's /events WebSocket (public route).
    nonisolated func eventStream() async -> AsyncStream<DaemonEvent>? {
        await loadSavedPairing()
        guard let config = await self.config else { return nil }
        var comps = URLComponents(url: config.baseURL, resolvingAgainstBaseURL: false)!
        comps.scheme = comps.scheme == "https" ? "wss" : "ws"
        comps.path = "/events"
        let task = URLSession.shared.webSocketTask(with: comps.url!)
        task.resume()
        return AsyncStream { continuation in
            Task {
                do {
                    while !Task.isCancelled {
                        let message = try await task.receive()
                        if case .string(let text) = message,
                           let data = text.data(using: .utf8),
                           let event = try? JSONDecoder().decode(DaemonEvent.self, from: data) {
                            continuation.yield(event)
                        }
                    }
                } catch {
                    continuation.finish()
                }
            }
            continuation.onTermination = { _ in task.cancel(with: .goingAway, reason: nil) }
        }
    }

    /// Ask Henry on the hub (POST /reply → an SSE stream of MessageEvents).
    /// Yields the assistant's reply as it arrives — `text` (the answer) and
    /// `thinking` (the reasoning, if an extended-thinking model). The hub does
    /// the work; this device only relays the ask and renders the reply. Matches
    /// the daemon's `data: {json}\n\n` framing and `type`-tagged MessageEvent enum.
    nonisolated func replyStream(_ text: String, sessionId: String) -> AsyncThrowingStream<ReplyDelta, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    await loadSavedPairing()
                    guard let config = await self.config else {
                        continuation.finish(throwing: APIError.notPaired); return
                    }
                    var req = URLRequest(url: config.baseURL.appendingPathComponent("/reply"))
                    req.httpMethod = "POST"
                    req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
                    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
                    req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    let msg = ReplyMessage(
                        role: "user",
                        created: Int(Date().timeIntervalSince1970),
                        content: [ReplyContent(type: "text", text: text, thinking: nil)],
                        metadata: ReplyMeta(userVisible: true, agentVisible: true)
                    )
                    req.httpBody = try JSONEncoder().encode(ReplyRequest(user_message: msg, session_id: sessionId))

                    let (bytes, resp) = try await URLSession.shared.bytes(for: req)
                    guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
                    if http.statusCode == 401 { throw APIError.unauthorized }
                    guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }

                    for try await line in bytes.lines {
                        guard line.hasPrefix("data: ") else { continue }
                        let json = String(line.dropFirst(6))
                        guard let data = json.data(using: .utf8),
                              let event = try? JSONDecoder().decode(ReplyEvent.self, from: data)
                        else { continue }
                        switch event.type {
                        case "Message":
                            if let m = event.message, m.role == "assistant" {
                                let t = m.content.compactMap(\.text).joined()
                                let th = m.content.compactMap(\.thinking).joined()
                                if !t.isEmpty || !th.isEmpty {
                                    continuation.yield(ReplyDelta(text: t, thinking: th))
                                }
                            }
                        case "Finish":
                            continuation.finish(); return
                        case "Error":
                            continuation.finish(throwing: APIError.badStatus(422)); return
                        default:
                            break
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }
}

// ── Dictation → note → to-dos (the phone's capture path) ─────────────────────
//
// Three existing hub surfaces, stitched together for capture-on-the-go:
//   POST /api/dictation/transcribe        — the hub's LOCAL Whisper (multipart WAV)
//   POST /api/projects/{id}/notes         — the note row + Brain index (the note
//                                           contract — source/description — is
//                                           enforced daemon-side)
//   POST /api/projects/{id}/cards         — confirmed to-dos as board cards
//
// The phone records; the hub transcribes. No cloud STT, ever.

/// A project row for the picker (camelCase like every projects/cards route).
struct ProjectSummary: Decodable, Identifiable, Equatable {
    let id: String
    let slug: String
    let name: String
    let description: String
    let status: String
}

/// A board column (used only to check "does this project have a board at all").
struct BoardColumn: Decodable, Identifiable {
    let id: String
    let name: String
    let position: Int
}

/// The created note row (snake_case — ProjectNote has no serde rename).
struct CreatedNote: Decodable {
    let id: String
    let project_id: String
}

extension APIClient {
    func projects() async throws -> [ProjectSummary] {
        try await get("/api/projects", as: [ProjectSummary].self)
    }

    func columns(projectId: String) async throws -> [BoardColumn] {
        try await get("/api/projects/\(projectId)/columns", as: [BoardColumn].self)
    }

    func createNote(projectId: String, title: String?, body: String) async throws -> CreatedNote {
        struct Req: Encodable { let title: String?; let body: String }
        return try await post("/api/projects/\(projectId)/notes",
                              body: Req(title: title, body: body), as: CreatedNote.self)
    }

    /// One confirmed to-do → a standard card. `columnId` nil lets the daemon
    /// place it in the project's first column; tagged so its origin is queryable.
    func createCard(projectId: String, title: String, noteId: String) async throws {
        struct Meta: Encodable { let source: String; let note_id: String }
        struct Req: Encodable {
            let title: String
            let cardType: String
            let createdBy: String
            let metadataJson: Meta
        }
        struct Resp: Decodable { let id: String }
        _ = try await post(
            "/api/projects/\(projectId)/cards",
            body: Req(title: title, cardType: "standard", createdBy: "user",
                      metadataJson: Meta(source: "permagent.note.dictation", note_id: noteId)),
            as: Resp.self
        )
    }

    /// Upload a WAV clip to the hub's local-Whisper transcriber. 503 means the
    /// hub has no dictation model configured — surfaced as `.dictationUnavailable`
    /// so the UI can show a setup hint instead of a generic failure.
    func transcribe(wav: Data) async throws -> String {
        guard let config else { throw APIError.notPaired }
        let boundary = "permagent-\(UUID().uuidString)"
        var req = URLRequest(url: config.baseURL.appendingPathComponent("/api/dictation/transcribe"))
        req.httpMethod = "POST"
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        req.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")
        var body = Data()
        body.append("--\(boundary)\r\n".data(using: .utf8)!)
        body.append("Content-Disposition: form-data; name=\"audio\"; filename=\"dictation.wav\"\r\n".data(using: .utf8)!)
        body.append("Content-Type: audio/wav\r\n\r\n".data(using: .utf8)!)
        body.append(wav)
        body.append("\r\n--\(boundary)--\r\n".data(using: .utf8)!)
        req.httpBody = body

        let (data, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
        if http.statusCode == 401 { throw APIError.unauthorized }
        if http.statusCode == 503 { throw APIError.dictationUnavailable }
        guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }
        struct Resp: Decodable { let text: String }
        return try JSONDecoder().decode(Resp.self, from: data).text
    }
}

// ── /reply request + event shapes (mirror the daemon's serde) ────────────────

/// One streamed slice of the reply: answer `text` and/or reasoning `thinking`.
struct ReplyDelta { let text: String; let thinking: String }

/// A content block is `{ type, text?, thinking? }`; tool blocks have neither and
/// decode to nil. Thinking blocks carry `thinking`; answer blocks carry `text`.
private struct ReplyContent: Codable { let type: String; let text: String?; let thinking: String? }
private struct ReplyMeta: Codable { let userVisible: Bool; let agentVisible: Bool }
private struct ReplyMessage: Codable {
    let role: String
    let created: Int
    let content: [ReplyContent]
    let metadata: ReplyMeta
}
private struct ReplyRequest: Encodable {
    let user_message: ReplyMessage
    let session_id: String
}
private struct ReplyEvent: Decodable {
    let type: String
    let message: ReplyMessage?
    let error: String?
}

/// A stable per-install chat session id so the conversation persists across
/// launches. The hub creates the session lazily on first reply (get_agent).
enum MobileSession {
    private static let key = "ai.permagent.mobile-chat-session"
    static func chatSessionId() -> String {
        if let existing = UserDefaults.standard.string(forKey: key) { return existing }
        let id = UUID().uuidString
        UserDefaults.standard.set(id, forKey: key)
        return id
    }
}

struct DaemonEvent: Decodable, Sendable {
    let type: String
    let payload: [String: AnyCodable]?
}

/// Minimal AnyCodable for event payloads.
///
/// Backed by a closed enum rather than `Any`. `Any` is not `Sendable`, which
/// made `DaemonEvent` non-Sendable and meant yielding one into the event
/// stream's continuation tripped Swift 6's `sending` check ("Sending 'event'
/// risks causing data races"). Annotating the surrounding closure could not fix
/// that — the TYPE was the problem, so the annotation only moved the error.
///
/// The enum is not a workaround: the decoder below only ever produced a String,
/// Int, Double or Bool, so this is a faithful — and now checkable —
/// representation of what the payload could always hold.
struct AnyCodable: Decodable, Sendable {
    enum Value: Sendable, Equatable {
        case string(String)
        case int(Int)
        case double(Double)
        case bool(Bool)
        /// The decoder's existing fallback for an unrecognized scalar.
        case empty
    }

    let value: Value

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) { value = .string(s) }
        else if let i = try? c.decode(Int.self) { value = .int(i) }
        else if let d = try? c.decode(Double.self) { value = .double(d) }
        else if let b = try? c.decode(Bool.self) { value = .bool(b) }
        else { value = .empty }
    }

    var string: String? {
        switch value {
        case .string(let string): return string
        case .empty: return ""
        default: return nil
        }
    }
}

enum KeychainStore {
    private static let key = "ai.permagent.hub-pairing"
    static func save(_ config: HubConfig) {
        guard let data = try? JSONEncoder().encode(config) else { return }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: key,
        ]
        SecItemDelete(query as CFDictionary)
        var add = query
        add[kSecValueData as String] = data
        SecItemAdd(add as CFDictionary, nil)
    }
    static func load() -> HubConfig? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: key,
            kSecReturnData as String: true,
        ]
        var out: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess,
              let data = out as? Data else { return nil }
        return try? JSONDecoder().decode(HubConfig.self, from: data)
    }
}

// Multipart body assembly helper (for the dictation upload).
private extension Data {
    mutating func appendString(_ string: String) {
        if let data = string.data(using: .utf8) { append(data) }
    }
}
