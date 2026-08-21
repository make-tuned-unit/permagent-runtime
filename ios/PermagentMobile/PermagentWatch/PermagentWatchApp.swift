// Permagent for Apple Watch — two buttons, nothing else.
// Tap Chat or Note and the orb is already listening. The iPhone relays to
// the hub (WatchConnectivity); this process stores no pairing token and
// never talks to the daemon itself.

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
