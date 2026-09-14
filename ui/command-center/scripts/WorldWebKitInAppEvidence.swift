// Copy of scripts/blender/WorldWebKitEvidence.swift, retargeted at the REAL app
// (the Command Center dev server) instead of the standalone worldcensus page,
// so the Solar Forum is measured in WKWebView — the engine Tauri uses on macOS.
//
// Differences from the original:
//  - loads http://127.0.0.1:5284/ui/ and drives the app's own sidebar to the
//    World workspace, rather than a single-purpose census page;
//  - authenticates the same way the app's browser build does (the daemon token
//    in localStorage under `permagent-daemon-token`), read at RUNTIME from
//    ~/.permagent/secrets/daemon_token.json and never printed;
//  - reads `window.__worldPerf` (the shared PerfSampler) because the in-app
//    page has no `__worldDebug`/`__worldPerfLog` census hooks.
//
// Standalone WKWebView qualification, NOT an installed-Tauri acceptance claim.
// Nonpersistent browser store, local dev server only.
//
// Build + run:
//   swiftc -O -o /tmp/WorldWebKitInAppEvidence \
//     ui/command-center/scripts/WorldWebKitInAppEvidence.swift
//   /tmp/WorldWebKitInAppEvidence
import AppKit
import WebKit

let appURL = ProcessInfo.processInfo.environment["APP_URL"] ?? "http://127.0.0.1:5284/ui/"
let snapshotPath = ProcessInfo.processInfo.environment["SNAPSHOT_PATH"]
    ?? "/private/tmp/permagent-world-webkit-in-app.png"

func daemonToken() -> String? {
    let path = NSString(string: "~/.permagent/secrets/daemon_token.json").expandingTildeInPath
    guard let data = FileManager.default.contents(atPath: path),
          let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let token = object["token"] as? String else { return nil }
    return token
}

@MainActor
final class Evidence: NSObject, NSApplicationDelegate, WKNavigationDelegate {
    var window: NSWindow!
    var web: WKWebView!
    var started = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        if let token = daemonToken() {
            // Same credential path as the app's browser build (api.ts
            // browserToken): a value in localStorage, set before page scripts run.
            let escaped = token.replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            let source = "try{localStorage.setItem('permagent-daemon-token',\"\(escaped)\");}catch(e){}"
            configuration.userContentController.addUserScript(
                WKUserScript(source: source, injectionTime: .atDocumentStart, forMainFrameOnly: true))
            print("TOKEN_INJECTED yes")
        } else {
            print("TOKEN_INJECTED no — the app will render its unavailable state")
        }
        web = WKWebView(frame: NSRect(x: 0, y: 0, width: 1440, height: 1000), configuration: configuration)
        web.navigationDelegate = self
        window = NSWindow(contentRect: web.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "Permagent World in-app — WebKit verification"
        window.contentView = web
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        window.orderFrontRegardless()
        print("SCREEN_MAX_FPS \(window.screen?.maximumFramesPerSecond ?? 0)")
        print("SCREENS \(NSScreen.screens.count) OCCLUSION \(window.occlusionState.contains(.visible) ? "visible" : "occluded") ACTIVE \(NSApp.isActive)")
        print("TARGET \(appURL)")
        web.load(URLRequest(url: URL(string: appURL)!))
        // Independent hard timeout: navigation/load failure cannot leave a
        // background verification process or browser window running forever.
        Task { try? await Task.sleep(for: .seconds(150)); print("WEBKIT_EVIDENCE_TIMEOUT"); exit(2) }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        print("WEBKIT_NAVIGATION_FAILED \(error.localizedDescription)")
        exit(3)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard !started else { return }
        started = true
        Task {
            do {
                // The app boots (splash → config → workspaces) before the
                // sidebar exists; poll rather than guess a single delay.
                var clicked = "no-world-button"
                for attempt in 1...30 {
                    try? await Task.sleep(for: .seconds(2))
                    let state = try await web.evaluateJavaScript("""
                    (() => {
                      const b = [...document.querySelectorAll('button')]
                        .find(x => (x.innerText || '').trim() === 'World');
                      if (!b) return 'waiting:' + (document.body.innerText || '').replace(/\\s+/g, ' ').slice(0, 80);
                      b.click();
                      return 'clicked';
                    })()
                    """)
                    let text = state as? String ?? "null"
                    if text == "clicked" { clicked = "clicked after \(attempt * 2)s"; break }
                    if attempt % 5 == 0 { print("WORLD_TAB_WAIT \(text)") }
                }
                print("WORLD_TAB \(clicked)")
                print("OCCLUSION_AFTER_LOAD \(self.window.occlusionState.contains(.visible) ? "visible" : "occluded")")
                // The forum chunk plus the GLB take a while on a cold load.
                try? await Task.sleep(for: .seconds(35))
                let value = try await web.evaluateJavaScript("""
                JSON.stringify({
                  engine: 'WKWebView standalone (in-app route)',
                  forumShells: document.querySelectorAll('.forum-shell').length,
                  forumVisible: [...document.querySelectorAll('.forum-shell')]
                    .some(s => s.getBoundingClientRect().width > 1),
                  brand: document.querySelector('.forum-brand h1')?.textContent ?? null,
                  sovereign: document.querySelector('#forum-agent')?.value ?? null,
                  perf: window.__worldPerf ?? null,
                  glb: performance.getEntriesByType('resource')
                    .filter(e => e.name.includes('solar-forum.glb'))
                    .map(e => ({ name: e.name.split('/').pop(), ms: Math.round(e.duration), bytes: e.transferSize })),
                  canvas: (() => {
                    const c = document.querySelector('.forum-shell canvas');
                    if (!c) return null;
                    const b = c.getBoundingClientRect();
                    const gl = c.getContext('webgl2') || c.getContext('webgl');
                    const d = gl && gl.getExtension('WEBGL_debug_renderer_info');
                    return { css: [Math.round(b.width), Math.round(b.height)],
                             buffer: [c.width, c.height],
                             renderer: d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : 'n/a' };
                  })(),
                  bodyText: (document.body.innerText || '').slice(0, 200)
                })
                """)
                print("WEBKIT_EVIDENCE \(value as? String ?? "null")")
                // Why might the sampler be silent? The frameloop gates on
                // document.hidden, and a blank snapshot could be a WKWebView
                // snapshot artefact rather than a render failure.
                _ = try await web.evaluateJavaScript("""
                (() => {
                  window.__rafN = 0;
                  const tick = () => { window.__rafN++; requestAnimationFrame(tick); };
                  requestAnimationFrame(tick);
                  window.__glbHead = 'pending';
                  fetch('world/solar-forum.glb', { method: 'HEAD' })
                    .then(r => { window.__glbHead = String(r.status); })
                    .catch(e => { window.__glbHead = 'ERR ' + e.message; });
                  return 1;
                })()
                """)
                try? await Task.sleep(for: .seconds(6))
                let second = try await web.evaluateJavaScript("""
                JSON.stringify({
                  perf: window.__worldPerf ?? null,
                  hidden: document.hidden,
                  visibility: document.visibilityState,
                  rafTicks: window.__rafN,
                  glbHead: window.__glbHead,
                  resourceEntries: performance.getEntriesByType('resource').length,
                  glbEntries: performance.getEntriesByType('resource').filter(e => e.name.includes('solar-forum.glb')).length,
                  canvasClientRect: (() => { const c = document.querySelector('.forum-shell canvas'); if (!c) return null; const b = c.getBoundingClientRect(); return [Math.round(b.width), Math.round(b.height)]; })(),
                  worldBox: (() => { const w = document.querySelector('.forum-world'); if (!w) return null; const b = w.getBoundingClientRect(); return [Math.round(b.width), Math.round(b.height)]; })()
                })
                """)
                print("WEBKIT_PERF_SECOND \(second as? String ?? "null")")
                let snapshot = try await web.takeSnapshot(configuration: nil)
                if let tiff = snapshot.tiffRepresentation,
                   let bitmap = NSBitmapImageRep(data: tiff),
                   let png = bitmap.representation(using: .png, properties: [:]) {
                    try png.write(to: URL(fileURLWithPath: snapshotPath), options: .atomic)
                    print("SNAPSHOT \(snapshotPath)")
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
