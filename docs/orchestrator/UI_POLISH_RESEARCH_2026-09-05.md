# Permagent: native-quality polish research

Research date: 2026-09-05. Root owns direction and independent review; the
parallel inventory worker owns source enumeration. This is research and a
design brief, not a completed visual inspection of installed competitors or
Permagent. No competitor assets, frameworks, or UI implementations are copied.

## Evidence and limits

The operator's [r/macapps thread](https://www.reddit.com/r/macapps/comments/1ku9l8z/what_are_the_bestdesigned_mac_apps_youve_used/)
names Craft, Raycast, Bear, Things, Pixelmator Pro, IINA, CleanShot X, Bike,
Dropover, mymind, Panic apps and LookAway among its references. It contains
disagreement about capability, usability and reliability, not a controlled
ranking. Treat it as a discovery list, not proof that a popular look is better.
Historical remarks about discontinued apps are not current support evidence.

The primary-source observations below describe documented features. The
Permagent implications are our design hypotheses to test with real journeys.
Marketing performance claims are not imported as measured benchmarks.

| Reference and primary source | Observed emphasis | Permagent hypothesis |
|---|---|---|
| [Craft](https://www.craft.do/) | Documents, embedded tasks, spaces, folders and collections connect creation with organization | Give project artifacts a calm, readable home; put the relevant next action beside the work instead of multiplying dashboards |
| [Raycast Action Panel](https://manual.raycast.com/action-panel) | Selected-item actions, primary action, searchable secondary actions, keyboard hints and back navigation | Make every object actionable in a predictable place; reuse current command/navigation actions rather than a second dispatcher |
| [Things Quick Find](https://culturedcode.com/things/support/articles/2803584/) | Search doubles as navigation, including keyboard entry | Make destinations and actions findable without requiring users to remember which tab contains them; do not hijack typing in editors or terminal |
| [Bear](https://bear.app/) | Focused Markdown writing, organization and cross-device notes | Establish comfortable transcript/document measure, clear hierarchy and restrained chrome; preserve exact content and citations |
| [Pixelmator Pro](https://www.apple.com/pixelmator-pro/) | Nondestructive editing, layers and a customizable workspace | Keep complex powers contextual and reversible; previews must not imply an action has already been committed |
| [CleanShot X](https://cleanshot.com/) | Immediate capture-to-copy/save/drop, history and on-device text recognition | Attachments should remain in the receiving pane with progress, usable previews, extraction status and recovery; OCR failure is actionable, never silently ignored |
| [Bike](https://www.hogbaysoftware.com/bike/) | Responsive editing, scrolling and resizing; keyboard focus, breadcrumbs, stable deep links | Treat input latency and navigation continuity as design requirements; maintain scroll/selection through updates and returns |
| [Dropover](https://dropoverapp.com/) | A temporary shelf for drag-and-drop workflows | Give drops an explicit destination and inspectable pending state; do not add a redundant permanent file store |
| [IINA](https://iina.io/) | macOS-oriented media playback and controls | Media ownership must be clear; closing a tab must release its playback, and hidden-view behavior must be predictable |
| [Panic Nova](https://nova.app/) | Editor, workflow, Git and file-transfer tools in a Mac-native workspace | Build should feel like one workspace, with tool state and navigation coordinated rather than several unrelated mini-apps |
| [mymind](https://mymind.com/) | Visual saving and retrieval of varied content | Use meaningful previews and recognition cues in memory/artifacts; keep Spectral provenance and explicit forgetting, not a parallel memory system |
| [LookAway](https://lookaway.com/) | A focused break-reminder experience | Use brief, purposeful feedback; never add distracting animations, nagging or unsolicited sound in the name of delight |

Apple's [Meet Liquid Glass](https://developer.apple.com/videos/play/wwdc2025/219/)
and [Materials guidance](https://developer.apple.com/design/human-interface-guidelines/materials)
provide the platform reference for a distinct control/navigation layer with
legibility and accessibility adaptations. Use native materials in existing
SwiftUI/AppKit surfaces where supported. The command center is an existing web
UI: a CSS blur is not native Liquid Glass, and a framework rewrite is not part
of this program. Its existing material tokens should express compatible visual
hierarchy with opaque/high-contrast fallbacks.

## Direction: a composed, living workspace

Permagent should be recognizably itself: a capable companion with a calm work
surface and the expressive solarpunk World. Avoid a collage of copied apps.

- Content first: readable typography, breathing room, aligned controls and one
  primary action per working context. Do not equate more glow with more polish.
- Progressive disclosure: common work stays visible; advanced actions remain
  discoverable through contextual controls and existing search/commands.
- Agent truth is part of the design: show actual model/provider, current task,
  cost status, approval boundary, provenance and recovery when relevant.
- Continuity: keep object identity, navigation, selection, draft, scroll and
  focus stable across side-pane resize, detachment, reconnect and device handoff.
- Delight follows successful work: restrained completion feedback, meaningful
  previews and responsive motion. No fake progress or animation masking a stall.
- Glass frames content; it does not reduce transcript, data-table or form
  readability. Respect system appearance, reduced motion and transparency.

## Capability placement contract

Inventory capability producers as well as UI routes. Every supported capability
needs a named primary home, contextual entry points where useful, its existing
API/command, required authority, pending/error/unavailable state and verification.
Backend-only administrative/internal primitives are not indiscriminately exposed;
record why they are internal. Unsupported platform operations get a clear
supported handoff, not a dead control. Security-sensitive actions remain gated.

Examples to reconcile against implementation: model/provider selection belongs
beside the current conversation/terminal and in defaults; recall inspection and
correction belong beside cited memory and in Brain; spending belongs beside a
run and in Spend; approval belongs beside the affected work and in Decisions;
referral sources belong in Grow with safe Build-browser navigation. These are
placement requirements, not claims that every path is already wired.

## Reuse before invention

Start from `styles/tokens.ts`, `useTheme`, `ViewHeader`, `Button`, `Chip`,
`Toggle`, `StateBlock`, `JobProgress`, `AsOf`, `DetailModal`, `FormModal`,
`ConfirmDialog`, the existing icon set, Settings atoms and ProviderModelPicker.
Preserve their regression budgets; do not expand raw-button, tiny-text, radius
or icon exceptions to make a migration pass. Extend an existing primitive only
when at least two real consumers need the behavior and both are migrated/tested.
Reuse existing SwiftUI design primitives separately on iOS/watchOS; share
semantics and tokens, not an inappropriate identical layout.

## Acceptance rubric

Before/after evidence must use the same data, viewport, appearance and source
snapshot. Review hierarchy, consistency, discoverability, responsiveness,
accessibility, state honesty and task completion separately; no average score
can hide a broken action or inaccessible control. A screen does not pass on a
screenshot alone. Record keyboard and pointer journeys, durable outcomes,
reconnect/error recovery and measured responsiveness on the shipped renderer.

Proposed local interaction budget: visible acknowledgement within 100 ms at
p95 on the declared device; slower network/model work displays honest progress
without inventing completion. Measure before enforcing; remote inference time
is not conflated with UI latency. Retain the World program's explicit native
60fps gate. Blur/motion must not introduce sustained frame or idle-work regressions.

Implementation is organized in `UI_POLISH_MASTER_PROGRAM_DAG.yaml` and
`UI_POLISH_SUBDAGS.md`. The exhaustive source inventory remains a separate
coverage ledger so new or hidden surfaces cannot disappear inside a broad node.

## Planning verification receipt — September 6 continuation

The existing `permagent-eval program validate` command accepted both the
20-node suite-polish manifest and its six-program portfolio parent, exit 0.
Research is passed; inventory remains active with explicit unresolved coverage.
Root independently executed `node --test
docs/orchestrator/ui-polish-surface-inventory.test.mjs`: 4 tests passed, zero
failed. These cover stable IDs/owners, explicit tool/settings route mappings,
fail-closed source parsing, and a synthetic newly unmapped route. They do not
claim visual, native, or exhaustive backend-capability acceptance. No production
UI was changed for this research/planning task.

## Concrete shared-system audit finding

Root measured the current opaque dark-theme `textDim` token (`#5A6478`):
3.15:1 against `bg` (`#0B1220`) and 2.60:1 against `surface` (`#1E2433`).
The existing `StateBlock` uses that color for small explanatory text. These
measurements use sRGB relative luminance, not an installed screenshot. They
identify a U2 legibility candidate: nonessential/disabled decoration must be
distinguished from explanatory text that users need to read. Existing
`textMuted` measures 6.12:1 and 5.07:1 on those same backgrounds. Review actual
use sites and translucent composites before changing a global token; add
semantic text-on-surface regressions rather than merely brightening everything.
The reference for ordinary text is [WCAG contrast minimum](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html).
