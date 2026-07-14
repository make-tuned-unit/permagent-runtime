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

    /// Pair from a scanned/pasted URL of the form http://<hub>:3001/ui/#token=…
    func pair(from url: String) async -> Bool {
        guard let comps = URLComponents(string: url.trimmingCharacters(in: .whitespaces)),
              let host = comps.host,
              let token = comps.fragment?
                  .split(separator: "&")
                  .first(where: { $0.hasPrefix("token=") })
                  .map({ String($0.dropFirst("token=".count)) }),
              !token.isEmpty,
              let base = URL(string: "\(comps.scheme ?? "http")://\(host):\(comps.port ?? 3001)")
        else { return false }
        await APIClient.shared.pair(HubConfig(baseURL: base, token: token))
        isPaired = true
        await listen()
        return true
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
            InboxView().tabItem { Label("Decisions", systemImage: "tray.full.fill") }
                .badge(session.unread)
            GoalsView().tabItem { Label("In Flight", systemImage: "bolt.fill") }
            ControlHubView().tabItem { Label("Control", systemImage: "slider.horizontal.3") }
        }
    }
}
