# iOS design pass

Owner: root/Astra, directly requested by operator. Backend workers continue
their existing verification lanes; no design work is delegated.

Direction: quiet, luminous, distinctly Permagent. Existing cyan/indigo/violet
brand, SwiftUI, bundled fonts and navigation remain. Glass belongs to floating
controls, not every paragraph or content card. No new UI framework.

Reference: Apple HIG Materials and Adopting Liquid Glass:
https://developer.apple.com/design/human-interface-guidelines/materials
https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass

Sequential graph (each exit gates the next):

1. D0 audit — screenshots + actual theme/chat/voice/model/control code.
   Exit: identify spacing, typography, material, accessibility and truthfulness
   defects, preserve existing actions and model routing.
2. D1 foundation — shared materials, legible dynamic typography, 44pt controls,
   reduced-transparency/motion behavior. Exit: regression policy tests and build.
3. D2 conversation — composer, greeting, navigation and voice hierarchy; full
   orb, bottom live captions, readable assistant transcript. Exit: source wiring,
   short/tall viewport checks, no lost status/prompts or controls.
4. D3 supporting surfaces — provider/model sheet and shared cards, home/control
   hierarchy. Exit: every existing action retained, correct loading/error state.
5. D4 visual verification — fresh app build, isolated clearly labeled debug
   fixtures if needed, light/dark and accessibility review. No fixture is a live
   hub or audible-playback receipt.
6. D5 device delivery — signed build and installation, actual routing/voice
   acceptance against compatible daemon. Exit: operator-visible result and
   retained rollback; no source-only completion claim.

Initial audit: heavy opaque cards; 34–38pt chat controls; fixed-size fonts ignore
Dynamic Type; glass and content roles mixed; model IDs dominate the sheet;
unbounded manual spacing; voice errors concealed behind reply content (repaired
in preceding lane); model errors falsely blamed credentials (repaired).

Status: D0–D3 implemented; D4 simulator review/regression passed for the recorded
surfaces below; D5 Release artifact is installed on the paired phone, but live
acceptance remains pending an unlocked device and compatible desktop daemon.
This is not a claim that all product DAGs passed.

## Verification receipt

- Root implemented the design directly; workers did not perform this pass.
- Shared font ramp now honors Dynamic Type; opaque chrome fallback covers
  Reduce Transparency and Increase Contrast. Native iOS26 glass is used for
  controls/composer. Brand colors, bundled fonts and existing navigation remain.
- Shared content cards/backdrops carry into Home, Control, Notes, Agents,
  Features, Pronunciation, Schedules, To-dos, Dictation and Meetings. These
  shared updates do not imply every populated detail screen was screenshot-tested.
- Voice keeps its 300pt orb. The reading area can scroll on constrained screens;
  captions remain above controls and become scrollable at accessibility sizes.
  Voice transcript font and highlighted font scale together. Visible errors,
  teaching and enrollment prompts are preserved independently of reply text.
- Actual app + embedded Watch simulator build passed. Signed Debug physical
  device build passed using existing automatic-signing team; no provisioning
  changes. Existing camera/Watch concurrency and Bluetooth deprecation warnings
  remain, rather than being suppressed in this design pass.
- Final regression bundle:
  `/private/tmp/permagent-ios-tests-20260905-layout.xcresult`:
  **150 passed, 0 failed, 0 skipped** (xcresulttool verified on iPhone 17 Pro,
  iOS 26.5).
- Signed Release device build passed with
  `xcodebuild -project ios/PermagentMobile/PermagentMobile.xcodeproj -scheme
  Permagent -configuration Release -destination 'generic/platform=iOS'
  -derivedDataPath /private/tmp/permagent-ios-device-build-20260905 build`.
  The resulting `Permagent.app` includes the embedded Watch app and was signed
  by the existing automatic-signing team; no provisioning changes were made.
- Isolated DEBUG-only visual mode renders actual SwiftUI screens, blocks saved
  pairing/bootstrap and interactions, and labels sample data. Screenshots are
  not evidence of live provider calls or audible playback.
- Reviewed dark chat/voice, light model/control, and largest Dynamic Type voice.
  Large-text review reproduced truncation and oversized glyphs; these were
  corrected. The full-size orb may require scrolling at maximum accessibility
  sizes; it is never shrunk to fit. Missing voice-identity SF Symbol was found
  visually and replaced with a symbol-availability regression.
- Screenshots: `/private/tmp/permagent-design-chat-dark.png`,
  `/private/tmp/permagent-design-voice-dark.png`,
  `/private/tmp/permagent-design-models-light.png`,
  `/private/tmp/permagent-design-control-light.png`,
  `/private/tmp/permagent-design-voice-large-v2.png`.
  Earlier screenshots precede the last icon/background refinements.
- Phone install completed at 16:43:47 local time via `devicectl`, bundle
  `ai.permagent.ios`; the existing app was updated in place (no uninstall,
  erase, or user-data reset). A launch request was refused by CoreDevice because
  the phone was locked (`FBSOpenApplicationServiceErrorDomain`, reason
  `Locked`). No unlock, polling, or daemon deployment was performed. The app is
  installed for the operator to open normally after unlocking the phone.
- Model-switch HTTP404 on the old installed hub remains a daemon deployment
  dependency; new error handling explains the mismatch, not a bad API key.
