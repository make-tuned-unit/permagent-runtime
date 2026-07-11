// Daemon API client — the phone is a thin client of the hub (MULTI_DEVICE.md).
// Same bearer pairing model as the web: one token, stored in the Keychain.

import Foundation

struct HubConfig: Codable, Equatable {
    var baseURL: URL       // http://<magicdns>:3001
    var token: String      // daemon_token (the pairing secret)
}

enum APIError: Error { case unauthorized, badStatus(Int), notPaired }

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
