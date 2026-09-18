# Browser visual smoke harness

This bounded check uses the real standalone browser forum page and the
Playwright-installed Chromium instance. Set `PLAYWRIGHT_CHANNEL` only when a
specific installed browser channel is required. It does not launch a server, access credentials, mock the
daemon, create sessions, send tasks, or fabricate job state. Authenticated API
and event failures are recorded as daemon availability evidence; they are kept
separate from WebGL, asset, shader, page-error, and interaction failures.

Run it from `ui/command-center` after the forum page is being served:

```text
node scripts/verify-forum-browser.mjs
```

The default URL is `http://127.0.0.1:5285/ui/forum.html`. Override it with
either `FORUM_URL` or the first positional argument:

```text
FORUM_URL=http://127.0.0.1:5285/ui/forum.html node scripts/verify-forum-browser.mjs
node scripts/verify-forum-browser.mjs http://127.0.0.1:5285/ui/forum.html
```

The script captures day and night media appearances, waits for the successful
Solar Forum GLB response and a visible WebGL canvas, and samples distributed
RGB values from the canvas PNG to reject a uniformly blank canvas. Alpha is
never used as nonblank evidence. A framebuffer RGB sample is also recorded
when the current frame is readable, but it is best-effort because the default
WebGL `preserveDrawingBuffer` behavior may make that sample unsupported. The
harness collects page/console/request failures, HTTP daemon responses such as
401/500, and shader compile/link/context failures, and
writes:

- `assets/world/forum/browser/solar-forum-day.webp`
- `assets/world/forum/browser/solar-forum-night.webp`
- `docs/design/solar-forum/browser-report.json`

It clicks the real `Gallery` district and `Find …` sovereign controls only.
It does not click Ask, job, capability, approval, or cancellation controls.
The JSON report's `daemonUnavailable` section may be populated when the page is
not authenticated or the local daemon is offline; that condition does not by
itself mark the render as failed. Render failures set a nonzero process exit
code. The report includes the page's feed text and the selected sovereign for
manual review.

If a later interaction or stability check fails, the `finally` path still tries
to save the theme PNG and records `screenshotCaptured` or the capture failure in
the JSON report. RGB evidence uses a page screenshot clipped to the canvas
bounding box, so animated WebGL does not have to satisfy an element-stability
wait.

The harness prints and persists phase checkpoints (`browser-start`,
`page-loaded`, `asset-ready`, `canvas-ready`, and screenshot completion) after
each theme. Page and browser shutdown are bounded; if Playwright exposes the
owned child process after a shutdown timeout, only that process is terminated.
PNG parsing has bounded dimensions and input lengths, so unusual export data
cannot create an unbounded decode loop.

Console warnings such as Three.js deprecations or GPU `ReadPixels` stall
messages remain in `errors` for diagnosis but are excluded from the fatal
shader bucket; actual compile/link/invalid-operation/context-loss messages stay
fatal.

This is browser evidence for the adapter. It does not validate native Unreal's
live daemon bridge, interactive gameplay, device FPS, or GPU memory.
