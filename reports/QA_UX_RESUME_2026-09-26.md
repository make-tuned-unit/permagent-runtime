# QA and UX overhaul recovery — 2026-09-26

Recovered the interrupted “Permagent app QA and UX overhaul” session into branch `codex/qa-ux-resume` at `/private/tmp/permagent-qa-ux-resume`, based on Claude's integrated Round 2 commit `3530da4e`. This is a reviewed continuation of that thread, not the separate “QA walk resume” runtime work. Original worktrees and the main checkout were preserved.

## Recovered scope

The seven interrupted lanes were snapshotted and committed separately: pressable controls, owner identity, memory source/forget, Grow deltas and publish failure, relationship strength, worker plan/Librarian requests, and prompt-cache minimums. The last lane's proposed optimization was rejected during review; its recovery commit is followed by a correction.

Source thread: `/Users/j/.claude/projects/-Users-j-Documents-dev-permagent-runtime/24ef18a6-bc9b-4968-aa92-0cf152466a4d.jsonl`. Source worktrees: `/Users/j/Documents/dev/permagent-worktrees/dagK-*`. Patches, test logs, browser harness, and screenshots: `/private/tmp/qa-ux-recovery`.

## Review corrections and completed work

- Owner updates preserve omitted fields; explicit null clears a field. Linking a person no longer erases name/email. Failed saves preserve edits and remain retryable.
- Browser tabs, bookmarks, saved tab sets, and terminal tabs use separate native selection and close/delete buttons, avoiding nested interactive elements. Enter and Space work independently.
- Memory forget uses one absolute Undo deadline. Hover cannot extend an already committed delete; dismissing commits once. Partial/unverified deletion reports failure. Successful graph deletion refreshes data. Source labels include owner, relationship, and user answers.
- Relationship signals now reach the directory, graph sizing, filtering/sorting, and person detail. Persisted previous buckets prevent repeated fading alerts and detect transitions after gaps between visits. Missing source reads produce unavailable signals rather than invented inactivity. The owner is excluded from relationship reminders.
- Worker-plan capture stops at the first tool call, preventing later narration from being labeled the initial plan. Completed-goal evidence carries the plan.
- Librarian requests now have list/answer endpoints and an inline Settings UI with loading, empty, error, retry, saving, and success states. Answers are stored privately with provenance; person context uses a separate manual field. The ledger respects `PERMAGENT_PATH_ROOT`, uses a shared lock with the sweep, and removes requests only after successful persistence.
- Grow retains the recovered previous-period comparisons and publish-failed retry flow. Failed publish results no longer appear successful just because the server returned HTTP 200.
- Prompt caching retains the established rolling breakpoints. Minimum cacheable length applies to the entire prefix, not independently to each incremental chunk. Added a regression for short turns following a large stable prefix. Reference: https://platform.claude.com/docs/en/build-with-claude/prompt-caching . No claim is made that the reported flat-cache behavior or cost is fixed.

## Verification

- Full UI suite: **442 files, 3,419 tests passed** (`ui-final.log`).
- Production UI build: **passed** (`ui-build.log`), including TypeScript. Vite reports large bundle chunks; no build error.
- Rust: `cargo clippy -p permagent -p permagent-daemon --lib --bins --tests -- -D warnings`, through the repository build guard and an isolated target directory. See `rust-gate.log` for final result. Earlier `cargo check --tests` also passed.
- Browser: actual components with mocked API responses at **1100, 1440, and 2560 px** for owner settings, People, memory list, Librarian requests, and browser controls. All 15 checks had no document overflow or page errors. Tab selection with Enter and closing with Space passed; nested button count was zero. Screenshots were visually inspected, including the corrected memory harness layout.
- No paid model requests, live publishes, installed-app replacement, or writes to the user's live Brain data were performed.

## Remaining validation and limitations

Rust test targets were compiled/linted, but Rust tests were not executed. Full native-app integration, real daemon/Brain answer-and-forget flows, and Grow/worker-plan end-to-end runs remain to be exercised in an isolated runtime or CI before release. Browser checks use fixtures and are not proof of those backend flows.

Worker plans are persisted with completion evidence, not streamed live while the worker runs. Relationship memory attribution still relies on display-name annotations, so ambiguous names can map to multiple people. The Librarian sweep holds the ledger lock across its stage-four work; answering can wait for an active sweep. Answer writes across Brain/manual fields/ledger are not a single transaction; failed writes leave the request available for retry.

This branch is based on Round 2. Reconcile with the current runtime integration branch before merging or installing; do not overwrite the parallel QA/runtime fixes with this older base. Remaining backlog beyond the seven recovered lanes is not claimed complete.
