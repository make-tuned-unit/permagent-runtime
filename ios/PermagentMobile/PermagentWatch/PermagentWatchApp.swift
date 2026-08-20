// Permagent for Apple Watch — two buttons, nothing else.
// Orb chat and note dictate. The iPhone relays to the hub (WatchConnectivity);
// this process stores no pairing token and never talks to the daemon itself.

import SwiftUI

@main
struct PermagentWatchApp: App {
    @StateObject private var relay = WatchRelay.shared

    var body: some Scene {
        WindowGroup {
            WatchHomeView()
                .environmentObject(relay)
                .task { relay.start() }
        }
    }
}
