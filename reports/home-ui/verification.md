# Verification — Home and Projects baseline

**Historical baseline.** The subsequent page-by-page implementation and current gates are tracked in [the UX DAG verification](../ux-dag/verification.md). Statements below describe the earlier Home/Projects slice, not current delivery status.

Source workspace: `/Users/j/Documents/dev/permagent-runtime/.agent-worktrees/home-ui-20261015`, branch `ui/home-20261015`.
Consolidated source and stack-discovery backend: `/Users/j/Documents/dev/permagent-worktrees/world-production-20261006`, branch `world/production-20261006`.

The canonical branch combines the UI commits with the recovered voice changes and the new stack endpoint. The only UI difference from the tested Home worktree is the recovered voice delivery-tag change; its two tests pass again on the canonical branch (`voice-tags-final.log`).

No native installation, deployment, live configuration write, message, recording, or worker launch was performed. The installed application and daemon remained observe-only. Browser fixtures intercept every fetch and replace WebSocket; their data is entirely synthetic.

## Automated checks

- Recovered Home: TypeScript and production UI build passed. Full suite passed **559 files / 4,589 tests** (`resume-full-suite.log`). This supersedes the inherited partial verification and its resolved sparse-checkout/cache failures.
- Projects and spacing-policy final recheck: **37 files / 207 tests passed** (`final-recheck.log`). Coverage includes bounded task previews with review priority, complete security controls behind the summary, automatic read-only stack requests, source evidence, absent folder, failure/partial states, stale response rejection, lazy collection loading, retained drafts, note deep links and project switching.
- TypeScript after the Projects additions: passed (`projects-typecheck-final.log`).
- Combined UI suite: **560 files passed / one spacing-policy file failed**, with **4,596 tests passed / one failed**, in 422 seconds (`final-full-suite.log`). The failure identified two new chip-padding values outside the shared scale. Both were replaced with spacing tokens; the complete Projects directory plus that policy passed the 207-test final recheck. The connection-request test was added after the full run and passed in that recheck.
- Final production UI build, including TypeScript: **PASS**, exit 0 (`projects-build-final.log`; external build log `20261015-220831-2945.log`). Output remains on the external build volume. Existing chunk-size and mixed dynamic/static import warnings are unchanged.
- Backend detector: **5 tests passed** against the actual source file in an isolated Cargo harness, with serde/config/tempfile dependencies (`20261015-220411-297.log` on the external build volume). Cases cover nested manifests, dependency aliases, malformed/oversized files, missing roots, symlinks, excluded manifest values and a workspace exceeding the scan limit.
- Full daemon integration: the initial daemon test build was stopped cleanly during a cold dependency rebuild, before any daemon test ran. A guarded `cargo check -p permagent-daemon --lib` is being used to check route integration; result pending. The isolated test pass is not a claim that the complete daemon test binary ran.

Vitest caches are directed to `reports/home-ui/.vite` for the final runs. The initial Projects-only run used the default cache and failed solely on a matcher unavailable in Vitest 1.6; that assertion was corrected and all 202 tests passed. Existing React act deprecation, mocked-daemon and Browserslist warnings remain unrelated to these changes.

## Browser inspection

Actual components rendered in local Chrome with synthetic data through `scripts/home-review-server.mjs`. The server has no daemon proxy. This verifies browser layout and interactions, not native WKWebView acceptance.

Home:

- Desktop daily overview; 480px busy state with long titles and large task/conversation lists.
- Silver-theme empty state, failed decision request and retry, exact legacy-default display upgrade.
- Decision Inbox with eight rows, Layouts gallery and Customize/Done entry points. No layout was saved to a live backend.

Projects:

- Desktop Overview with 30 tasks and 12 security findings: five task rows, full totals, two finding previews and total high/critical risk count.
- 480px silver Overview: purpose and security precede current tasks; Watcher and activity follow. No horizontal content overflow observed.
- Security review opens the existing full findings/actions; Escape closes it and returns focus to Review security. No fix, dismiss, verify or rescan was triggered.
- Desktop and 480px Details: detected technologies appear without manual entry; source disclosure works; partial scan is explicit.
- Knowledge opens one collection at a time. A synthetic unsaved note survives navigation to Stack and back. Empty/unconnected, failed discovery, failed task feed and never-scanned security states were inspected. The connection action prepares a chat request without overwriting an existing draft; its behavior is covered by a unit test.

The read-only native inspection covered all primary pages and all 13 Settings panes; source-only surfaces and unresolved observations are marked in `APP_UX_REVIEW_2026-10-15.md`. No screenshots of private records are included in the report or intended source commits.

## Scope and limits

Home and the first Projects changes are implemented. The audit supplies proposed structures for the remaining pages; it does not claim those pages have been redesigned or user-tested. The People graph remains unchanged and is explicitly preserved as the primary People experience.

Stack discovery reads supported manifests from the connected local folder. It does not execute project code, contact a provider, infer login identities, prove deployment health, or overwrite manual service records. Its UI requires the matching daemon endpoint. Full app installation and native acceptance remain separate from source validation.

The Watcher audit identifies repeated unchanged signals and a stale-card query that includes terminal states. The proposed evidence/change/action contract is recorded, but its background generation policy is not changed in this slice.
