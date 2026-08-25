# Voice latency and the Orb — forensics, research, plan (2026-08-25)

Jesse's report, 2026-08-25 morning (iPhone, hands-free): (1) waiting for the agent
is too long; (2) the app sits on "listening" too long after they stop talking;
(3) the Orb should PULSE with their voice while they speak, and go DYNAMIC —
changing shape — while the agent speaks.

Everything below is measured from `~/.permagent/logs/daemon-sidecar.log` for the
one voice session that morning (`20260825_7`, `client=ios_voice`, 11:52–11:57 UTC
= 08:52–08:57 ADT), 8 complete turns. **No transcript text appears in this
document** — timings, tool names and file:line only.

## 1. Measured timeline

The daemon already instruments the whole turn (`crates/goose-server/src/routes/voice.rs`).
Per-turn numbers, in ms, across the 8 complete turns:

(`scripts/voice-latency-report.py --markdown`, added in this PR, reproduces this
straight from the daemon log.)

| stage | source | n | min | median | p90 | max |
|---|---|---|---|---|---|---|
| STT (Moonshine/sherpa) | `TIMING STT` | 8 | 44 | 116 | 180 | 271 |
| pre-stream (setup + ctx/recall + reply_setup) | `pipeline:` | 8 | 69 | 142 | 250 | 447 |
| **agent TTFT (first spoken token)** | `TTFT: …after stream start` | 8 | 4077 | **7355** | 9993 | 10339 |
| first-sentence TTS (Kokoro) | `STREAM sentence` | 8 | 733 | 896 | 2024 | 2697 |
| **speech-end → first audio (server-side)** | `TIMING first audio` | 8 | 6964 | **9159** | 11180 | 11994 |
| whole turn (to last sentence) | `TIMING Total` | 6 | 8398 | 13092 | 16749 | 17409 |

Four further turns that morning were sub-1.5 s taps that produced an empty
transcript and never reached first audio; they are excluded from the percentiles.

Client-side, add the endpointing hangover *before* the daemon's clock even
starts: iOS holds `listening` for `quickSilenceMs = 800 ms` (utterances with
< 3.5 s of voiced audio) or `silenceMs = 1400 ms` (everything longer)
— `ios/PermagentMobile/PermagentMobile/VoiceVAD.swift:44-56`. Most real asks that
morning were 3.3–17.1 s of recorded audio, i.e. **the 1400 ms branch**.

So the honest end-to-end number Jesse experienced is:

```
user stops talking
  → 1400 ms   iOS VAD trailing-silence hangover      (VoiceVAD.swift:50)
  →  116 ms   STT                                    (voice.rs:966)
  →  142 ms   context + recall + agent setup         (voice.rs:1557)
  → 7355 ms   MiniMax-M2.7 thinking, before token 1  (voice.rs:1750)
  →  896 ms   Kokoro synthesis of sentence 1         (ort_kokoro_backend.rs:684)
  ≈ 10.6 s median, ≈ 12.6 s p90, to the first sound
```

## 2. Root causes, in order of size

**(a) The agent is a reasoning model and thinks before every spoken word — 73 % of the wait.**
`~/.permagent/config.yaml:334-335` sets `GOOSE_PROVIDER: minimax`, `GOOSE_MODEL: MiniMax-M2.7`.
Every logged voice request (`~/.permagent/logs/llm_request.*.jsonl`) carries
`reasoning: true`, and **every assistant message in the morning's session contains a
`thinking` block** — including one-line social replies. Only one of the eight
turns made a tool call, so this is not an agentic loop: it is 4–10 s of chain of
thought in front of a two-second answer.

Two things are *already right* and should not be "fixed":
- Prompt caching is on and correct. The payload carries 4 `cache_control`
  breakpoints and the system prompt is split into a 113,501-char cached stable
  prefix + a 3,347-char volatile suffix (`crates/goose/src/providers/formats/anthropic.rs:406-441`).
  MiniMax honours `cache_control` on its Anthropic-compatible endpoint for M2.7
  (5-minute TTL, up to 4 breakpoints) — so the ~70 k-token prompt (≈29 k system +
  ≈39 k of 124 tool schemas) is *not* being prefilled cold each turn.
- TTS is already sentence-streamed and warm. `enqueue_ready_sentences` +
  `spawn_synth` (`voice.rs:1640-1710`) synthesise sentence *N* while the model is
  still emitting *N+1*; first audio goes out on sentence 1, RTF 0.27–0.35×, and
  the Kokoro model is loaded once at daemon boot (`ort_kokoro_backend.rs:544`).
  **The brief's assumption that the first chunk waits for the whole reply is false.**

**(b) The endpointing hangover is 2–3× current practice — the "stuck on listening" complaint.**
1400 ms for any utterance over 3.5 s of voiced audio. Note the client *does*
leave `listening` the instant the VAD decides — `state = .thinking` is set
synchronously in `endTurn()` (`ios/PermagentMobile/PermagentMobile/VoiceView.swift:680`),
before any daemon reply. So this is purely the silence window, not a state-machine bug.

**(c) The Orb does not carry the state it is supposed to carry.**
`VoiceOrbDrive.bands()` (`ios/PermagentMobile/PermagentMobile/VoiceOrbDrive.swift:22-55`)
takes a `level` and blends it with a synthetic `sin(t)` "breath". `listening`
floors the mic level under a 0.16–0.27 breath, so quiet or AGC-squashed speech
never reads as *the user's* voice; `speaking` uses `max(level * 1.45, residual)`
on the playback tap, which swells but never *changes shape*. `thinking` is a
slower sine of the same shape as everything else — the three states are the same
object at three speeds. With a 7 s think in the middle of every turn, an
indistinct `thinking` state is exactly what makes the wait feel unbounded.

## 3. What current practice says (validated 2026-08-25)

**Endpointing.** OpenAI's Realtime `server_vad` defaults to
`silence_duration_ms: 500`; the docs frame shorter values as "respond more
quickly, but may jump in on short pauses"
(<https://developers.openai.com/api/docs/guides/realtime-vad>). Pipecat's shipped
default is Silero VAD with `stop_secs = 0.2` feeding a Smart Turn v3 model, and
its tuning guidance is 0.2–0.3 s for conversational agents
(<https://docs.pipecat.ai/pipecat/learn/speech-input>). LiveKit splits the
decision in two: VAD detects silence, then a fine-tuned Qwen2.5-0.5B EOU model
scores the transcript, waiting `min_endpointing_delay` 0.5 s on a confident
end-of-turn and up to `max_endpointing_delay` 6 s when the text looks unfinished
(<https://docs.livekit.io/agents/logic/turns/turn-detector/>,
<https://livekit.com/blog/turn-detection-voice-agents-vad-endpointing-model-based-detection>).
The consensus: **a fixed hangover belongs at 300–800 ms**, and anything longer is
bought back with semantics, not with a bigger timer. Our 1400 ms is a fixed
timer doing a semantic job.

**Where the time goes.** Independent 2026 measurements agree with our own: LLM
inference is ~70 % of voice-pipeline latency, more than STT, transport and TTS
combined, and the working budget from STT-final to first audio is 200–700 ms
(<https://futureagi.com/blog/how-to-optimize-voice-agent-latency-2026/>,
<https://thepromptbench.com/voice-and-realtime/latency-budgets-for-realtime-voice/>).
Sentence-level TTS streaming and a warm TTS model are the two techniques we
already have; prefix caching (also already on) is worth 200–400 ms of TTFT.
Nothing in the research offers a way to make a *thinking* model answer fast —
the recommendation is uniformly to use a smaller/faster model on the voice path.

**Thinking cannot be turned off on M2.x.** MiniMax accepts
`thinking: {"type": "disabled"}` on the Anthropic-compatible endpoint but keeps
thinking anyway; the documented lower-latency escape hatch is the
`MiniMax-M2.7-highspeed` variant, which is *already declared* in
`crates/goose/src/providers/declarative/minimax.json`
(<https://platform.minimax.io/docs/api-reference/text-anthropic-api>).

**Orb conventions.** The three-or-four-state vocabulary (idle / listening /
thinking / speaking) with **listening driven by live mic amplitude** and
**speaking driven by the TTS output envelope** is the settled pattern across
Siri-style implementations, and the states are distinguished by *kind* of motion —
displacement/noise field for speaking vs. a steady breath for thinking — not by
speed alone (<https://smoothui.dev/docs/components/siri-orb>,
<https://github.com/aguscruiz/voiceorb>). The consistent implementation note is
to drive amplitude outside the render tree (MotionValue / CADisplayLink) so a
60 fps audio signal does not re-render the view each frame.

## 4. Target budget

| segment | today (median) | target |
|---|---|---|
| endpoint hangover (quick ask) | 800 ms | **500 ms** — shipped |
| endpoint hangover (dictation) | 1400 ms | **held** — see §5 |
| STT | 108 ms | unchanged |
| pre-stream | 142 ms | unchanged |
| agent TTFT | 7355 ms | **≤ 1500 ms** — needs a model decision (§5) |
| first-sentence TTS | 896 ms | ≤ 800 ms (shorter first chunk) |
| **end-of-speech → first audio** | **≈ 10.6 s** | **≤ 1.2 s** on the fast path |

The ≤ 1.2 s goal is **not reachable without changing what answers a voice turn.**
The endpointing change buys 300 ms on a short ask and 150 ms on a noise-only
open; the orb work buys perceived time, not wall-clock time. The remaining 7.4 s
is the reasoning model, and nothing on the client can touch it.

## 5. Plan

**Shipped in this PR (`feat/voice-endpointing-and-orb`):**
1. **Endpointing, quick tier.** `quickSilenceMs 800 → 500` (OpenAI Realtime's
   `server_vad` default) and `abortSilenceMs 650 → 500` on iOS; the web VAD gains
   the same two-tier split it never had (a single fixed 900 ms), so a short ask
   there also hands over at 500 ms. Both windows are runtime-tunable —
   `voice.vad.*` in `UserDefaults` on iOS, `endpointWindowMs` options on the web —
   clamped so a bad value cannot wedge a turn open or halve every sentence.
2. **Endpointing, long tier — deliberately NOT cut.** `silenceMs` stays at
   1400 ms. Every agent that runs 500 ms on a dictation-length turn pairs the
   timer with a semantic end-of-turn model, and we have none. Without one, a
   shorter window cuts people off mid-thought: `testNaturalPausesDoNotEndTheTurn`
   is the standing regression guard for exactly that reported bug (a 1.2 s pause
   after four seconds of speech must survive), and it fails at 900 ms. A false
   endpoint costs far more than 900 ms of latency. Claiming otherwise would have
   traded a measurable complaint for an unmeasured one.
3. **Orb — listening.** The synthetic breath is demoted from 0.14–0.27 to
   0.05–0.08, i.e. from *above* ordinary speech to *below* it. This is the real
   bug behind "the orb doesn't pulse with my voice": `max(breath, level)` was
   returning the breath on almost every frame, so the orb pulsed to a metronome.
4. **Orb — thinking.** A different KIND of motion: `low` (shape) held flat so it
   does not pulse, `mid` (spin) pinned high so it visibly turns. With a 7.4 s
   think in every turn, a thinking state that looks like listening is very
   plausibly part of what Jesse read as "stuck on listening".
5. **Orb — speaking.** The residual is a floor rather than a blend, so the shape
   follows the real TTS envelope; `amp` is now monotonic in the envelope at every
   phase. Reduce Motion is honoured on both surfaces — and the web's single
   static frame now redraws on a state change, which it previously never did.
6. **Tests.** XCTest for endpoint timing on synthetic frames, the tuning knob and
   its clamps, and the three orb states; vitest for `endpointWindowMs` and the
   whole orb driver, which is extracted to `orbDrive.ts` for exactly that reason.
7. **Measurement.** `scripts/voice-latency-report.py` parses the daemon's existing
   TIMING lines into the table in §1; before/after goes in the PR body.

**Needs Jesse's decision (not in this PR):**
8. **Route the voice turn to `MiniMax-M2.7-highspeed`** (same model family, lower
   latency, already in the provider manifest) or to a small non-reasoning model,
   *for the voice path only*. This is the only lever on the 7 s, and it is a
   quality/cost/latency trade-off Jesse owns — the routing principle is best-fit,
   not cheapest-first.
9. **Trim the voice turn's tool surface.** 124 tool schemas (~39 k tokens) ride on
   every spoken "how are you". They are cached, so this is a smaller win than it
   looks, but it also lengthens what the model has to think about.
