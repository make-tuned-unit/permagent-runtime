// Standalone WKWebView qualification, NOT an installed-Tauri acceptance claim.
// Nonpersistent browser store, local dev census only, no operator credentials.
import AppKit
import WebKit

@MainActor
final class Evidence: NSObject, NSApplicationDelegate, WKNavigationDelegate {
    var window: NSWindow!
    var web: WKWebView!
    var started = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        web = WKWebView(frame: NSRect(x: 0, y: 0, width: 1440, height: 1000), configuration: configuration)
        web.navigationDelegate = self
        window = NSWindow(contentRect: web.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "Permagent World — WebKit verification (no account)"
        window.contentView = web
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        print("SCREEN_MAX_FPS \(window.screen?.maximumFramesPerSecond ?? 0)")
        web.load(URLRequest(url: URL(string: "http://127.0.0.1:5173/ui/worldcensus.html?perf=1&dpr=1.5")!))
        // Independent hard timeout: navigation/load failure cannot leave a
        // background verification process or browser window running forever.
        Task { try? await Task.sleep(for: .seconds(45)); exit(2) }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard !started else { return }
        started = true
        Task {
            try? await Task.sleep(for: .seconds(16))
            do {
                let value = try await web.evaluateJavaScript("""
                JSON.stringify({engine:'WKWebView standalone',
                  assets:window.__worldDebug?.assetStats?.(),
                  samples:window.__worldPerfLog?.slice(-10) ?? [],
                  canvas: {width:document.querySelector('canvas')?.width,
                           height:document.querySelector('canvas')?.height}})
                """)
                print("WEBKIT_EVIDENCE \(value as? String ?? "null")")
                let snapshot = try await web.takeSnapshot(configuration: nil)
                if let tiff = snapshot.tiffRepresentation,
                   let bitmap = NSBitmapImageRep(data: tiff),
                   let png = bitmap.representation(using: .png, properties: [:]) {
                    try png.write(to: URL(fileURLWithPath: "/private/tmp/permagent-world-webkit.png"), options: .atomic)
                }
                window.close()
                exit(0)
            } catch {
                print("WEBKIT_EVIDENCE_FAILED \(error.localizedDescription)")
                exit(1)
            }
        }
    }
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let evidence = Evidence()
    app.setActivationPolicy(.regular)
    app.delegate = evidence
    app.run()
}
