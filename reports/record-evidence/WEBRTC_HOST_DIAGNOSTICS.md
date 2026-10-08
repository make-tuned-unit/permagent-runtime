# Practice host WebRTC diagnosis — 2026-10-07

## Observed native runs (parent Computer Use)

1. First negotiation timed out after12 seconds; no fixture speech played. Permagent mic recording was discarded; no acceptance evidence.
2. Complete-SDP retry reached both peers connected, ICE connected, gathering complete, but preparation stalled. Removed video-play barrier, started canvas/silent-audio pumps before negotiation, added bounded per-element playback diagnostics.
3. Next retry: offer stayed `connectionState=connecting`, `iceConnectionState=connected`, gathering complete, signaling stable for15 seconds. One visible peer connected while the other stayed connecting. No speech acceptance. This is not evidence of missing ICE candidates.

## What the standards support

Overall connectionState aggregates ICE and DTLS transport state. ICE connected with overall connecting points toward unfinished DTLS or transport aggregation; that is an inference requiring individual transport evidence, not a confirmed WebKit defect. The updated fixture exposes each transceiver's sender/receiver transport state, the associated ICE transport state, final peer states and getStats transport rows. [W3C connection-state definitions](https://w3c.github.io/webrtc-pc/#dom-rtcpeerconnectionstate)

The peer-pair offer/answer flow is legal; source audit found no obvious SDP order violation. It differs from the standard sample by awaiting complete ICE gathering rather than trickling candidates. A controlled next experiment is explicit `bundlePolicy: max-bundle` to gather one media transport, or a queue-safe standard trickle exchange, compared using DTLS states and actual inbound RTP. Neither workaround has been applied or shown to solve this runtime. `max-bundle` is a normal public WebRTC configuration, not a security bypass. [W3C bundle policy](https://w3c.github.io/webrtc-pc/#rtcbundlepolicy-enum), [official peer sample source](https://github.com/webrtc/samples/blob/gh-pages/src/content/peerconnection/pc1/js/main.js)

Do not claim WKWebView generally lacks WebRTC, or that microphone permission is now proven necessary. The current host has already completed ICE and once completed both connections. Historical WebKit candidate restrictions alone do not explain this run. No private preferences, microphone grant, external STUN/TURN, or certificate weakening were added.

## Diagnostics made accessible

Log now displays newest first, each entry split into independent paragraphs of at most350 characters. Latest checkpoint and each peer diagnostic have their own text elements. The separate alert retains the actual failure. Final evidence stores connection and DTLS states before peers close. This prevents CUA's approximately512-character AX text clipping from showing only early preparation logs.

Direct mode remains selectable and explicit: no peer objects, no call claim, same scheduled hash-verified speech. Parent owns all playback/capture and should establish direct readiness before starting Record.

## Alternative dedicated browser surface (proposal, not launched)

Only `/Applications/Google Chrome.app` was found among installed Chromium-family browsers. Chromium documents a separate user-data directory as the mechanism for an independent profile/process. [Chromium user-data directory documentation](https://chromium.googlesource.com/chromium/src/+/main/docs/user_data_dir.md)

Example launch, only after parent resolves Computer Use ownership:

```sh
open -na '/Applications/Google Chrome.app' --args --user-data-dir='/Users/j/Documents/dev/permagent-worktrees/world-production-20261006/reports/record-evidence/chromium-profile' --no-first-run --no-default-browser-check --app='http://127.0.0.1:5489/'
```

That separates persisted state and browser process; it does **not** create a different macOS bundle identity. CUA must address the exact new window/PID with observed runtime support, otherwise a browser connector or app-wide quit can collide with the other session. Do not quit Chrome globally. No browser/profile was launched or created by this audit. Waiting for a released browser surface is safer than assuming profile isolation implies UI-control isolation. The existing dedicated WK host remains suitable for the separately labeled direct-audio test and continued diagnostics; it has not passed the real-call acceptance requirement.
