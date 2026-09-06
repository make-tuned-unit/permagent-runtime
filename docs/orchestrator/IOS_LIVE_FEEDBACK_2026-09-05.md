# iOS live feedback — 2026-09-05, 10:14–10:15 Atlantic

Status: reproduced integration defects; fixes pending runtime/device verification.

Evidence: operator screenshots IMG_6330.PNG and IMG_6331.PNG; existing daemon log
`~/.permagent/logs/server/2026-09-03/20260903_195445-permagentd.log`.
Times below are UTC (local time was UTC−03:00).

## Observed, not inferred

- POST `/config/model-route` returned 404 at 13:14:10, :19, :26, and :35.
  The installed daemon predates this route. The phone's blanket API-key error
  was incorrect; no credential failure is demonstrated by these requests.
- At 13:14:56.583, STT completed in 177 ms for the reported greeting.
- Actual provider invocation logged `model=deepseek-chat`; the phone displayed
  `claude-sonnet-5`. A saved global default is not proof of resolved voice routing.
- Server first-audio generation: 5,777 ms after speech end; first Kokoro
  synthesis 2,264 ms for 1.6 seconds of audio, second synthesis 761 ms.
  Total server turn: 6,545 ms. These are server measurements, NOT audible
  playback receipts from the phone.
- Client closed the socket at 13:15:04.691 and reconnected at :04.703.
  Cause is not established from server logs alone.
- Operator reports no audible response. Screenshot shows reply text but does
  not prove audio playback. Orb shrank and user transcript was placed at top.

## Closure graph

1. Diagnose exact endpoint and actual model from logs — passed above.
2. Correct model-switch error classification and stop presenting legacy global
   defaults as verified voice routing — source changed; new regression tests.
3. Preserve full-size orb, move live user transcript above lower controls,
   retain assistant scrolling/highlighting — worker source changed; tests queued.
4. Audit playback completion and reconnect lifecycle; reproduce any identified
   defect before repair. No assumption that a rebuild fixes missing sound.
5. Run regenerated-project Swift regression suite and app/watch compile gate.
6. Complete daemon verification and controlled compatible daemon deployment.
7. Device acceptance: switch model, confirm actual next-turn provider/model,
   hear response, observe word highlighting and multi-paragraph scroll, retain
   full-size orb, and confirm bottom transcript placement. Still open.

No provider secrets were read or changed. No model call was added for this audit.
