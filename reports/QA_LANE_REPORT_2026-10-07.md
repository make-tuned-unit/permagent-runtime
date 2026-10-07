# QA lane report — wq/qa, 2026-10-07 (base 70a9f658)

Facts only. Frontend checks only; no cargo run, no daemon touched.

## Commits (git log 70a9f658..HEAD)
- f6a21769 Q18 History crash
- 590d6752 Q7/Q8/Q10/Q11 Home
- a3cce82a Q8 (calendar import once) / Q12 People graph states
- ad745fc4 Q1 unpaired browser
- 9a5dbea8 Q2 pairing host, Q24 dead pairing link
- 1038ad4d Q3/Q17 chat pill
- 97172a41 Q16 nav click behind Record modal
- 7e843ae9 Q9 Learn-next copy
- ce78f333 Q6 Browser listeners
- c615b99d Q13 Build run titles
- 5b20415a Q14 clipped Grow text
- 28562381 Q19 Grow results timeout
- cba23360 Q20 Kanban skeleton + overflow edge
- 492df5ae Q15 Brain list clearance
- b20739fb Q4 Settings keyboard scroll
- d911bd61 harness removed; four stale pins/tests fixed
- (last) Q21/Q23

## Findings and root causes
- Q18: daemon serialises SessionInsights camelCase (`totalSessions`, `totalTokens`: crates/goose/src/session/session_manager.rs); UI read snake_case so `.toLocaleString()` hit undefined. `api.getSessionInsights` now accepts both and never returns undefined; SessionsList also guards.
- Q10: reproduced in a real-browser harness (Chrome, 1244x879): the weather card (compact manifest) was capped at its configured h:2 (136px) while its content is 151px, so the forecast row was clipped. Compact manifest cards now grow to content (no maxHeight cap, span cap lifted). Also `settleSpan` no longer freezes a card at a too-short span (growth-only after freeze), the most plausible cause of the card painting over its neighbour. The Focus-over-weather overlap itself was NOT reproduced; Q11 (Review clipped) NOT reproduced.
- Q8: PUT on load came from per-browser localStorage "pass done" flags (a fresh browser re-ran the cosmetic compaction and wrote). Only structural changes (retired card stripped, missing card added) are written on load now; cosmetic ceilings apply in memory and are marked done only when the user saves. Calendar import: module-level dedupe within 2 s (the double POST is React StrictMode in dev).
- Q12: 31 mock people render correctly in the harness (screenshot reports/qa-evidence/q12-before.png), so the live cause is not found. Added: empty-directory state, WebGL-context-lost message, ErrorBoundary around the Canvas, clearer load-error copy. Live cause likely GL context loss or a live-data shape; unverified.
- Q1/Q24: BootScreen and App now detect a 401 with a published token error and show "This browser isn't paired" / "This pairing link is invalid or has expired" with the Settings -> Devices instruction, no 10 s of retries. A claim failure already kept any stored token (api.ts exchangeClaim), so a paired browser is unaffected.
- Q2: placeholder host removed; no tailnet -> link uses getApiBaseUrl()/page origin/127.0.0.1:3001 with a hint that it only works locally. /api/devices/pair returns only code+expiry, so the address is derived client-side.
- Q3/Q17: existing scroll-clearance already pads scroll ends; the overlap is mid-scroll. Pill is now icon-only (tooltip + aria-label) below 1360 px width; published size follows via the existing ResizeObserver. Design call: a docked pill would be the alternative.
- Q16: the Record picker's scrim covered the rail, so a nav click only dismissed it. Scrim click now closes and forwards the click to a rail control under the pointer. Stale tooltip not addressed.
- Q14: the daemon cuts evidence/recommendation at 400/600 chars (routes/growth_actions.rs:747-748). UI trims a capped, unfinished string to a whole word with an ellipsis. Real fix is daemon-side.
- Q19: request could hang; now 20 s abort -> error state with retry, "still loading" copy at 6 s. Why the live route is slow was not investigated.
- Q22, Q25: not reproduced, no change (see walk table).

## Pre-existing failures on base (coordinator ask)
- spaceScale: voiceReview.tsx (World file) used raw 12px; now space.xl. Cause: World lane added raw spacing.
- rawButton: World forum buttons (ForumAuthNotice 1, HankConversation 5, voiceReview 2) recorded as counted STANDING_EXCEPTIONS (World lane should migrate and delete them); AutomateView's recover button converted to Button.
- terminalChrome: hash re-frozen; cause is commit 5c4a8ee1 (2026-09-27, Grow-completion notice in the mount effect) which never moved the pin.
- worldLabelStacking: TEST-ONLY change. `.forum-desk-toggle` renders only in campus layout (ForumView.tsx:509 `!compact`), and the default habitat view omits it, so the test now requires it only when present. No CSS or World logic changed. Files touched under components/world/forum: voiceReview.tsx (spacing tokens, dev fixture page) and worldLabelStacking.test.tsx.

## Evidence (reports/qa-evidence/)
- q10-before-1244.png / q10-after-1244.png / q10-after-1440.png: Home with mocked data in real Chrome; weather cell 136px -> 151px, forecast row fully visible.
- q12-before.png: People graph with 31 mock people renders.
- q17-pill-1244.png (icon-only), q17-pill-1440.png (labelled; name blank because the identity call is mocked out).
- Harness (qa-harness.html, src/qa-harness.tsx) deleted in d911bd61.

## Checks
- `npx tsc --noEmit`: exit 0 on the final tree (a transient exit 2 earlier was the harness' unused React import, removed with the harness).
- Focused vitest per area (dashboard, people, App/startup, settings, chat/ChatLauncher, voice, browser, build, grow, projects, brain, automate, splash, store): all passed at the time of each commit.
- Full `npx vitest run`: see final line below.

## Unverified / left out
- No real-browser or native verification for Q1, Q2, Q4, Q15, Q16, Q19, Q20, Q21, Q23 (unit/type checks only). Q15 and Q17 listed for screenshots: Q17 pill captured alone, not over Settings/Projects/Finance; Q15 not captured.
- Q11, Q12 live cause, Q22, Q25 not reproduced.
- New unit tests added: LearnNext.summary.test.ts, App.startup unpaired case, settleSpan growth case. None for Q16/Q19/Q20/Q15.
- Daemon changes needed: Q14 truncation point; Q19 slow route (unexamined).

## Final full run
`npx vitest run`: 458 files, 3574 tests passed, exit 0 (started 19:47:21, before the Q21/Q23 commit; those two changes were covered by the focused automate/store run, 154 passed, but not by this full run). `npx tsc --noEmit` exit 0 after Q21/Q23.
