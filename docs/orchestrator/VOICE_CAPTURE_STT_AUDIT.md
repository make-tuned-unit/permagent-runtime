# Voice Capture → STT Observability Receipt (V2)

**Date:** 2026-09-04 (America/Halifax)  
**Child DAG:** V2 of `MASTER_VOICE_UX_RELIABILITY_DAG_2026-09-04.md`  
**Status:** implementation, deterministic fixture checks, and daemon compile
pass complete; daemon device gate remains V6.

## What the 16:04 AST evidence proves

`/Users/j/.permagent/logs/daemon-sidecar.log`, at 19:04:20–19:05:02Z,
records a socket connection and three stopped captures (about 1.8s, 1.6s, and
9.3s). Batch STT returned the empty string in 23ms, 23ms, and 70ms. No
transcript was emitted, so no agent/provider/TTS invocation followed. The
Spectral session `20260904_6` has zero messages.

That establishes a **no-agent-invoked** user experience. It does not establish
whether the root cause was near-silence, malformed/non-finite PCM, microphone
routing, an STT model failure, or the stale daemon binary: the historic records
did not contain PCM aggregates or a turn/socket correlation key. Do not
attribute those empty turns to the user or to a provider.

## Implemented event contract

`crates/goose-server/src/routes/voice.rs` now emits, keyed by `turn_id`,
`socket_epoch`, and the existing session ID:

| Event | Safe fields | Purpose |
|---|---|---|
| `voice_socket` | socket epoch, lifecycle stage, close reason class | correlate reconnect/close without logging client-provided close text |
| `voice_capture_health` | frame/byte/sample counters, finite/non-finite count, RMS/peak in millionths, health label | distinguish no frames, zero/near-silent valid PCM, finite signal, and malformed bytes/float values |
| `voice_latency_stage` | capture start/stop, STT outcome, provider start, timing | identify the last completed pipeline stage |
| `voice_latency_summary` | one terminal outcome, aggregate health label, stop/empty reason, latency fields | determine no-agent vs provider/TTS failure |

No raw PCM, waveform, transcript text, client close text, or secret is added to
telemetry. `transcript_chars` was already an aggregate; it remains the only
transcript-derived telemetry field.

Malformed PCM (trailing bytes or non-finite `f32` values) is rejected before
speaker/STT work with terminal outcome `capture_rejected_malformed`. Finite
near-silent PCM still goes to STT—the `-60 dBFS` label is diagnostic, not a
server-side VAD retune. Empty STT records its evidence-backed reason:
`zero_pcm`, `near_silent_pcm`, or `finite_signal_no_words`.

The provider is marked only immediately before `agent.reply`. This makes the
report distinguish `n_no_agent_invoked` from `n_agent_invoked_no_audio`.
`reply_sent` means all reply frames were queued to the socket; it deliberately
does **not** claim client playback drained. A client playback receipt remains
an iOS/V6 requirement before treating a long reply as fully heard.

## V2.1 client terminal-outcome wire contract

The server now sends one additive, typed frame **immediately before** the
existing `idle` frame for terminal no-reply captures. Legacy clients can ignore
the unknown frame and continue to reset on `idle`; V3 must read the first frame
and render a recoverable explanation before returning to ready.

```json
{"type":"turn_outcome","outcome":"capture_rejected_malformed","reason":"malformed_pcm"}
{"type":"turn_outcome","outcome":"capture_rejected_short","reason":"short_capture"}
{"type":"turn_outcome","outcome":"empty_stt","reason":"zero_pcm"}
{"type":"turn_outcome","outcome":"empty_stt","reason":"near_silent_pcm"}
{"type":"turn_outcome","outcome":"empty_stt","reason":"finite_signal_no_words"}
```

The reason enum is closed and contains no audio, threshold, timing, transcript,
or device data. Each of those frames is followed by `{"type":"idle"}`.
Speaker rejection and STT error retain their established dedicated frames; this
addition does not alter their client behavior or terminal telemetry accounting.

## Deterministic verification

- `scripts/testdata/voice_structured_observability.txt` replays three privacy
  safe fixtures: near-silent empty STT/no provider, nonempty STT + sent reply,
  and malformed PCM rejection.
- `scripts/test_voice_latency_report.py` validates legacy report compatibility
  and the structured outcomes above.
- `CaptureHealth` unit coverage includes no frames, zero PCM, near-silent
  finite PCM, finite speech-like PCM, NaN/Infinity, and trailing bytes.

Passed during this child:

```text
python3 scripts/test_voice_latency_report.py
git diff --check
env CARGO_INCREMENTAL=0 cargo check -p permagent-daemon --message-format=short
```

The focused daemon lib-test binary compiles, but its process is killed by the
OS at startup (`SIGKILL`) before any test executes. The binary is currently
about 655 MiB and links the daemon's full dependency graph. This is a test
infrastructure limitation, not a passing result or a source compilation error.
The Python fixture is the executable regression gate for this V2 node; split
the pure capture-health/report code into a lightweight test target in a later
harness DAG before requiring Rust unit execution in CI.

```text
env CARGO_INCREMENTAL=0 cargo test -p permagent-daemon routes::voice::tests --lib -- --nocapture
  # compiled, then SIGKILL before test execution
```

## V6 handoff

Rebuild both daemon and iOS app from this revision, then run one clean spoken
turn, one quiet/no-speech turn, one malformed-frame protocol fixture, and one
reply interrupted by socket close. Join the emitted session/turn/socket IDs.
Only that fresh binary/device evidence can determine the 16:04 root cause or
claim a no-response fix.

## Incremental transcript producer audit (2026-09-05)

This is a read-only source audit; no voice production source was changed.

### Current producer and transport

- iOS `VoiceView` already accepts `transcript_partial` frames through
  `VoiceTranscriptBuffer`. Partials replace one another, a final frame clears
  the partial, and a later partial cannot overwrite a final. The client sends
  16 kHz mono Float32 PCM binary frames between `start` and `stop`.
- The daemon voice wire contract in `crates/goose-server/src/routes/voice.rs`
  intentionally documents final-only `transcript` output. Its receive loop
  owns one `audio_buffer`, appends each validated binary chunk, and takes the
  complete buffer only after `stop`.
- `AppState::init_voice_providers` configures only local
  `SherpaMoonshineStt` when the offline Moonshine files exist under the voice
  model directory. There is no configured cloud STT transport on this voice
  WebSocket. The `SpeechToText` trait's cloud wording is an abstraction/future
  seam, not a live route. `/api/dictation/transcribe` is a separate local
  Whisper upload path and cannot supply voice-turn partials.
- `SherpaMoonshineStt` uses `OfflineRecognizer` and the synchronous
  `SpeechToText::transcribe(&[f32], sample_rate, config)` method. Long audio is
  split into offline windows only after capture ends; this is not streaming.
  The sherpa dependency exposes an online recognizer API, but the current
  shipped assets are Moonshine offline files (`preprocess.onnx`, encoder and
  decoders, `tokens.txt`), so switching the call site alone would not create a
  supported incremental producer.

### Ownership, cancellation, and timeout findings

- The iOS VAD owns turn boundaries and caps one listening turn at 60 seconds;
  the daemon has no independent capture-duration cap in this route. The daemon
  validates malformed Float32 frames and performs an early learned-speaker
  check once two seconds are buffered, but both the check and final STT run in
  the socket handler's receive path.
- On `stop`, the handler moves the buffer into `spawn_blocking` and awaits the
  batch STT call. While that await is active, the receive loop cannot observe a
  close or a new control frame. The `cancelled` flag is set when the socket
  loop exits and is effective for TTS work, but it cannot interrupt an already
  running offline STT call. No STT-specific timeout or cancellation token is
  currently wired.
- The safe ownership boundary for a future stream is therefore one producer
  per turn: a bounded PCM input channel, a single STT stream worker, and one
  serialized WebSocket writer. A reconnect/close must invalidate the turn
  generation so an old worker cannot emit partial or final text into the new
  socket. Partial backpressure should coalesce to the newest text; it must not
  reorder or duplicate a final frame.

### Smallest safe incremental path

1. Add an optional streaming STT capability beside (not by weakening) the
   existing batch trait. Configure it only with a supported online model and
   pinned local assets; retain Moonshine batch final-only behavior when no
   stream provider is available.
2. Give each `start` a turn/generation ID. A stream worker owns its recognizer
   and PCM queue; `stop` closes input and asks the worker for exactly one final
   result. Socket close cancels the worker and drops all output. A writer task
   emits ordered `transcript_partial` frames and then one authoritative
   `transcript` frame, with no raw audio or transcript content in telemetry.
3. Keep the existing malformed/short/empty terminal outcomes and batch fallback
   unchanged. Do not simulate partials by repeatedly calling offline
   `transcribe` on growing snapshots: that duplicates inference, risks stale
   text, and has no safe ownership when `stop` races a worker.

### Bounded regression plan

- Pure provider tests with a fake stream: chunk order/ownership, newest-partial
  coalescing, one final after `stop`, stale-generation suppression after close,
  cancellation before and during inference, and final-only fallback.
- Daemon route fixture with no model or paid provider: assert
  `start → partial(1) → partial(2) → stop → final → reply`, no partial after
  final, and close/reconnect cannot leak frames across generations. Reuse the
  existing malformed, short, empty-STT, and terminal-outcome fixtures.
- iOS protocol tests already cover partial replacement and final precedence;
  add only the wire sequence when a daemon producer exists. Device evidence
  must use a freshly built daemon plus a configured supported local streaming
  model. Until that asset/configuration exists, the honest V3 status remains
  **final-only, producer pending**.

### Pinned online asset candidate — source verification September 5

The [Sherpa small-model documentation](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/small-online-models.html)
lists the English 20M streaming Zipformer. Its [pinned upstream repository](https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/tree/d42f2d9f7ca24806fb667456a18a9f1b60f70d16)
declares Apache-2.0. Root verified the adapter's three ONNX filenames against
that revision and the published file SHA-256 values:

| File | Published SHA-256 |
|---|---|
| `encoder-epoch-99-avg-1.int8.onnx` | `3810755ce7c3ab26b42a8bcf39d191308fa27fb0f53358823ba46141d03b7eb3` |
| `decoder-epoch-99-avg-1.onnx` | `45a7f940ecfb53d89fa270ad11b88b961e53a317203eb24b1c8e95ed208b0f30` |
| `joiner-epoch-99-avg-1.int8.onnx` | `e085d73b593cf9b0707f370dbd656d58327d3fe36d80d849202ef81df02cb01e` |

`tokens.txt` must come from the same pinned revision. This is source/provenance
research, not an installed-model integrity receipt. No model was downloaded or
enabled by this check. Presence of four filenames alone is not integrity
validation, and KWS assets are not substitutes. Accuracy/last-word preservation,
real-time factor and noisy-speech behavior still require a real local fixture.

Root review of the staged producer additionally requires speaker admission
before publishing partials, bounded live worker count under repeated starts,
and centralized cleanup on disconnect. Trait/adapter tests alone cannot pass
those route/device boundaries.

### Local fixture staging receipt — subsequent September 5 check

Root downloaded the four model files from the pinned revision above into
`/private/tmp/permagent-online-stt.IAD6Nb`, outside production model storage.
All three computed ONNX SHA-256 hashes match the published values above.
The computed token-file SHA-256 is
`49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb`;
this records local integrity, not a separately published token checksum.

Public `0.wav`, `1.wav`, and `trans.txt` fixtures from that same revision were
also staged. Their Git blob hashes match the upstream tree respectively:
`bfe1519ead65b33e26f8b81f25b1c072cbb90d13`,
`498b3f3357ccffa1a9deece350839dc58e2dd5c5`, and
`c3b9759d692309cba86e64e465e1a08220ee01c3`.
No user audio was uploaded, no running-app setting was changed, and no model
inference or accuracy result is claimed by this staging receipt. The next gate
is an opt-in local fixture execution after the daemon compiles.
