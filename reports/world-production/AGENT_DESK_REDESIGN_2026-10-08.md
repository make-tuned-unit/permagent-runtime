# Agent Desk continuation — October 8, 2026

This continues checkpoint `3a2a06bd` in `world/production-20261006`, responding to the user's narrow Agent Desk screenshot. Source changes are available in the existing 5301 preview. Installed Permagent and the retained native r3 package were not replaced.

## Design and browser acceptance

The compact desk now groups a 112×136 portrait, one name/role and reported status with Chat, Sit together and Write a brief. An actual role-specific suggested question replaces introductory prose. Working style, optional identity, work, tools and World settings are disclosures. The legacy campus retains its separate layout.

At 776×730, opening the desk reduces the scene to 207px and gives the desk 483px, placing primary actions within the viewport. At 776×1100 the scene is capped at 280px. Closing at 776×730 restores the scene to 690px. Desktop keeps the side-by-side layout. Temporary viewport overrides were reset after inspection.

Root Computer Use refound the UI after each transition and verified:

- Henry and Librarian profiles, role-specific question, reported status, and Henry-only identity/seating controls.
- Write a brief opens the editor; suggested question transfers the exact text to an editable Query without dispatch.
- Working-style disclosure opens/collapses; optional identity loads on expansion and truthfully reports no linked identity.
- Desk close/reopen and narrow-scene restoration.
- Sit together enters the one-to-one table; Leave table returns to World. Voice was not activated.
- All four Learn lessons render their own content; Use this question transfers the selected lesson question into Brief.
- All three Evidence demonstrations: planning answer feedback, newer-record provenance feedback, and shade → prediction → inspect result. These remain explicitly fictional and do not dispatch work.
- Profile Chat opens the shared connected conversation and dock; close returns cleanly. Existing history was inspected; no message or inference was sent.

Screenshots: [desktop](agent-desk-redesign-desktop-20261008.png), [narrow](agent-desk-redesign-narrow-20261008.png). The normal preview uses daemon 3001; only navigation, reversible view state and illustrative examples were exercised.

Worker validation: 43 focused World/profile tests and TypeScript pass. Source remains frozen after CUA. The app navigation self-knowledge now describes the desk's actual controls and shared-chat/draft boundaries.

## Other lanes

Brain preview resolution now reads a saved memory by stable key, validates returned identity, cancels stale requests and displays actual source/full text. A saved memory outside the loaded graph is explicitly labeled; failed lookups retain a labeled preview. Worker validation: 42 focused tests and TypeScript pass. This is source/test evidence, not yet the isolated CUA acceptance of the new lookup. The [isolated UI batch](ISOLATED_PROJECT_UI_BATCH_2026-10-08.md) preserves the next concrete project/card/note checks.

Record adds default-off scoped PCM diagnostics and a read-only CoreAudio HAL probe source. Seven Python/source protocol checks pass. Swift changes and the HAL probe are **not compiled or captured**. Previous zero-PCM failure remains unresolved; no end-to-end Record pass is claimed. Exact commands and interpretation are in [scoped capture recipe](../record-evidence/SCOPED_CAPTURE_RECIPE.md).

The isolated daemon's one-hour session expired. Restart correctly refused below its 25GiB startup guard; free disk was approximately 24GiB, then 23GiB. No guard was weakened, unrelated target cleaned or production daemon restarted. No heavy build or new capture was attempted. The native r3 and final daemon remain the previous checkpoint binaries, predating this continuation's source changes.

Next: isolated Brain/project CUA when the runtime can start under its guard; compile and exercise scoped audio diagnostics; finish the remaining flow matrix/native acceptance. This report does not close overall app or Record acceptance.
