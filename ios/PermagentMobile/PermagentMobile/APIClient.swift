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

    func get<T: Decodable>(_ path: String, as type: T.Type) async throws -> T {
        try await request(path, method: "GET", body: Optional<Int>.none)
    }

    func post<T: Decodable, B: Encodable>(_ path: String, body: B, as type: T.Type) async throws -> T {
        try await request(path, method: "POST", body: body)
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
            func listen() {
                task.receive { result in
                    switch result {
                    case .success(let message):
                        if case .string(let text) = message,
                           let data = text.data(using: .utf8),
                           let event = try? JSONDecoder().decode(DaemonEvent.self, from: data) {
                            continuation.yield(event)
                        }
                        listen()
                    case .failure:
                        continuation.finish()
                    }
                }
            }
            listen()
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
                        content: [ReplyContent(type: "text", text: text)],
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

struct DaemonEvent: Decodable {
    let type: String
    let payload: [String: AnyCodable]?
}

/// Minimal AnyCodable for event payloads.
struct AnyCodable: Decodable {
    let value: Any
    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) { value = s }
        else if let i = try? c.decode(Int.self) { value = i }
        else if let d = try? c.decode(Double.self) { value = d }
        else if let b = try? c.decode(Bool.self) { value = b }
        else { value = "" }
    }
    var string: String? { value as? String }
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
