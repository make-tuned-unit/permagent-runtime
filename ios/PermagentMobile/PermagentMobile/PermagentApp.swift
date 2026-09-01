// Permagent for iOS — the pocket client of the hub (MULTI_DEVICE.md).
// v1 = the supervision surfaces: chat with the agent, Decision Inbox, active
// goals, live notifications. One Brain lives on the hub; this device carries
// only its pairing token (Keychain). The embedded Watch app talks through
// this process (HubWatchRelay).

import SwiftUI

@main
@MainActor
struct PermagentApp: App {
    @StateObject private var session = HubSession()
    @ObservedObject private var route = AppRoute.shared
    @State private var splashDone = false

    init() {
        // WatchConnectivity may launch the phone app in the background while
        // the screen is locked. Install and activate the delegate before any
        // SwiftUI view task or pairing bootstrap is allowed to run.
        HubWatchRelay.shared.start()
    }

    var body: some Scene {
        WindowGroup {
            ZStack {
                ChatSurface.bg.ignoresSafeArea()
                if session.isPaired {
                    MainTabs().environmentObject(session)
                } else {
                    PairingView().environmentObject(session)
                }
                // Splash sits OVER the real UI rather than gating it, so
                // `bootstrap()` (Keychain read, hub reconnect) runs during the
                // 2.5s instead of after it — the animation costs no launch time.
                if !splashDone {
                    SplashView { splashDone = true }
                        .transition(.opacity)
                        .zIndex(1)
                }
            }
            // No `.preferredColorScheme` — the app follows the system
            // appearance, as the desktop app does with its `system` theme
            // preference. Brand tokens are dynamic colors (Theme.swift), so
            // both palettes are already in place. The tab bar (and every
            // stock control) tints to the spark accent.
            .tint(ChatSurface.spark)
            .task { await session.bootstrap() }
            .fullScreenCover(isPresented: $route.showVoice) {
                VoiceView()
            }
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
        // Before anything else: reclaim recordings the last run left behind.
        // An upload that was in flight when the process died goes back in the
        // queue, a recording still marked open is closed, and a clip left in
        // staging is adopted rather than swept. Audio survives an app kill and
        // a device restart because this runs before any code path could look
        // at a half-finished recording and decide it was rubbish.
        RecordingStore.shared.recoverAfterLaunch()

        HubWatchRelay.shared.start()

        await APIClient.shared.loadSavedPairing()
        let paired = await APIClient.shared.isPaired
        if paired {
            // Drain on launch, so a meeting recorded with the Mac asleep
            // sends itself the next time the app is opened in range.
            MeetingUploader.shared.requestDrain()
            // Resolve the name BEFORE flipping isPaired. MainTabs' first paint
            // is what the iOS 26 tab bar snapshots; if we show the tabs on the
            // generic fallback and refresh afterwards, the chat tab stays
            // "your agent" even though the chat header updates.
            await AgentIdentity.shared.refresh()
        }
        isPaired = paired
        if paired {
            HubWatchRelay.shared.pushStatus()
            await listen()
        } else {
            HubWatchRelay.shared.pushStatus()
        }
    }

    /// Pair from a scanned/pasted URL. Two forms:
    /// - current hub: http://<hub>/ui/#claim=<code> — a one-time claim code
    ///   exchanged for this device's own bearer token via the public
    ///   `POST /pair/claim` (routes/devices.rs, #628)
    /// - legacy:      http://<hub>:3001/ui/#token=<token> — a raw bearer token
    ///
    /// **Never default the port.** The hub's own pairing URL comes from
    /// `tailscale serve`, which fronts the daemon on the scheme's default port
    /// (80) and carries no explicit port at all — the daemon itself stays bound
    /// to 127.0.0.1:3001 and is NOT reachable on 3001 from the tailnet. Filling
    /// in 3001 when the URL omits a port rewrites a working serve URL into the
    /// one address that is guaranteed to refuse the connection. An absent port
    /// means the scheme default, which is what every other HTTP client does.
    func pair(from url: String) async -> Result<Void, PairingError> {
        guard let comps = URLComponents(string: url.trimmingCharacters(in: .whitespaces)),
              let scheme = comps.scheme,
              scheme == "http" || scheme == "https",
              let host = comps.host,
              let base = URL(string: comps.port.map { "\(scheme)://\(host):\($0)" }
                  ?? "\(scheme)://\(host)")
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
        await AgentIdentity.shared.refresh()
        isPaired = true
        HubWatchRelay.shared.pushStatus()
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
                if event.type == "identity_changed" {
                    await AgentIdentity.shared.refresh()
                }
            }
        }
    }
}

/// The app's top-level surfaces. Hashable tags so any card, row, or agent
/// nudge can LAND somewhere: Home's tiles jump straight to their section
/// instead of being read-only numbers.
enum AppTab: Hashable {
    case home, chat, notes, decisions, goals, control
}

struct MainTabs: View {
    @EnvironmentObject var session: HubSession
    // Observe so a later identity_changed (Settings rename) rebuilds the tab
    // bar. Observing alone is not enough on iOS 26: the liquid-glass tab bar
    // snapshots `.tabItem` on first insert, which is why the chat tab used to
    // stay on "your agent" after pairing. We also delay showing MainTabs until
    // refresh() has finished (see HubSession.bootstrap / pair).
    @ObservedObject private var identity = AgentIdentity.shared
    @State private var tab: AppTab = .home
    /// Versioned because v1 used the non-neural spectral print. Every install
    /// gets this setup at most once; an existing learned hub print is detected
    /// and completes without recording again.
    @AppStorage("voiceIdentitySetupCompletedV2") private var voiceSetupCompleted = false
    @State private var showVoiceSetup = false
    var body: some View {
        TabView(selection: $tab) {
            HomeView(tab: $tab).tabItem { Label("Home", systemImage: "circle.hexagongrid.fill") }
                .tag(AppTab.home)
            ChatView().tabItem { Label(identity.displayName, systemImage: "bubble.left.and.bubble.right.fill") }
                .tag(AppTab.chat)
            // "Notes" — the tab owns dictation AND the notes library. The mic
            // icon stays because one tap on this tab still starts a note by
            // voice; browsing is the secondary path inside it.
            DictateView().tabItem { Label("Notes", systemImage: "mic.fill") }
                .tag(AppTab.notes)
            InboxView().tabItem { Label("Decisions", systemImage: "tray.full.fill") }
                .badge(session.unread)
                .tag(AppTab.decisions)
            GoalsView().tabItem { Label("In Flight", systemImage: "bolt.fill") }
                .tag(AppTab.goals)
            ControlHubView().tabItem { Label("Control", systemImage: "slider.horizontal.3") }
                .tag(AppTab.control)
        }
        .id(identity.displayName)
        .liquidGlassTabMinimize()
        .onAppear {
            if !voiceSetupCompleted { showVoiceSetup = true }
        }
        .fullScreenCover(isPresented: $showVoiceSetup) {
            VoiceIdentityView(onboarding: true) {
                voiceSetupCompleted = true
                showVoiceSetup = false
            }
        }
    }
}
