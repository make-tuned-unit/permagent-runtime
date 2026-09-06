# Voice UI component and state contract

**Child DAG:** V1 of `MASTER_VOICE_UX_RELIABILITY_DAG_2026-09-04.md`  
**Status:** V1 complete (contract only; no production code changed)  
**Date:** 2026-09-04 (America/Halifax)

## First-party research constraints

This contract uses product behavior as evidence, not as a visual template:

- OpenAI documents that current Voice can keep text and voice in the same chat,
  shows responses as text while spoken, retains a post-call transcript, exposes
  explicit mute/exit controls, and warns that background noise and overlapping
  speech can cause interruption. Permagent should therefore preserve one
  conversation, keep visible readable text, expose explicit recovery/control
  state, and measure false interruption rather than hiding it.
- Anthropic documents hands-free and push-to-talk as complementary modes,
  automatic prompt population on supported surfaces, natural-pause handling,
  in-conversation model selection, same-conversation text/voice continuity,
  and a push-to-talk fallback for noisy environments. Permagent should retain
  both modes, make the active model actionable, and never leave failed capture
  looking like successful listening.

Sources (retrieved 2026-09-04):

- https://help.openai.com/en/articles/20001274/
- https://support.claude.com/en/articles/11101966-use-voice-mode

## Purpose and boundary

This contract translates the supplied Claude reference screenshots and the
current Permagent implementation into behavior that can be implemented
surgically. Claude is a usability reference, not a source of assets or code.
The existing WebSocket, Spectral/session storage, provider catalog, and
`ModelPickerView` endpoint contract remain authoritative.

Owners below are file-level ownership boundaries for later child DAGs:

| Surface | Component owner | Responsibility |
| --- | --- | --- |
| Voice header and conversation | `VoiceView.swift` | state, transcript placement, model action, status/recovery |
| Chat composer | `Views.swift` | actionable provider/model capsule; no inert identity control |
| Provider/model selection | `ModelPickerView.swift` | configured providers, expandable models, next-turn semantics |
| Wire parsing and ranges | `VoiceProtocolTypes.swift` | ordered partial/final text and UTF-16-safe timing ranges |
| STT production and telemetry | `crates/goose-server/src/routes/voice.rs` | event ordering, terminal outcomes, privacy-safe diagnostics |

V1 does not change these files. V2 owns daemon observability before V3 can
claim streaming transcript support. V3/V4/V5 own the later iOS implementation.

## Interaction contract

### State matrix

| Engine state | User transcript | Agent reply | Primary status | Required actions |
| --- | --- | --- | --- | --- |
| `connecting` | Placeholder remains visible: “Connecting…” | Hidden | “CONNECTING…” | End voice; controls are disabled |
| `ready` / hands-free | Last final user text may remain as history | Last completed reply remains readable | “LISTENING FOR YOU” | End voice; model button; hands-free toggle |
| `listening` | Show newest provisional text immediately; label it provisional | Prior reply remains readable but inactive | “LISTENING…” | End/cancel capture; model button |
| `thinking` | Keep final user text visible | Show reply loading state without erasing prior turns | “THINKING…” | Cancel turn; model button |
| `speaking` | Keep final user text visible | Show the complete reply; highlight exactly the current spoken word | “SPEAKING” | Interrupt; model button |
| `failed(reason)` | Preserve captured/final text if available | Preserve any readable reply | “VOICE UNAVAILABLE” plus reason | Retry/reconnect; end voice; model button when usable |

Every turn must end in a visible terminal state. Empty STT must say what
happened (for example, “I couldn’t make out speech. Try again.”) and expose a
retry action; it must not leave only “LISTENING FOR YOU” on screen. Server
terminal outcomes are defined by V2 and are not guessed by the client.

### User transcript

The user’s words belong above the agent reply in the conversation region and
must be visible while capture/decoding is in progress:

- `transcript_partial` replaces the previous partial atomically and is styled
  as provisional (muted text and an accessible “still transcribing” hint).
- A final `transcript` replaces the partial exactly once, uses normal text
  contrast, and remains visible for the turn.
- A later partial must never overwrite a final transcript.
- New turns reset only the provisional buffer; they do not erase prior history.
- Long, multiline, emoji, apostrophe, and punctuation input must wrap without
  hiding the status or controls.

The accessible element has the label **“What you said”** and exposes the full
current turn as its value. If the producer is final-only, the UI must honestly
show final-only behavior; it must not manufacture partial text.

### Agent reply and speech synchronization

The complete reply is rendered in a bounded, vertically scrollable transcript.
It remains readable after audio ends. While audio is playing:

- highlight only the active spoken word/range using the timing metadata;
- scroll that range into view with minimal movement, including across
  paragraphs;
- never jump to the bottom merely because the reply arrived;
- preserve a user’s manual scroll position unless the active range is outside
  the viewport, then use the smallest scroll needed to reveal it;
- clear the old highlight when a turn is interrupted, disconnected, or ends;
- treat missing, malformed, negative, reversed, or out-of-bounds timing as
  “audio readable, highlight unavailable,” never as a crash or invalid range.

Accessibility exposes the entire reply as **“Agent reply”**. The active-word
visual treatment is supplemental and must not be the only way to understand
the sentence. Dynamic Type and Reduce Motion must preserve reading order and
avoid animated jumps.

## Identity and model affordances

The name “Henry” is an agent identity, not a fake model selector. The current
chat composer’s plain `Text(identity.nameCapitalized)` is therefore replaced
in V5 by one clearly actionable provider/model capsule. Recommended copy:

- collapsed label: **“Model · {model}”**;
- accessibility label: **“Choose model. Current model {model} via {provider}.”**;
- unavailable label: **“Model unavailable. Open model settings.”**

Tapping the capsule opens the existing `ModelPickerView` for the relevant
scope. The picker keeps provider rows expandable, lists configured models only,
and applies a selection to the next turn. The current turn keeps its original
provider/model. No provider endpoint or model route is duplicated in the UI.

The Voice header uses the same action and terminology. Replace the current
hardware-oriented `cpu` glyph with a configuration/model glyph such as
`slider.horizontal.3` (preferred) or `wand.and.stars` if the icon set makes the
former unavailable. The glyph is secondary to the visible model name and has
the same accessible label as the capsule. It must be tested at 1x/2x, with
Dynamic Type, VoiceOver, and Reduce Motion; it must never imply local hardware
inference.

“Henry” remains the agent identity in the Voice header and status copy, e.g.
“Henry is speaking,” and must not be styled as the model control. The existing
typed agent roster provides a separate contract: tapping the header identity
opens agent controls, while the model action opens provider → model selection.
One control must not ambiguously switch both agent identity and model.

## Layout contract

For iPhone portrait widths 375–430 pt:

1. Header: `VOICE`, Henry identity, model action, end action.
2. Conversation region: user transcript, status/recovery message, then full
   agent reply; the orb may occupy remaining space but may not cover feedback.
3. Controls: interrupt/cancel or push-to-talk, then hands-free toggle.

The conversation region must have a minimum visible height while the orb is
present. Transcript and reply must remain usable above the bottom safe area and
keyboard. No control may look tappable while disabled without a loading or
unavailable explanation.

## Acceptance tests for V1 handoff

These are implementation gates for V3–V6, not claims that V1 has run them.

| ID | Scenario | Pass condition |
| --- | --- | --- |
| UX-01 | `ready → listening → partial(1) → partial(2)` | newest provisional user text is visible and old partial is replaced |
| UX-02 | partial → final → reply | final supersedes partial once; both user text and reply remain visible |
| UX-03 | 3-paragraph reply with ordered timings | active words highlight correctly and scroll into view without bottom-jump |
| UX-04 | emoji, apostrophe, repeated word, malformed timing | correct UTF-16 range or readable unhighlighted fallback; no crash |
| UX-05 | empty STT, STT error, disconnect | terminal explanation and retry/reconnect action are visible |
| UX-06 | tap chat model capsule and Voice model icon | existing provider list expands; configured model selection is observable |
| UX-06a | tap the Voice header identity | existing agent controls/roster opens; model selection does not open |
| UX-07 | switch during active turn | current turn remains on its original model; next turn uses selected route |
| UX-08 | VoiceOver, Dynamic Type, Reduce Motion, 375 pt width | labels, focus order, wrapping, and reduced animation remain usable |
| UX-09 | disabled/loading picker and failed model route | state is explicit; active model is not silently changed |
| UX-10 | rebuilt app + daemon | evidence identifies matching source/build revision before device claims |

## V1 verification receipt

- V0 evidence lock is present in the master DAG and is marked passed.
- Current source inspection confirmed the existing partial/final transcript
  bindings, reply timing/highlight view, inert chat Henry `Text`, CPU icon, and
  expandable provider/model picker.
- Contract covers all V1 work items, file ownership, state transitions,
  accessibility, narrow-width behavior, and downstream acceptance gates.
- No production code was edited by this child.
- Child exit: **passed**. V2 may instrument capture/STT in parallel; V3–V5
  implementation remains gated on this receipt.
