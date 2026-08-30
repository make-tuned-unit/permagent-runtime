// Permagent for Apple Watch — two buttons, nothing else.
// Tap Chat or Note and the orb is already listening. The iPhone relays to
// the hub (WatchConnectivity); this process stores no pairing token and
// never talks to the daemon itself.

import SwiftUI
import WatchKit

final class WatchExtensionDelegate: NSObject, WKExtensionDelegate {
    func handle(_ backgroundTasks: Set<WKRefreshBackgroundTask>) {
        // WatchConnectivity wakes the extension without presenting a window.
        // Activate the relay here and complete every refresh task; leaving one
        // outstanding consumes watchOS's background budget and can terminate
        // the app after repeated deliveries.
        Task { @MainActor in WatchRelay.shared.start() }
        for task in backgroundTasks {
            task.setTaskCompletedWithSnapshot(false)
        }
    }
}

@main
struct PermagentWatchApp: App {
    @WKExtensionDelegateAdaptor(WatchExtensionDelegate.self) private var extensionDelegate
    @StateObject private var relay = WatchRelay.shared

    var body: some Scene {
        WindowGroup {
            WatchHomeView()
                .environmentObject(relay)
                .task { relay.start() }
        }
    }
}
