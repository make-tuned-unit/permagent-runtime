# Local Record practice-call fixture

This is synthetic playback for testing the Permagent Record feature. It never requests a microphone/camera, opens a real meeting service, contacts STUN/TURN servers or starts playback on page load. Parent/operator owns Computer Use and Permagent recording.

## Launch

From the production worktree:

```sh
python3 reports/record-evidence/serve.py --port 5489
```

Open **http://127.0.0.1:5489/** through Computer Use. Server binds IPv4 loopback only and serves only this fixture directory. Stop the server with Ctrl-C. There is no external dependency or package installation.

1. Create/select a disposable `Record QA 2026-10-07` project in Permagent.
2. Open Record, choose project, choose the requested capture topology, Start recording.
3. In fixture, choose scenario and click **Start practice call**. No audio plays before that click.
4. Observe the two receiver video tracks, connected state and clock; return to Permagent while scheduled audio continues.
5. At fixture completion, click **Stop & save in Permagent**. The fixture Stop button only stops fixture playback.
6. Download run evidence from fixture. Inspect the actual saved transcript, Orchestrator triage and project Kanban separately; captions here are ground truth and never claimed as observed STT.

## Scenarios

- **Microphone smoke (20s):** Samantha near voice traverses WebRTC and plays through near receiver. Turn system-audio capture off in Permagent. Ordinary speakers into the real microphone provide an acoustic mic smoke. A pre-existing loopback input can provide clean synthetic mic input if selected through supported controls; this fixture does not install or configure routing.
- **Far-side (55s):** Daniel remote voice only; system capture on, mic quiet. Speech begins at 3s and final commitment at 49s, ending ~52.8s. Checks first chunk and final tail.
- **Duplex (100s):** two synthetic peers exchange audio, with planned overlap at 48s, pauses and last commitment at 96s. Both receiver outputs share this Mac's system mix. This is bidirectional WebRTC + echo/attribution stress. It does not prove isolated microphone/system capture. For clean duplex acceptance use established loopback routing or a separate controlled physical input/device.

Individual receiver audio can be muted with the Near/Far playback toggles; output level is independent. Toggling sound is logged. Videos are muted because each decoded receiver audio is played by exactly one audio element, avoiding a fixture-created double playback.

## Implementation and evidence

Two `RTCPeerConnection` objects use `iceServers: []`. Each sends its own participant canvas video track and Web Audio `MediaStreamDestination` track. Only decoded incoming tracks are played. No direct source audio is connected to local AudioContext output. Sources are scheduled against Web Audio time after both peers connect and all WAV files pass SHA256 verification. The UI clock may update slowly in a background tab, while scheduled audio continues. Receiver stats at stop include inbound RTP packet/byte counts, audio samples/energy and decoded video frames where the browser exposes them. Stats are delivery evidence, not proof of acoustic audibility or successful Permagent capture.

`manifest.json` records each exact script, side, onset/end/duration, voice/rate, WAV format, hash and scenario semantic expectations. `Download run evidence` exports connection/output/cue log, receiver stats and stop reason, with observed Permagent transcript explicitly null. A failed connection, fetch/hash mismatch or autoplay rejection is shown and ends fixture output. Stop/completion/navigation closes both peers, tracks and audio context.

Use expected commitments and negations from manifest, not word-perfect ASR matching. No purchase/email/launch approval or task execution is expected. Raw transcript, typed note, provenance and evidence-linked todos must remain inspectable. See `../world-production/RECORD_TEST_PLAN_2026-10-07.md` for product invariants and fault cases.

## Regenerate speech

```sh
python3 reports/record-evidence/generate_fixtures.py
```

Uses installed macOS Samantha and Daniel voices through `/usr/bin/say` file output, then ffmpeg converts to 24kHz mono signed PCM16 WAV. It does not play audio. Existing fixture files/manifest are replaced deliberately. Regeneration may change hashes across OS/voice versions; retain the current manifest with any recorded evidence. Total current speech payload is about 2.95 MB.

## Verification status

Generation completed without playback. Python AST parsing, JS module syntax check, all 11 SHA256 hashes, WAV formats and cue containment in scenario durations passed. Actual browser WebRTC connection/render/playback and native Permagent capture are **pending parent Computer Use**. Do not report these as live-tested merely because the server responds.

## Dedicated native surface (when browsers are occupied)

Built app: `native-host/Permagent Record Practice.app`, unique bundle id `ai.permagent.record-practice-20261007`. Parent Computer Use owns launch and interaction. It opens only `http://127.0.0.1:5489/`; keep the fixture server running. The host uses public WKWebView APIs, a nonpersistent website store, optional Web Inspector on macOS 13.3+, user-gesture playback, and denies device-capture permission requests. No microphone/camera usage permission is declared. Synthetic canvas/Web Audio tracks do not need device capture.

Quit with **Cmd-Q** or close its last window. This dispatches fixture cleanup and has a one-second termination fallback. It only stops this host, not Permagent Record or the separate loopback server. Stop the server with Ctrl-C in its owned session. Rebuild without launching:

```sh
/usr/bin/python3 reports/record-evidence/native-host/build.py
```

Use the system Python explicitly: the installed Homebrew Python currently has an unrelated `pyexpat` linkage failure importing plistlib. `native-host/build-verification.json` records source/binary hashes, architecture and build command. Swift compilation and strict ad-hoc signature verification passed; actual WebKit WebRTC, playback and accessibility remain pending Computer Use. A manifest link replaces the fixture in this single-window host; use Cmd-R to return. HTML download handling is not yet implemented in this minimal WKWebView host, so evidence export may need the inspector or an added public WKDownload delegate after runtime validation.

## WebKit retry and explicit direct fallback

Parent's first native host run timed out after12 seconds of WebRTC preparation; no fixture speech played, and the active Permagent recording was discarded. This is a failed call setup, not acceptance evidence.

Refresh the existing host with Cmd-R. The updated **Transport** selector keeps **WebRTC practice call** as default. Its offer/answer now carry complete gathered ICE SDP, and logs include ICE candidates, gathering/signaling/connection states, track arrival and ICE errors. Zero host candidates ends explicitly without scheduling speech. This will distinguish WebKit host-candidate suppression from a transport negotiation failure. No microphone permission or private preference has been added; capture remains denied by this running wrapper.

Choose **Direct audio test — NOT a call** explicitly if transport remains unavailable. It sends the same hash-verified, clock-scheduled WAVs to Web Audio output and displays local canvas portraits. It never creates peers and evidence records `transport: direct`, `webrtc_connected: false`. This can validate Record audio capture, tail/transcript and downstream triage, but cannot pass the actual practice-video-call requirement. No fallback starts automatically. Output toggles and level work in either mode. Inline evidence JSON remains visible even if this WK host cannot save an HTML download.

Logic regression command: `node --experimental-vm-modules reports/record-evidence/test_fixture.mjs`. Passed three mocked scenarios: direct schedule and cleanup; no-candidate failure creates no speech and closes peers; complete SDP exchange precedes connected scheduling. These checks do not claim native playback or real networking. The JS syntax check passed; parent owns the next live run.

### Connected-but-preparing stall repair

Parent's second native run reached **both peers connected / ICE connected / gathering complete**, but stayed in preparation for about a minute. This establishes connection success for that run, not media playback. The prior code awaited all four media play promises before starting the canvas frame tick; an unfulfilled video play promise could therefore hold all speech scheduling indefinitely.

The fixture now paints frames during preparation, drives both outgoing audio tracks with zero-valued sources, bounds each play promise at5 seconds, and never makes audio depend on video readiness. Log checkpoints are `frame-pump`, `silent-track-pump`, per-element `play-request` then `play-resolved` or `play-failed`, `audio-elements-ready`, `receiver-proof`, `audio-files-ready`, and `playback`. Receiver proof distinguishes actual inbound audio packets and decoded video frames from connection state; neither is claimed to prove audible output or successful Record capture. A five-second repeating stats snapshot follows the initial proof snapshot.

Logic checks now also simulate permanently pending video play promises and verify that audio scheduling proceeds with a visible video warning, and that the frame pump starts before media play requests. All fixture checks pass. Live rendering/playback remains the parent's next acceptance check. No host binary change was required.
