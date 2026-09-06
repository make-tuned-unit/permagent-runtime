# Voice + Chat Model Affordance Receipt (V5)

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed for implementation; device interaction remains gated by V6

## Contract delivered

- Chat and Voice use separate `chat` and `voice` scopes and persist through
  `POST /config/model-route`; neither picker mutates the global session route.
- `/config` exposes resolved effective routes so environment, configured,
  measured-default, and explicit voice-session fallbacks do not render blank.
- The chat composer shows an actionable `Model · {model}` capsule.
- The Voice header shows the active model beside `slider.horizontal.3`, replacing
  the hardware-oriented CPU glyph.
- Tapping the agent identity opens the existing typed agent controls/roster.
  Identity and model selection remain distinct actions.
- Changes apply on the next turn; an in-flight turn is not rerouted.

## Verification receipt

Passed:

- independent read-only source/contract review;
- Swift parse of `ModelPickerView.swift`, `Views.swift`, `VoiceView.swift`,
  `AgentsView.swift`, and `VoiceProtocolTypes.swift`;
- full iPhoneOS SDK typecheck reported by the implementation worker before the
  final header-only patch; the final patch separately passes Swift parse;
- `CARGO_INCREMENTAL=0 cargo check -p permagent-daemon --lib`;
- `git diff --check`.

The focused daemon unit-test executable compiled but macOS terminated the
655 MiB monolithic lib-test process with SIGKILL before it ran. This is recorded
as a test-infrastructure/resource defect, not a passing test and not an
assertion failure. V6 must run the scoped endpoint interaction against the
fresh daemon, and the coding-harness program must split or otherwise bound
daemon test binaries so a targeted test cannot link every daemon unit.

## V6 device assertions

1. Tap the chat model capsule and Voice model action; each opens the configured
   provider → model list.
2. Select a route and verify the correct `chat` or `voice` POST body.
3. Confirm the current turn finishes on its original route and the next turn
   uses the new route.
4. Tap the Voice header identity and verify agent controls open—not the model
   picker.
5. Verify Dynamic Type, VoiceOver labels, keyboard avoidance, and 375 pt width.

