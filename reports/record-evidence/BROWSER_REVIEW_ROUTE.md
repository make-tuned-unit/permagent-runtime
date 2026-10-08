# Isolated browser QA at5302

Prepared only; this worker has not launched a server, created a claim, opened a browser or read any live bearer.

## Why a separate browser server

`routes/mod.rs::ui_dist_path` serves `/ui` only from `PERMAGENT_UI_DIR` or a `ui/command-center/dist` beneath the debug executable's ancestor directories. The isolated daemon's clean child environment does not include that override, and the searched ancestor locations under `.shared-target`/the worktree container have no such dist. This source/filesystem inspection does not claim an HTTP request was made. A5302 Vite server serves current integration frontend source directly, avoiding a stale/native-root asset build.

`ui/command-center/vite.record-review.config.mjs` is standalone and never merges the default config's3001 proxy. It binds only127.0.0.1:5302, refuses a busy port and does not open a browser. Both API-base defines point to5302; every literal daemon route prefix discovered in non-test Rust source is proxied to exactly127.0.0.1:3002, including pairing, streams/WebSockets, config, sessions, activity and less common API roots. Origin guard source permits loopback origins on arbitrary ports. There is no production fallback or credential injection in the proxy.

The config exposes no environment variable prefixes, restricts Vite file serving to frontend/shared UI source, and does not expose `.shared-target` or the isolated runtime. The launch helper passes a clean environment (no inherited VITE/backend overrides, DEBUG proxy logging or provider keys) and limits Node's old-space heap to1GiB. Proxy/build errors suppress details in terminal logs; informational URLs redact token/claim/code fragments. Browser errors remain visible for QA.

## Node launch preflight

The initial launch failed because the clean environment searched Homebrew/system directories, while this machine has Node through mise. The config itself was present. The helper now pins the already-installed canonical executable `/Users/j/.local/share/mise/installs/node/24.19.0/bin/node` (v24.19.0); it does not follow the moving `lts` alias or search inherited PATH. The sanitized child PATH starts with that executable's directory. Missing/non-executable/redirected Node, missing isolated config and missing Vite entry point have separate errors.

`dry-run` validates these local prerequisites and prints the exact command without contacting the daemon or starting a server. After this fix, five Python helper tests and three Node config tests passed; the dry run succeeded. No server, pairing, UI, frontend/native build or runtime was started by the fixing worker.

## Parent execution after daemon readiness

```sh
python3 reports/record-evidence/isolated_browser.py dry-run
python3 reports/record-evidence/isolated_browser.py start --max-seconds 3600
```

Keep the foreground command session. Start validates the isolated artifact manifest and authenticated3002 readiness, then launches one owned Vite process group. Interrupt/terminate that supervisor or let its deadline expire to stop only that owned group. No installed app or daemon is started/stopped by this helper. The parent remains sole CUA operator.

Use the exact browser origin `http://127.0.0.1:5302/ui/` (not a shared production tab or a different hostname/origin).

## Legitimate browser pairing

The normal public UI claim flow is preferable to placing a daemon bearer in localStorage or a URL. Source contracts:

1. Authenticated `POST3002/api/devices/pair` with a synthetic device name mints a random128-bit, single-use claim expiring after10minutes.
2. The browser opens5302 `/ui/#claim=...`. `api.ts` immediately scrubs the fragment and posts the code to public `/pair/claim` through the same-origin5302 proxy.
3. The daemon returns a fresh per-device token once. The normal frontend persists it in that origin's localStorage and emits the authenticated `devices_paired` event. The master daemon bearer never enters the browser.

With the owned5302 server already running, the parent can prepare the claim:

```sh
python3 reports/record-evidence/isolated_browser.py pair-link
```

The helper uses only the isolated root's bearer in memory for the protected pairing request. It writes the claim URL to a unique0600 file in canonical `<isolated-root>/browser-pairing/` (0700), and returns **only the file path**. It does not print token/code/URL fragments, touch the system clipboard, redeem the claim, or manipulate a browser. The parent can read this private file within its CUA workflow, navigate using a variable without printing it, and inspect the page only after normal claim redemption has scrubbed the fragment. Do not include the private handoff file or raw URL in evidence. If expired/already consumed, mint a new one through this same API; do not copy a production token.

Alternative when native windows become available: use the isolated app's Settings→Devices to create the one-use claim, then the browser's pairing-link flow or World→“Pair this tab” box. The origin that redeems the code is the paired origin. Opening a3002 URL would not pair the5302 browser storage.

After pairing, verify the new device via Settings→Devices or the public-metadata-only authenticated device list. Record its ID/name if later revoking the test device; never capture its bearer. Test cards/source-note links, the two synthetic project boards, Orchestrator context and reversible synthetic CRUD through CUA. Browser QA cannot substitute for native capture/stop-tail or mic-permission acceptance.

Validation: three Node config tests passed (all discovered source prefixes, fixed loopback targets/no fallback, logger redaction) and two Python helper tests passed (legitimate API/private handoff/no secret output; clean child environment). Syntax/dry-run passed. No launch, pairing request, model inference, browser interaction or build was performed.
