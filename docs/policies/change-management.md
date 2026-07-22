# Change Management Policy

Status: **living document** · Owner: @make-tuned-unit · Maps to SOC 2 **CC8.1** (change management).

This policy documents how code changes reach `main`. It exists both as an operating
standard and as the audit-ready artifact for the SOC 2 scope defined in
[`docs/design/soc2-scoping.md`](../design/soc2-scoping.md). It is deliberately honest
about which controls are **in force today** versus **planned**, and the prerequisite each
planned control is sequenced behind — enabling a strict gate before its prerequisite is met
trades robustness for fragility (e.g. a required check that flakes would block legitimate
merges).

## In force today

- **All changes land via pull request.** No direct pushes to `main` for feature work; the
  branch's PR is the change record (author, diff, discussion, linked issue).
- **Required status checks on `main`** (branch protection): `build`, `lint`,
  `test (ubuntu-latest)`, `test (macos-15)`, `frontend`, `tauri-shell`. A PR cannot merge
  unless all six pass. This is the primary automated change-control gate.
- **Squash merge** — one reviewable commit per change on `main`; linear, bisectable history.
- **CODEOWNERS** declares ownership of the repo and of security-/compliance-sensitive paths,
  so reviewers are auto-requested once review is enabled.
- **Audit trail** — git history + the PR record provide who/what/when for every change.

## Compensating control for solo operation

The owner account (`@make-tuned-unit`) is currently the sole maintainer, so **required
human review is not enabled** — a solo owner cannot approve their own PR, and requiring it
would block all merges. The interim compensating control is: **required CI gate + full PR
audit trail + CODEOWNERS**. This is a documented, time-bound exception, not an oversight.

## Planned controls (each sequenced behind its prerequisite)

| Control | Prerequisite | Rationale |
|---|---|---|
| **`enforce_admins = true`** (admins cannot bypass required checks) | CI reliably green — i.e. after the test-flake sweep (#858/#859) lands | Enabling a hard gate while known flakes exist would let a flake block a legitimate merge with no override. Robust only once CI is trustworthy. |
| **Required signed commits** on `main` | Commit signing configured for **every** commit path (owner, and any automated/agent commits) | Turning on required signatures before signing is set up rejects all commits and halts merges. Set up signing first, then require. |
| **Required human review** (≥1 approval) + CODEOWNERS review | A **second engineer** (or an external reviewer / a designated reviewer identity) | Impossible and merge-blocking for a solo owner; the compensating control above covers the gap until a second reviewer exists. This is the trigger to flip it on. |
| **Emergency/hotfix path** | The above are in force | Define a documented break-glass procedure (who, how logged) once gates are strict, so incidents don't create untracked bypasses. |

## Review triggers

- **On adding a second engineer:** enable required review (≥1 approval), keep CODEOWNERS.
- **On CI reliability confirmed (flakes fixed):** enable `enforce_admins`.
- **On setting up commit signing:** enable required signed commits.
- **When entering the SOC 2 audit window** (tied to the first paying team customer — see the
  scoping doc): all of the above should be in force for the in-scope service repos/paths.

Revisit this document whenever branch protection changes or the team grows.
