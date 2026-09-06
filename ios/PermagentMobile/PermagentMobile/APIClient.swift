// Daemon API client — the phone is a thin client of the hub (MULTI_DEVICE.md).
// Same bearer pairing model as the web: one token, stored in the Keychain.

import Foundation

struct HubConfig: Codable, Equatable {
    var baseURL: URL       // http://<magicdns> — via tailscale serve, no port
    var token: String      // daemon_token (the pairing secret)
}

enum APIError: Error { case unauthorized, badStatus(Int), notPaired, dictationUnavailable, daemon(String) }

actor APIClient {
    static let shared = APIClient()
    private var config: HubConfig?

    func pair(_ config: HubConfig) {
        self.config = config
        KeychainStore.save(config)
    }

    func loadSavedPairing() {
        #if DEBUG
        if DesignPreview.enabled { return }
        #endif
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

    /// `send` with a JSON body, for endpoints whose response body we don't
    /// need (set_provider answers a shape that varies by daemon version —
    /// only the status matters here).
    func send<B: Encodable>(_ path: String, method: String = "POST", body: B) async throws {
        guard let config else { throw APIError.notPaired }
        var req = URLRequest(url: config.baseURL.appendingPathComponent(path))
        req.httpMethod = method
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONEncoder().encode(body)
        let (_, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
        if http.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }
    }

    /// POST that returns the raw body. `/config/read` answers a bare JSON
    /// value (`true` / `null` / a string), not an object `get` can decode —
    /// same contract the desktop `readConfig` uses.
    func postData<B: Encodable>(_ path: String, body: B) async throws -> Data {
        guard let config else { throw APIError.notPaired }
        var req = URLRequest(url: config.baseURL.appendingPathComponent(path))
        req.httpMethod = "POST"
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONEncoder().encode(body)
        let (data, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
        if http.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }
        return data
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

                    // The parsing — including the segment-break bookkeeping
                    // that fixes "…works.Let me dig deeper…" — lives in
                    // ReplyStreamParser (ChatStream.swift) so the regression
                    // tests can drive it with raw SSE lines.
                    var parser = ReplyStreamParser()
                    for try await line in bytes.lines {
                        switch parser.consume(line: line) {
                        case .none:
                            break
                        case .delta(let delta):
                            continuation.yield(delta)
                        case .finish:
                            continuation.finish(); return
                        case .error(let message):
                            continuation.finish(throwing: APIError.daemon(message)); return
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    /// One frame from the session's reattachable event stream.
    enum SessionEvent: Sendable {
        /// The daemon's first frame: the session's in-flight request ids. Empty
        /// means nothing is running — the stored transcript is the whole truth.
        case activeRequests([String])
        /// The turn finished (a terminal Finish frame arrived).
        case finished
    }

    /// Reattach to a session's live event stream (GET /sessions/{id}/events).
    ///
    /// This is how a phone that was locked or closed mid-reply catches up: the
    /// hub keeps running the turn regardless of who is watching, and this
    /// stream's opening ActiveRequests frame says whether one is still live.
    /// Deliberately NOT used for token-level rendering — replayed frames can
    /// span earlier turns, so the caller re-fetches the stored transcript for
    /// content and uses this stream only for "is it done yet".
    nonisolated func sessionEvents(sessionId: String) -> AsyncThrowingStream<SessionEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    await loadSavedPairing()
                    guard let config = await self.config else {
                        continuation.finish(throwing: APIError.notPaired); return
                    }
                    var comps = URLComponents(
                        url: config.baseURL.appendingPathComponent("/sessions/\(sessionId)/events"),
                        resolvingAgainstBaseURL: false
                    )!
                    comps.queryItems = [URLQueryItem(name: "token", value: config.token)]
                    var req = URLRequest(url: comps.url!)
                    req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    req.timeoutInterval = 600

                    let (bytes, resp) = try await URLSession.shared.bytes(for: req)
                    guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
                    if http.statusCode == 401 { throw APIError.unauthorized }
                    guard (200..<300).contains(http.statusCode) else { throw APIError.badStatus(http.statusCode) }

                    struct Frame: Decodable { let type: String; let request_ids: [String]? }
                    for try await line in bytes.lines {
                        guard line.hasPrefix("data: ") else { continue }
                        guard let data = String(line.dropFirst(6)).data(using: .utf8),
                              let frame = try? JSONDecoder().decode(Frame.self, from: data)
                        else { continue }
                        switch frame.type {
                        case "ActiveRequests":
                            continuation.yield(.activeRequests(frame.request_ids ?? []))
                        case "Finish", "Error":
                            // Either way the turn is over; the transcript holds
                            // whatever truth there is.
                            continuation.yield(.finished)
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
        try await createNote(projectId: projectId, title: title, body: body, kind: nil)
    }

    /// `kind: "meeting"` is not decoration — it is what makes the hub run its
    /// write-up pass over the transcript and file the action items it finds as
    /// cards on this project's board (`extract_meeting_todos`,
    /// crates/goose-server/src/routes/projects.rs). It is the same field the
    /// desktop meeting recorder sends, so a meeting captured on the phone and
    /// one captured on the Mac land through one path, not two.
    func createNote(projectId: String, title: String?, body: String, kind: String?) async throws -> CreatedNote {
        struct Req: Encodable { let title: String?; let body: String; let kind: String? }
        return try await post("/api/projects/\(projectId)/notes",
                              body: Req(title: title, body: body, kind: kind), as: CreatedNote.self)
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

    /// Upload one meeting SEGMENT for transcription.
    ///
    /// Distinct from `transcribe(wav:)` in the two ways the meeting queue
    /// needs. It gives the segment a long enough timeout to be transcribed —
    /// minutes of audio on a CPU Whisper, where the default 60 seconds would
    /// abandon a request the hub was still working on and make a working setup
    /// look broken. And it preserves the daemon's own error text as
    /// `APIError.daemon`, because "the hub could not decode this segment" and
    /// "the hub is asleep" call for completely different words on screen, and
    /// a bare status code cannot tell them apart.
    ///
    /// Returning normally is the ONLY signal that authorises deleting the
    /// audio this data came from. Every throw retains it.
    func transcribeSegment(_ audio: Data, filename: String, mimeType: String) async throws -> String {
        guard let config else { throw APIError.notPaired }
        let boundary = "permagent-\(UUID().uuidString)"
        var req = URLRequest(url: config.baseURL.appendingPathComponent("/api/dictation/transcribe"))
        req.httpMethod = "POST"
        req.setValue("Bearer \(config.token)", forHTTPHeaderField: "Authorization")
        req.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")
        // A meeting segment is minutes of audio and the hub transcribes it on
        // CPU; the default 60 s resource timeout would abandon a request the
        // hub was still working on and make a working setup look broken.
        req.timeoutInterval = 300
        var body = Data()
        body.append("--\(boundary)\r\n".data(using: .utf8)!)
        body.append("Content-Disposition: form-data; name=\"audio\"; filename=\"\(filename)\"\r\n".data(using: .utf8)!)
        body.append("Content-Type: \(mimeType)\r\n\r\n".data(using: .utf8)!)
        body.append(audio)
        body.append("\r\n--\(boundary)--\r\n".data(using: .utf8)!)
        req.httpBody = body

        let (data, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw APIError.badStatus(0) }
        if http.statusCode == 401 { throw APIError.unauthorized }
        if http.statusCode == 503 { throw APIError.dictationUnavailable }
        guard (200..<300).contains(http.statusCode) else {
            let detail = String(data: data, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            throw APIError.daemon(detail.isEmpty ? "HTTP \(http.statusCode)" : detail)
        }
        struct Resp: Decodable { let text: String }
        return try JSONDecoder().decode(Resp.self, from: data).text
    }

    /// Bare JSON value from `/config/read`. Only a literal `true` is on —
    /// matching the desktop `readFlag` (a missing key, `"true"`, or `1` is off).
    func readConfigFlag(_ key: String) async -> Bool {
        struct Req: Encodable { let key: String; let is_secret: Bool }
        guard let data = try? await postData("/config/read", body: Req(key: key, is_secret: false)) else {
            return false
        }
        return (try? JSONDecoder().decode(Bool.self, from: data)) == true
    }

    func upsertConfig(_ key: String, value: Bool) async throws {
        struct Req: Encodable { let key: String; let value: Bool; let is_secret: Bool }
        try await send("/config/upsert", body: Req(key: key, value: value, is_secret: false))
    }

    func integrations() async throws -> [IntegrationStatus] {
        try await get("/integrations", as: [IntegrationStatus].self)
    }

    func agentRoster() async throws -> AgentRoster {
        try await get("/api/agents/roster", as: AgentRoster.self)
    }

    func pronunciations() async throws -> [String: PronunciationEntry] {
        try await get("/voice/pronunciations", as: [String: PronunciationEntry].self)
    }

    func unresolvedPronunciations() async throws -> [UnresolvedPronunciation] {
        struct Resp: Decodable { let unresolved: [UnresolvedPronunciation] }
        return try await get("/voice/pronunciations/unresolved", as: Resp.self).unresolved
    }

    func savePronunciation(word: String, soundsLike: String) async throws {
        struct Req: Encodable { let word: String; let sounds_like: String }
        try await send("/voice/pronunciations", method: "PUT",
                       body: Req(word: word, sounds_like: soundsLike))
    }

    func deletePronunciation(_ word: String) async throws {
        try await send("/voice/pronunciations/\(word)", method: "DELETE")
    }
}

struct IntegrationStatus: Decodable {
    let provider: String
    let connected: Bool
    let token_present: Bool
}

struct PronunciationEntry: Decodable {
    let ipa: String
    let sounds_like: String
}

struct UnresolvedPronunciation: Decodable, Identifiable {
    let word: String
    let spelled_out_times: Int
    var id: String { word }
}

/// Merged `/api/agents/roster` — the same surface Settings → Agents on the
/// desktop reads. `gate` is validated rather than trusted: a daemon older
/// than this app may omit it, and a missing switch must render as "no
/// switch", never as a toggle claiming off.
struct AgentRoster: Decodable {
    let workers: [RosterWorker]
    let dispatch_roster: [DispatchPersona]
}

struct AgentGate: Decodable {
    let config_key: String
    let enabled: Bool
}

struct RosterWorker: Decodable, Identifiable {
    let id: String
    let display_name: String
    let what_it_does: String
    let why_it_matters: String?
    let gate: AgentGate?
    let live_state: LiveState?

    enum CodingKeys: String, CodingKey {
        case id, display_name, what_it_does, why_it_matters, gate, live_state
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        display_name = try c.decode(String.self, forKey: .display_name)
        what_it_does = try c.decode(String.self, forKey: .what_it_does)
        why_it_matters = try c.decodeIfPresent(String.self, forKey: .why_it_matters)
        if let g = try? c.decode(AgentGate.self, forKey: .gate), !g.config_key.isEmpty {
            gate = g
        } else {
            gate = nil
        }
        live_state = try? c.decode(LiveState.self, forKey: .live_state)
    }
}

struct DispatchPersona: Decodable, Identifiable {
    let key: String
    var id: String { key }
    let display_name: String
    let role: String
    let engine: String
    let gate: AgentGate?
    let availability: Availability?

    enum CodingKeys: String, CodingKey {
        case key, display_name, role, engine, gate, availability
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        key = try c.decode(String.self, forKey: .key)
        display_name = try c.decode(String.self, forKey: .display_name)
        role = try c.decode(String.self, forKey: .role)
        engine = try c.decodeIfPresent(String.self, forKey: .engine) ?? ""
        if let g = try? c.decode(AgentGate.self, forKey: .gate), !g.config_key.isEmpty {
            gate = g
        } else {
            gate = nil
        }
        availability = try? c.decode(Availability.self, forKey: .availability)
    }

    var engineLabel: String { engine.replacingOccurrences(of: "_", with: " ") }
}

struct LiveState: Decodable {
    enum Kind { case ok(String), notQueryable, unavailable(String) }
    let kind: Kind
    enum CodingKeys: String, CodingKey { case status, value, reason }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .status) {
        case "ok":
            kind = .ok(try c.decodeIfPresent(String.self, forKey: .value) ?? "")
        case "unavailable":
            kind = .unavailable(try c.decodeIfPresent(String.self, forKey: .reason) ?? "")
        default:
            kind = .notQueryable
        }
    }
}

struct Availability: Decodable {
    enum Kind { case available, unavailable(String), probeFailed(String) }
    let kind: Kind
    enum CodingKeys: String, CodingKey { case status, reason }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .status) {
        case "available":
            kind = .available
        case "probe_failed":
            kind = .probeFailed(try c.decodeIfPresent(String.self, forKey: .reason) ?? "")
        default:
            kind = .unavailable(try c.decodeIfPresent(String.self, forKey: .reason) ?? "")
        }
    }
}

// The /reply request + event shapes, the stream parser, and the transcript
// mapping all live in ChatStream.swift — Foundation-only so the regression
// tests compile them without the app target.

/// A stable per-install chat session id so the conversation persists across
/// launches.
///
/// The hub does NOT create sessions lazily — that assumption (in the comment
/// this replaces) is why chat never worked from the phone. Minting a UUID
/// locally and posting it as `session_id` made `/reply` answer 200 and then
/// immediately fail its stream with
/// `Failed to read session for <uuid>: Session not found`, which the UI
/// reported as "Couldn't reach Henry" — pointing at the network while the
/// network was fine (diagnosed from the hub log, 2026-08-04).
///
/// Sessions are created by `POST /api/sessions`, which returns the id. The
/// phone must ask for one and use what it is given.
enum MobileSession {
    private static let key = "ai.permagent.ios-chat-session"

    private struct SessionResponse: Decodable { let id: String }
    private struct CreateBody: Encodable { let workingDir: String }

    /// Adopt an existing hub session as this install's conversation and return
    /// its history as chat bubbles. Both voice and text follow the adopted id,
    /// because they deliberately share one thread.
    ///
    /// Tool traffic is dropped: a resumed thread should read the way it read
    /// when it was live — the user's words and the agent's answers — not a
    /// transcript of every tool call underneath them.
    static func adopt(_ id: String) async throws -> [ChatBubble] {
        struct Session: Decodable { let conversation: [StoredMessage]? }

        let session = try await APIClient.shared.get("/api/sessions/\(id)", as: Session.self)
        UserDefaults.standard.set(id, forKey: key)
        // The mapping (segment joins, role filtering) is ChatTranscript's —
        // pure and regression-tested in ChatStreamTests.
        return ChatTranscript.bubbles(from: session.conversation ?? [])
    }

    /// The persisted conversation id, if any — read-only, never creates.
    /// Cold launch uses this to put the ongoing conversation back on screen
    /// (and catch up on any reply that finished while the app was closed).
    static var persistedId: String? {
        UserDefaults.standard.string(forKey: key)
    }

    /// Forget the current conversation, so the next turn mints a fresh one.
    ///
    /// Voice and text share this id deliberately — a spoken turn lands in the
    /// same thread — so ending it here ends both. Nothing is deleted on the
    /// hub: the old session keeps its history and stays browsable there; this
    /// only stops new turns from joining it.
    static func endConversation() {
        UserDefaults.standard.removeObject(forKey: key)
    }

    /// The hub-created session for this install, creating one if needed.
    ///
    /// A cached id is verified against the hub before reuse: sessions can be
    /// deleted on the desktop, and a stale id fails exactly the same way an
    /// invented one did. Verification is a cheap GET and only happens once per
    /// launch, so the cost is a single request against never chatting again.
    static func chatSessionId() async throws -> String {
        if let existing = UserDefaults.standard.string(forKey: key) {
            if (try? await APIClient.shared.send("/api/sessions/\(existing)", method: "GET")) != nil {
                return existing
            }
            UserDefaults.standard.removeObject(forKey: key)
        }
        let created: SessionResponse = try await APIClient.shared.post(
            "/api/sessions",
            body: CreateBody(workingDir: "/tmp"),
            as: SessionResponse.self
        )
        UserDefaults.standard.set(created.id, forKey: key)
        return created.id
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
        // The locked phone is the watch's network relay. Make the pairing
        // credential available after the first device unlock so a
        // WatchConnectivity background launch can authenticate while locked.
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
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
        guard let config = try? JSONDecoder().decode(HubConfig.self, from: data) else { return nil }
        var updateQuery = query
        updateQuery.removeValue(forKey: kSecReturnData as String)
        let accessibility: [String: Any] = [
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        SecItemUpdate(updateQuery as CFDictionary, accessibility as CFDictionary)
        return config
    }
}

// Multipart body assembly helper (for the dictation upload).
private extension Data {
    mutating func appendString(_ string: String) {
        if let data = string.data(using: .utf8) { append(data) }
    }
}
