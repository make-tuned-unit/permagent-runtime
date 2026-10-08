import AppKit
import WebKit

// A dedicated local playback surface. No microphone/camera capture APIs,
// private WebKit preferences, browser automation or external navigation.
@MainActor
final class PracticeDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, WKNavigationDelegate, WKUIDelegate {
    private var window: NSWindow!
    private var webView: WKWebView!
    private let fixtureURL = URL(string: "http://127.0.0.1:5489/")!
    private var terminating = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        let menu = NSMenu()
        let appMenuItem = NSMenuItem()
        menu.addItem(appMenuItem)
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "About Permagent Record Practice", action: #selector(showAbout), keyEquivalent: "")
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(withTitle: "Quit Permagent Record Practice", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appMenuItem.submenu = appMenu
        let viewMenuItem = NSMenuItem(title: "View", action: nil, keyEquivalent: "")
        let viewMenu = NSMenu(title: "View")
        let reload = NSMenuItem(title: "Reload local fixture", action: #selector(reloadFixture), keyEquivalent: "r")
        reload.target = self
        viewMenu.addItem(reload)
        viewMenuItem.submenu = viewMenu
        menu.addItem(viewMenuItem)
        NSApp.mainMenu = menu

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        // Explicit gesture still required for playback. The fixture's Start
        // button resumes Web Audio before preparing the synthetic peers.
        config.mediaTypesRequiringUserActionForPlayback = .all
        config.preferences.javaScriptCanOpenWindowsAutomatically = false
        webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = self
        webView.uiDelegate = self
        if #available(macOS 13.3, *) { webView.isInspectable = true }

        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1180, height: 900),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable],
                          backing: .buffered, defer: false)
        window.title = "Permagent Record Practice"
        window.minSize = NSSize(width: 800, height: 600)
        window.contentView = webView
        window.delegate = self
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        webView.load(URLRequest(url: fixtureURL))
    }

    @objc private func reloadFixture() { webView.load(URLRequest(url: fixtureURL)) }
    @objc private func showAbout() {
        NSApp.orderFrontStandardAboutPanel(options: [
            .applicationName: "Permagent Record Practice",
            .applicationVersion: "1.0 · localhost fixture",
            .credits: NSAttributedString(string: "Synthetic WebRTC playback only. No camera or microphone access. Closing this app stops its playback, not Permagent Record.")
        ])
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if terminating { return .terminateNow }
        terminating = true
        guard let webView else { return .terminateNow }
        // Give the fixture a chance to close its peer connections and tracks.
        // A bounded fallback still terminates the owned host if WebKit is busy.
        var replied = false
        let finish = {
            if !replied { replied = true; NSApp.reply(toApplicationShouldTerminate: true) }
        }
        webView.evaluateJavaScript("window.dispatchEvent(new Event('pagehide'))") { _, _ in finish() }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { finish() }
        return .terminateLater
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else { decisionHandler(.cancel); return }
        let local = url.scheme == "http" && url.host == "127.0.0.1" && url.port == 5489
        // blob: is only generated locally for the fixture evidence download.
        // Normal app navigation remains confined to the owned loopback server.
        decisionHandler(local || url.scheme == "blob" ? .allow : .cancel)
    }

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        // Manifest links stay in this dedicated host, never another browser.
        if let url = navigationAction.request.url,
           url.scheme == "http", url.host == "127.0.0.1", url.port == 5489 {
            webView.load(URLRequest(url: url))
        }
        return nil
    }

    @available(macOS 12.0, *)
    func webView(_ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin,
                 initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType,
                 decisionHandler: @escaping (WKPermissionDecision) -> Void) {
        // Synthetic MediaStream tracks require no device capture. Deny any
        // accidental future getUserMedia request instead of showing a prompt.
        decisionHandler(.deny)
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        window.title = "Permagent Record Practice — local server unavailable"
        NSLog("Local practice fixture failed to load: %@", error.localizedDescription)
    }
}

@main
struct PracticeMain {
    @MainActor static func main() {
        let application = NSApplication.shared
        let delegate = PracticeDelegate()
        application.delegate = delegate
        application.setActivationPolicy(.regular)
        withExtendedLifetime(delegate) { application.run() }
    }
}
