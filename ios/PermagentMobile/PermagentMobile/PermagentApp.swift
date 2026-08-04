// Permagent for iOS — the pocket client of the hub (MULTI_DEVICE.md).
// v1 = the supervision surfaces: chat with Henry, Decision Inbox, active
// goals, live notifications. One Brain lives on the hub; this device carries
// only its pairing token (Keychain).

import SwiftUI

@main
struct PermagentApp: App {
    @StateObject private var session = HubSession()
    var body: some Scene {
        WindowGroup {
            ZStack {
                Brand.shell.ignoresSafeArea()
                if session.isPaired {
                    MainTabs().environmentObject(session)
                } else {
                    PairingView().environmentObject(session)
                }
            }
            .preferredColorScheme(.dark)
            .tint(Brand.cyan)
            .task { await session.bootstrap() }
        }
    }
}

@MainActor
final class HubSession: ObservableObject {
    enum PairingError: Error {
        case malformedURL
        // Carries the underlying URLSession failure so the UI can name the real
        // cause (DNS vs. connection refused vs. offline vs. TLS) instead of a
        // single catch-all — the difference is exactly what tells us whether a
        // pairing failure is the phone, the tailnet, or the hub.
        case hubUnreachable(detail: String?)
        case claimRejected(statusCode: Int)
        case unexpectedResponse(statusCode: Int?)
    }

    @Published var isPaired = false
    @Published var unread = 0

    func bootstrap() async {
        await APIClient.shared.loadSavedPairing()
        isPaired = await APIClient.shared.isPaired
        if isPaired { await listen() }
    }

    /// Pair from a scanned/pasted URL. Two forms:
    /// - current hub: http://<hub>:3001/ui/#claim=<code> — a one-time claim code
    ///   exchanged for this device's own bearer token via the public
    ///   `POST /pair/claim` (routes/devices.rs, #628)
    /// - legacy:      http://<hub>:3001/ui/#token=<token> — a raw bearer token
    func pair(from url: String) async -> Result<Void, PairingError> {
        guard let comps = URLComponents(string: url.trimmingCharacters(in: .whitespaces)),
              let scheme = comps.scheme,
              scheme == "http" || scheme == "https",
              let host = comps.host,
              let base = URL(string: "\(scheme)://\(host):\(comps.port ?? 3001)")
        else { return .failure(.malformedURL) }
        func fragmentValue(_ key: String) -> String? {
            comps.fragment?
                .split(separator: "&")
                .first(where: { $0.hasPrefix("\(key)=") })
                .map { String($0.dropFirst(key.count + 1)) }
        }

        let token: String
        if let code = fragmentValue("claim"), !code.isEmpty {
            switch await Self.exchangeClaim(code: code, base: base) {
            case .success(let minted): token = minted
            case .failure(let error): return .failure(error)
            }
        } else if let legacy = fragmentValue("token"), !legacy.isEmpty {
            token = legacy
        } else {
            return .failure(.malformedURL)
        }
        await APIClient.shared.pair(HubConfig(baseURL: base, token: token))
        isPaired = true
        await listen()
        return .success(())
    }

    /// Turn a URLSession failure into a short, specific phrase. The URLError
    /// code is what disambiguates the four ways pairing "can't connect": name
    /// resolution, a refused connection, no network at all, or a TLS problem.
    private static func describeConnectionError(_ error: Error) -> String {
        guard let urlError = error as? URLError else {
            return error.localizedDescription
        }
        switch urlError.code {
        case .cannotFindHost, .dnsLookupFailed:
            return "the hub's name could not be resolved (DNS) — is MagicDNS on for this device?"
        case .cannotConnectToHost:
            return "the connection was refused — the hub is reachable but nothing is answering on that port."
        case .notConnectedToInternet, .networkConnectionLost:
            return "this device has no network connection."
        case .timedOut:
            return "the connection timed out — the hub did not respond."
        case .appTransportSecurityRequiresSecureConnection, .secureConnectionFailed:
            return "the connection was blocked as insecure (TLS)."
        default:
            return "\(urlError.localizedDescription) [URLError \(urlError.errorCode)]"
        }
    }

    /// `POST /pair/claim` `{"code": …}` → `{"token": …, "device": {…}}`.
    /// The code is 128-bit random, single-use, 10-min lived; unknown/expired
    /// codes answer 404. Only `token` is needed here — the device name was
    /// fixed when the hub minted the code.
    private static func exchangeClaim(code: String, base: URL) async -> Result<String, PairingError> {
        struct Req: Encodable { let code: String }
        struct Resp: Decodable { let token: String }
        var req = URLRequest(url: base.appendingPathComponent("/pair/claim"))
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        guard let body = try? JSONEncoder().encode(Req(code: code)) else {
            return .failure(.unexpectedResponse(statusCode: nil))
        }
        req.httpBody = body
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: req)
        } catch {
            return .failure(.hubUnreachable(detail: describeConnectionError(error)))
        }
        guard let http = response as? HTTPURLResponse else {
            return .failure(.unexpectedResponse(statusCode: nil))
        }
        if http.statusCode == 404 || http.statusCode == 410 {
            return .failure(.claimRejected(statusCode: http.statusCode))
        }
        guard (200..<300).contains(http.statusCode),
              let decoded = try? JSONDecoder().decode(Resp.self, from: data),
              !decoded.token.isEmpty
        else {
            return .failure(.unexpectedResponse(statusCode: http.statusCode))
        }
        return .success(decoded.token)
    }

    private func listen() async {
        guard let stream = await APIClient.shared.eventStream() else { return }
        Task {
            for await event in stream {
                if event.type == "decision_created" { unread += 1 }
            }
        }
    }
}

struct MainTabs: View {
    @EnvironmentObject var session: HubSession
    var body: some View {
        TabView {
            HomeView().tabItem { Label("Home", systemImage: "circle.hexagongrid.fill") }
            ChatView().tabItem { Label("Henry", systemImage: "bubble.left.and.bubble.right.fill") }
            DictateView().tabItem { Label("Dictate", systemImage: "mic.fill") }
            InboxView().tabItem { Label("Decisions", systemImage: "tray.full.fill") }
                .badge(session.unread)
            GoalsView().tabItem { Label("In Flight", systemImage: "bolt.fill") }
            ControlHubView().tabItem { Label("Control", systemImage: "slider.horizontal.3") }
        }
        .liquidGlassTabMinimize()
    }
}
