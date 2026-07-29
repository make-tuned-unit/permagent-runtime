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
    func pair(from url: String) async -> Bool {
        guard let comps = URLComponents(string: url.trimmingCharacters(in: .whitespaces)),
              let host = comps.host,
              let base = URL(string: "\(comps.scheme ?? "http")://\(host):\(comps.port ?? 3001)")
        else { return false }
        func fragmentValue(_ key: String) -> String? {
            comps.fragment?
                .split(separator: "&")
                .first(where: { $0.hasPrefix("\(key)=") })
                .map { String($0.dropFirst(key.count + 1)) }
        }

        let token: String
        if let code = fragmentValue("claim"), !code.isEmpty {
            guard let minted = await Self.exchangeClaim(code: code, base: base) else { return false }
            token = minted
        } else if let legacy = fragmentValue("token"), !legacy.isEmpty {
            token = legacy
        } else {
            return false
        }
        await APIClient.shared.pair(HubConfig(baseURL: base, token: token))
        isPaired = true
        await listen()
        return true
    }

    /// `POST /pair/claim` `{"code": …}` → `{"token": …, "device": {…}}`.
    /// The code is 128-bit random, single-use, 10-min lived; unknown/expired
    /// codes answer 404. Only `token` is needed here — the device name was
    /// fixed when the hub minted the code.
    private static func exchangeClaim(code: String, base: URL) async -> String? {
        struct Req: Encodable { let code: String }
        struct Resp: Decodable { let token: String }
        var req = URLRequest(url: base.appendingPathComponent("/pair/claim"))
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        guard let body = try? JSONEncoder().encode(Req(code: code)) else { return nil }
        req.httpBody = body
        guard let (data, resp) = try? await URLSession.shared.data(for: req),
              let http = resp as? HTTPURLResponse,
              (200..<300).contains(http.statusCode),
              let decoded = try? JSONDecoder().decode(Resp.self, from: data)
        else { return nil }
        return decoded.token
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
