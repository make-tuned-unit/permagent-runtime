# U3 onboarding: first useful moment

Status: researched/specification; not implemented or visually accepted.
Parent: UI_POLISH_MASTER_PROGRAM_DAG.yaml, u3_shell / MAC-WIZARD.
Operator explicitly included the complete setup wizard on September 6.

## O1 findings — September 6

Eight current Moments are provider, hardware, calibration, intent, code roots,
persona/voice, web search, and chat. The audit found these concrete defects:

- WizardShell mounts all Moments, hiding them with opacity/pointer events.
  Inactive steps still run hardware/start-local-runtime, secret reads, voice
  status, code scans and timers, and remain in keyboard/accessibility order.
- App config-read failure eventually enters onboarding, conflating outage
  with first use. Provider setup advances without the existing readiness probe.
- Hardware transport failure can auto-start Ollama; unknown RAM becomes zero.
  Download cancellation is exposed by the API but not wired into the journey.
- Code scan errors appear empty; saved web-search credentials appear connected
  without successful verification.
- No persisted resume cursor; persona and intent remain local until completion.
- Chat types a local greeting and displays Online without a real model reply.
- VoicePicker's displayed default may not reach the saved persona.voiceId.
- Raw backend errors reach console logging; saved keys remain in hidden local
  state. No actual key disclosure was observed; add sentinel regressions.

First implementation slice assigned: inactive-step side effects and focus/
accessibility isolation, preserving Back state with existing component patterns.
Other findings remain explicit O3/O4 work; this is not an onboarding pass.

O3 checkpoint: active-step gating/inert isolation is implemented; focused worker
tests passed (15). Startup recovery distinguishes unavailable/malformed config
from confirmed first-run state (7 focused startup/workspace tests passed).
Hardware late-result fencing remains under repair, not implicitly covered.

Root executed `node ui-polish-onboarding-evidence.mjs` against synthetic,
network-isolated config/provider responses. Dark and silver at 960x720 both
showed exactly seven inert hidden steps; no hardware, code-root or voice request
was observed. Screenshots were inspected at
`/private/tmp/permagent-onboarding-dark.png` and
`/private/tmp/permagent-onboarding-silver.png`. This is first-step browser
evidence only, not all-step/native acceptance. External font fetching was
blocked by the fixture, so typography uses available fallback fonts.

Visual review: current welcome is orderly and theme-consistent but still leads
with provider credentials instead of user value. The O2 direction is not
yet implemented: stronger personal introduction, progressive optional setup,
and genuine first interaction remain work. Do not relabel the lifecycle fix as
the requested complete visual redesign.

## Evidence and direction

[Raycast Quickstart](https://manual.raycast.com/quickstart) introduces practical
actions first and leaves deeper discovery to later use. Permagent inference:
finish setup with one genuine, useful interaction instead of a feature lecture.

[Craft Introduction](https://support.craft.do/en/introduction) organizes learning
around basic workspace/document concepts, then deeper tools. Permagent inference:
teach the companion, workspace, and control relationship in context; keep model
infrastructure detail available without dominating the welcome.

[Apple onboarding guidance](https://developer.apple.com/design/human-interface-guidelines/onboarding)
recommends a short, enjoyable, optional introduction and contextual instruction.
The page's search extract was available; full page requires JavaScript. These
sources support progressive learning, not a proven retention increase or a
pixel-accurate audit of competitors' installed onboarding.

Design hypothesis: a calm, luminous introduction to a living companion.
Use the existing cyan/violet identity, restrained glass controls, generous
typography, one focal illustration/agent presence and one primary action per
step. Respect reduced motion; never make a cinematic delay mandatory. The
payoff is a real first answer with truthful readiness, not simulated success.

## O2 visual direction for root review

- **Arrival:** preserve the authored identity asset; use a restrained luminous
  backdrop and a strong, short heading. No looping mandatory splash or busy
  dashboard before a person understands the companion. Existing motion tokens
  and reduced-motion support govern transitions.
- **Connection:** provider/local choices are clear cards within the existing
  wizard shell, with a short plain-language consequence and optional technical
  disclosure. Show verified, checking, unavailable and saved-but-unverified as
  different states. Never rank by provider brand alone.
- **Personalization:** preview the chosen name/tone live; make optional choices
  genuinely optional and editable later. Separate a preview label from an
  actual agent response. Don't force a personal story to finish setup.
- **Permissions and enhancements:** explain why at the point of use. Downloads
  show progress, size and cancellation; voice/search/code roots can be deferred
  with an accurate statement of what remains available.
- **First useful moment:** carry the user's chosen intent to the existing chat
  once, with a clear Send action unless explicitly authorized during setup.
  The final transition into Home should preserve context, not discard their
  choices or send a surprise task.
- **Layout system:** stable header/progress region, one central content area,
  consistent primary/back/skip row; narrow windows scroll content without losing
  the action row. Use existing Button, theme/spacing/type tokens and wizard
  atoms, not a second component library. Verify glass legibility over actual
  backgrounds instead of adding transparency to every panel.

These are design decisions to implement and validate, not measured engagement
claims. Retention must not be optimized through forced permissions, hidden skips
or misleading success. The eight current Moments may remain internally while
the visible journey groups them coherently; avoid a state-machine rewrite solely
for appearance.

## Sequential child gates

1. **O1 state and capability audit.** Enumerate every WizardShell Moment:
   provider, hardware/local model, calibration, intent, code roots, identity/
   voice, web search, first chat. Map read/write APIs, secrets, permissions,
   skip/back/resume, failures and completion persistence. Inspect whether
   inactive mounted steps request permissions or start background work.
   Gate: every step has explicit ownership and required/optional rationale.
2. **O2 journey and visual specification.** Separate essential usable-model
   setup from optional enrichment/search/voice/code access. Show measured local
   hardware and download sizes, provider choice without brand favoritism,
   progress and honest skip consequences. Defer optional setup accessibly to
   Settings; no account login requirement invented where none exists.
   Gate: keyboard/compact/light/dark/reduced-motion designs and recovery states.
3. **O3 regressions before surgical implementation.** Cover offline hub versus
   genuine first run, invalid credentials, no local model, interrupted/cancelled
   download, denied permission, repeated Next, Back, restart/resume, save failure,
   optional skips and secrets absent from logs. Reuse existing components,
   config and state seams; no parallel onboarding framework or credential store.
   Gate: failing cases demonstrated, patches preserve saved user choices.
4. **O4 first-use integration.** Verify identity/config persistence and first
   intent handoff exactly once, actual model readiness, and truthful unavailable
   state. Skipping enrichment must not claim all features work without a usable
   inference provider. Config-read failure must not become first-run detection.
   Gate: isolated real endpoint journey plus reload/restart evidence.
5. **O5 visual and native acceptance.** Verify every step at compact/normal
   windows, keyboard-only and screen reader, relevant themes and motion modes.
   Record time to usable interaction, forced steps, recovery outcomes and
   render responsiveness separately from downloads/model latency. Use synthetic
   fixtures; don't collect raw keys, prompts or private paths in telemetry.
   Gate: root visual review and native journey evidence, then u3/u16 fan-in.

Verification is automatic within approved scope; only new permissions or
external effects beyond that scope require operator involvement. A passing
static screenshot cannot substitute for setup completion. Subsequent iOS/watchOS
adaptations follow macOS acceptance and preserve the same identity natively.

## O3 gate execution — September 6, resumed lane

The "hardware late-result fencing remains under repair" line above is stale: it
was written at 14:38 and the repair landed in `MomentHardware.tsx` at 14:53,
alongside the matching `WizardStep.lifecycle.test.tsx` cases at 14:43. The lane
stopped before recording it.

Fencing as it now stands in `MomentHardware.tsx`: three independent generation
counters (`scanGenerationRef`, `statusGenerationRef`, `operationGenerationRef`),
each paired with a `current()` predicate that requires the step still be active
AND the generation still be the live one, checked after every await rather than
once at entry. Deactivation aborts an in-flight model pull via `pullAbortRef`,
bumps the operation generation, clears `ollamaStarting`, and rewinds a
`downloading` phase to `recommend` so re-entry offers an honest restart instead
of an orphaned completed-looking promise. `applyConfig` takes the predicate as
an argument, so a late config write cannot land after the user has left.

Reproduction, run on this snapshot: reverting only `MomentHardware.tsx` to its
committed version fails exactly four cases — inactive-step scan/Ollama start,
late Ollama response after deactivation, stale installed-model result, and a
model pull resolving after deactivation — and the working file passes all eight.
The file was restored byte-identically afterwards.

Gates executed on the uncommitted working tree, all green:

| Gate | Result |
|---|---|
| `npx tsc --noEmit` | clean, no output |
| `npx vitest run` wizard + voice + App.startup | 10 files, 48 tests passed |
| `npx vitest run` full command-center suite | 261 files, 1930 tests passed |
| `npx vite build` | built in 16.57s (pre-existing >500 kB chunk warnings only) |

What this does and does not establish: the O3 inactive-step and late-result
isolation slice is implemented and covered. It is not O2 visual acceptance, and
the O2 direction recorded above — leading with user value instead of provider
credentials, progressive optional setup, genuine first interaction — remains
unimplemented. O4 first-use integration and O5 native/visual acceptance are
untouched. No screenshots were taken in this pass; the earlier browser evidence
is still first-step only.

Remaining O1 findings with no implementation yet: code-scan errors reading as
empty, saved web-search credentials appearing connected without verification,
no persisted resume cursor, persona and intent staying local until completion,
the chat step typing a local greeting while displaying Online, and the sentinel
regressions for raw backend errors and retained keys.
