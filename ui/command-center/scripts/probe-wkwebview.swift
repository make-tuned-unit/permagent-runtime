// The white/black rectangle probe, in the SYSTEM WKWebView — the renderer
// Tauri actually ships on macOS.
//
// Why this exists: `probe-black-rect.mjs --engine webkit` drives Playwright's
// own WebKit build (webkit-2359 here), which is WebKit, but not the WebKit in
// /System that Permagent.app's WKWebView links against. On 2026-09-18 the
// Playwright build rendered the night forum clean at every size tried while
// Jesse was looking at a white block in the app, so the last difference left
// between the instrument and the product was the WebKit itself.
//
// It opens its OWN window, off-screen, in its own process: it never touches
// Permagent.app, the daemon, or anything the user is looking at. Occlusion
// detection is switched off so an off-screen window still gets
// requestAnimationFrame (WebKit throttles occluded windows to zero frames,
// which would read as "clean" for the wrong reason).
//
//   swift scripts/probe-wkwebview.swift [url] [WxH] [shots] [out.png] [dark|light]
//   STORAGE='{"permagent.forum.desk":"closed"}' swift scripts/probe-wkwebview.swift ...
//
// Prints one JSON line per frame: the pure-white and pure-black runs of at
// least 200 CSS px (the same fingerprint `probe-black-rect.mjs` counts),
// grouped into blocks and judged solid per block, read straight out of the
// WebGL drawing buffer rather than off a screenshot.
import AppKit
import WebKit

let args = CommandLine.arguments
let url = URL(string: args.count > 1 ? args[1] : "http://localhost:5173/ui/forum.html")!
let size = (args.count > 2 ? args[2] : "1512x945").split(separator: "x").map { Double($0)! }
let shots = args.count > 3 ? Int(args[3])! : 6
let outPng = args.count > 4 ? args[4] : ""
let scheme = args.count > 5 ? args[5] : "dark"
let settle = Double(ProcessInfo.processInfo.environment["SETTLE_MS"] ?? "8000")! / 1000
// STORAGE='{"permagent.forum.desk":"closed",...}' seeds localStorage before
// any page script runs, so the forum renders in the layout the user's app
// has. `probe-black-rect.mjs --app-state` prints the app's own values under
// "storage"; paste them here. Empty storage is desk and rail OPEN, which the
// user's app was not in on 2026-09-19 (see forumStorageFromRows).
let storage = ProcessInfo.processInfo.environment["STORAGE"] ?? ""

// Keep the drawing buffer readable after compositing, so readPixels sees the
// frame that was presented instead of a cleared buffer.
let preserve = """
(() => { const orig = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (type, attrs) {
    if (type === 'webgl2' || type === 'webgl') attrs = Object.assign({}, attrs || {}, { preserveDrawingBuffer: true });
    return orig.call(this, type, attrs); }; })();
"""

let measure = """
(() => {
  const c = document.querySelector('canvas'); if (!c) return JSON.stringify({ error: 'no canvas' });
  const gl = c.getContext('webgl2'); if (!gl) return JSON.stringify({ error: 'no webgl2' });
  const w = gl.drawingBufferWidth, h = gl.drawingBufferHeight, px = new Uint8Array(w * h * 4);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
  const scale = w / c.clientWidth, MIN = Math.round(200 * scale), MIN_ROWS = Math.round(24 * scale);
  // Runs per polarity, top row first (readPixels is bottom-up, so walk y down).
  const runs = { white: [], black: [] }; let lowAlpha = 0;
  const hits = [['white', (o) => px[o] === 255 && px[o+1] === 255 && px[o+2] === 255], ['black', (o) => px[o] === 0 && px[o+1] === 0 && px[o+2] === 0]];
  for (let y = h - 1; y >= 0; y--) {
    const yy = h - 1 - y;
    for (const [name, hit] of hits) {
      let start = -1;
      for (let x = 0; x <= w; x++) {
        const on = x < w && hit((y * w + x) * 4);
        if (on && start < 0) start = x;
        if (!on && start >= 0) { if (x - start >= MIN) runs[name].push({ y: yy, x0: start, x1: x }); start = -1; }
      }
    }
    for (let x = 0; x < w; x++) if (px[(y * w + x) * 4 + 3] < 250) lowAlpha++;
  }
  // The same rule as extremeRuns.ts (runBlocks, SOLID_FILL, MIN_BLOCK_ROWS):
  // fill is judged PER BLOCK of runs touching on adjacent rows. One box around
  // every run let the crushed tower facade dilute a real 340x251 block to
  // fill 0.221 on 2026-09-19, which read as "no rectangle" with one on screen.
  const judge = (rs) => {
    const parent = rs.map((_, i) => i);
    const find = (i) => { while (parent[i] !== i) { parent[i] = parent[parent[i]]; i = parent[i]; } return i; };
    let ps = 0, pe = 0, i = 0;
    while (i < rs.length) {
      let e = i; while (e < rs.length && rs[e].y === rs[i].y) e++;
      if (pe > ps && rs[ps].y === rs[i].y - 1) {
        let a = ps, b = i;
        while (a < pe && b < e) { if (rs[a].x0 < rs[b].x1 && rs[b].x0 < rs[a].x1) parent[find(b)] = find(a); if (rs[a].x1 <= rs[b].x1) a++; else b++; }
      }
      ps = i; pe = e; i = e;
    }
    const blocks = new Map();
    rs.forEach((r, k) => { const root = find(k); const bl = blocks.get(root) || { count: 0, box: [r.x0, r.y, r.x1 - 1, r.y] };
      bl.count += r.x1 - r.x0; bl.box = [Math.min(bl.box[0], r.x0), Math.min(bl.box[1], r.y), Math.max(bl.box[2], r.x1 - 1), Math.max(bl.box[3], r.y)]; blocks.set(root, bl); });
    let pick = null, count = 0;
    for (const bl of blocks.values()) {
      count += bl.count;
      bl.fill = bl.count / ((bl.box[2] - bl.box[0] + 1) * (bl.box[3] - bl.box[1] + 1));
      bl.solid = bl.fill >= 0.5 && bl.box[3] - bl.box[1] + 1 >= MIN_ROWS;
      if (!pick || (bl.solid && !pick.solid) || (bl.solid === pick.solid && bl.count > pick.count)) pick = bl;
    }
    return { count, blocks: blocks.size, box: pick ? pick.box : null, fill: pick ? Math.round(1000 * pick.fill) / 1000 : 0, solid: !!(pick && pick.solid) };
  };
  const white = judge(runs.white), black = judge(runs.black);
  const dbg = gl.getExtension('WEBGL_debug_renderer_info');
  return JSON.stringify({ w, h, scale, desk: document.querySelector('[data-desk]')?.getAttribute('data-desk'), appearance: document.querySelector('[data-appearance]')?.getAttribute('data-appearance'),
    renderer: dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : null, ua: navigator.userAgent,
    solid: white.solid || black.solid, white, black, lowAlpha });
})()
"""

final class Probe: NSObject, WKNavigationDelegate {
  let window: NSWindow
  let web: WKWebView
  var shot = 0
  override init() {
    let config = WKWebViewConfiguration()
    config.userContentController.addUserScript(WKUserScript(source: preserve, injectionTime: .atDocumentStart, forMainFrameOnly: true))
    if !storage.isEmpty {
      // The JSON is embedded as a JS object literal; JSON is valid JS.
      let seed = "(() => { const e = \(storage); for (const k of Object.keys(e)) { try { localStorage.setItem(k, String(e[k])); } catch (_) {} } })();"
      config.userContentController.addUserScript(WKUserScript(source: seed, injectionTime: .atDocumentStart, forMainFrameOnly: true))
    }
    web = WKWebView(frame: NSRect(x: 0, y: 0, width: size[0], height: size[1]), configuration: config)
    window = NSWindow(contentRect: NSRect(x: -30000, y: -30000, width: size[0], height: size[1]), styleMask: [.borderless], backing: .buffered, defer: false)
    super.init()
    web.appearance = NSAppearance(named: scheme == "dark" ? .darkAqua : .aqua)
    window.appearance = web.appearance
    // SPI: without this an off-screen window is "occluded" and gets no frames.
    let sel = NSSelectorFromString("_setWindowOcclusionDetectionEnabled:")
    if web.responds(to: sel) {
      typealias Fn = @convention(c) (AnyObject, Selector, Bool) -> Void
      unsafeBitCast(web.method(for: sel), to: Fn.self)(web, sel, false)
    } else { FileHandle.standardError.write("warning: occlusion SPI missing\n".data(using: .utf8)!) }
    // TRANSPARENT=1 mirrors the shipped window: tauri.conf.json sets
    // `transparent: true` with a sidebar vibrancy effect, which makes Tauri
    // turn off the web view's own background and the window's opacity. The
    // canvas is then composited over vibrancy instead of over an opaque view.
    if ProcessInfo.processInfo.environment["TRANSPARENT"] == "1" {
      web.setValue(false, forKey: "drawsBackground")
      window.isOpaque = false
      window.backgroundColor = .clear
      let effect = NSVisualEffectView(frame: web.frame)
      effect.material = .sidebar; effect.blendingMode = .behindWindow; effect.state = .active
      effect.addSubview(web)
      web.autoresizingMask = [.width, .height]
      window.contentView = effect
    } else {
      window.contentView = web
    }
    window.orderFrontRegardless()
    web.navigationDelegate = self
    web.load(URLRequest(url: url))
  }
  func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
    DispatchQueue.main.asyncAfter(deadline: .now() + settle) { self.next() }
  }
  func next() {
    guard shot < shots else { NSApp.terminate(nil); return }
    web.evaluateJavaScript(measure) { result, error in
      print((result as? String) ?? "{\"error\":\"\(String(describing: error))\"}"); fflush(stdout)
      let finish = { self.shot += 1; DispatchQueue.main.asyncAfter(deadline: .now() + 0.9) { self.next() } }
      if !outPng.isEmpty {
        self.web.takeSnapshot(with: nil) { image, _ in
          if let image, let tiff = image.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
             let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: URL(fileURLWithPath: outPng.replacingOccurrences(of: ".png", with: "-\(self.shot).png")))
          }
          finish()
        }
      } else { finish() }
    }
  }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let probe = Probe()
DispatchQueue.main.asyncAfter(deadline: .now() + 180) { print("{\"error\":\"timeout\"}"); NSApp.terminate(nil) }
app.run()
