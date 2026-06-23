# Goal Completion & Verification — Design Audit

**Status:** Audit / design (no code in this pass)
**Date:** 2026-06-23
**Scope:** the gap between "a goal is marked complete" and "the work is verifiably done and live."
**Related:** #424 / #451 (dispatch loop), #399 (terminal supervision epic), PR #311 (MERGED — completion checks + local verifier + evidence digest), **PR #454 (OPEN — dispatch evidence + Discuss-with-Henry stale-ref fix)**.

> ⚠️ **Reconcile first.** Two systems already exist on this path and were built by separate sessions:
> - **PR #311 (merged):** the qwen verifier + `completion_checks` + the Evidence panel.
> - **PR #454 (open):** deterministic `dispatch_evidence` captured in the worker's worktree + Discuss-with-Henry rewire.
>
> They **do not talk to each other.** ~70% of the "unified model" below is already built; the headline defect is that the *verifier* (#311) and the *evidence capture* (#454) measure two different trees and write two different metadata keys. Do not rebuild — **unify**.

---

## 1. Current-state completion path (with file:line)

The dispatch model uses **two engines** (`goal_engine.rs`):
- `ExternalCliEngine` — spawns `claude` in an **isolated detached worktree** at `<repo>/../.permagent-goal-worktrees/<run_id>` (`goal_engine.rs:242-277`, `create_goal_worktree`), commits + pushes there. **The user's `root_path` checkout is never touched.**
- `InternalSubagentEngine` — works in-process, in-place (no worktree, no push).

The dogfood used `ExternalCliEngine` (claude → Reckonize). That distinction is the root of Problems 1 & 2.

### The path, in order

1. **Dispatch** (`orchestrator.rs:546` `dispatch_goal`):
   - looks up the project (`orchestrator.rs:619`, `projects::get_project`), reads `project.root_path` (`:623`).
   - records the **diff baseline** `git rev-parse HEAD` in the project repo, stored to card metadata `baseline_commit` (`orchestrator.rs:641-650`, `:793-795`). ✅ baseline is captured correctly, at dispatch time, before the worker runs.
   - creates the worktree off that baseline and spawns the worker (`goal_engine.rs:213`, `:266`).

2. **Worker runs** in the worktree → commits → `git push origin HEAD:main`. Exit 0.

3. **Outcome → completion** (`orchestrator.rs:704+`, the completion tracker):
   - **PR #454 adds here:** on `GoalOutcome::Success(Some(evidence))`, persists deterministic `GoalEvidence` to card metadata key **`dispatch_evidence`** via `cards::set_goal_dispatch_evidence` (`cards.rs:759`). This evidence is collected **inside the worktree** against `baseline..HEAD` (`goal_engine.rs:183` `collect_evidence`) → correct worktree path, commit SHAs, diffstat, push target, worker stdout tail.
   - calls `handle_goal_completion` (`orchestrator.rs:2504`).

4. **`handle_goal_completion`** (`orchestrator.rs:2504+`):
   - moves the goal **InProgress → Review** (this move is **unconditional** — no success criterion gates it).
   - creates an `approve_review` **decision** (`orchestrator.rs:2526+`). PR #454 makes its `detail`/`payload` cite the `dispatch_evidence` brief (`build_review_detail`, `format_dispatch_evidence_brief` `orchestrator.rs:415-447`).
   - fires `GOAL_REVIEW_HOOK` (`orchestrator.rs:2524-2525`).

5. **The verifier** (`GOAL_REVIEW_HOOK` → installed `verification/mod.rs:82-96` `install_review_hook`, at daemon start `state.rs:114` → `run_for_goal` → `run_for_goal_with` `verification/mod.rs:100`):
   - resolves working dir as **`project.root_path`** (`verification/mod.rs:119-125`).
   - runs `completion_checks` in that dir (`:127-158`, `checks::run_checks`).
   - `analyze_diff(working_dir = root_path, baseline_commit, declared_paths)` (`:175-180`) → literal commands `git diff --name-only <baseline>`, `git diff --numstat <baseline>`, `git status --porcelain` (`verification/mod.rs:507-515`), all `current_dir(root_path)`.
   - builds an LLM prompt from that diff + checks, runs qwen (`:200-211`, `verifier::run_verifier`).
   - aggregates a verdict, **clamps to Fail if any check ≠ pass** (`:213-226`, the `checks_clamp`).
   - **one atomic write** to card metadata key **`verification`** (`:228-229`, `cards::set_goal_verification` `cards.rs:730`).
   - **on Pass only:** auto-answers the open `approve_review` decision via Henry-policy Tier-1 (`:244-246` `henry_approve_after_pass`). On Fail/Uncertain: nothing moves, the human decides.

6. **Review surfaces:**
   - **Evidence panel** (`EvidenceDigest.tsx`) reads `metadata.verification.evidence_digest` (`client.ts:104-121`) — i.e. **the qwen verifier's record, computed against `root_path`.**
   - **Discuss-with-Henry** (PR #454) reads `metadata.dispatch_evidence` (`session_events.rs:628+`, `format_dispatch_evidence_full`) — i.e. **the worktree-correct evidence.**

### The two-systems collision (the headline finding)

| | Source PR | Diff computed in | Diff range | Metadata key | Surfaced by |
|---|---|---|---|---|---|
| **qwen verifier** | #311 (merged) | `project.root_path` ❌ | `baseline..worktree+HEAD of root_path` | `verification` | **Evidence panel** |
| **dispatch evidence** | #454 (open) | worker's worktree ✅ | `baseline..HEAD` | `dispatch_evidence` | Decision detail + Discuss-with-Henry |

For an external-CLI goal, `root_path` HEAD **==** `baseline` (the worker never touched it), so the verifier's `baseline..HEAD` is **empty** → "0 files changed" → check-clamp + LLM both say **fail**. Meanwhile `dispatch_evidence` correctly shows "3 files, +1159". **After #454 merges, the same card carries two contradictory evidence blocks, and the Evidence panel still shows the false one.**

---

## 2. Root-cause confirmation

### Problem 1 — verifier false-fail: **CONFIRMED. Wrong directory, not wrong ref.**

The dispatch hypothesis was "diff against a ref that already contains the pushed commit (`origin/main..HEAD`)." The code is subtler and the real cause is cleaner:

- The baseline ref is **correct** (`baseline_commit`, captured pre-dispatch).
- The bug is the **cwd**: `analyze_diff` runs in `project.root_path` (`verification/mod.rs:121-125, 175, 507-515`). The external-CLI worker did all its work in `.permagent-goal-worktrees/<run_id>` and pushed to `origin` — **`root_path`'s HEAD is still `baseline`.** So `git diff <baseline>` in `root_path` is genuinely empty: there is nothing between `baseline` and a HEAD that *is* `baseline`.
- This is **not** entangled with push logic. It is a directory/source-of-truth bug. The fix is to point the verifier at the tree where the work lives — which **PR #454 already records** as `dispatch_evidence.worktree_path`, and whose diff it **already computed correctly** (`collect_evidence`, `goal_engine.rs:183`).

**True-fail vs false-fail disambiguator (the dispatch asked for this):** `dispatch_evidence.commits` is the signal. Empty `commits[]` + clean worktree = the worker genuinely did nothing (**true fail**). Non-empty `commits[]`/`files_changed>0` while the verifier's root_path diff is empty = **false fail from wrong-dir**. PR #454's evidence already provides this disambiguator; the verifier just isn't consuming it.

### Problem 2 — no per-goal "done" criterion: **PARTLY FALSE. The machinery exists; it is unpopulated and ungated.**

`completion_checks` already exist (`verification/checks.rs:25-54`) with exactly the right primitives:
- `command_exit_zero` — run a command, pass iff exit 0 (**this is literally `npm run build` as a gate**).
- `http_assert` — hit a loopback URL, assert response (**close to Problem 3's "curl the live URL"**).
- `file_exists`, `grep_absent`.

They run (`verification/mod.rs:127-158`) and **clamp the verdict to Fail** if any check fails (`:213-226`). So the *mechanism* for "done only if `npm run build` exits 0" is built. What's missing:

1. **Nothing authors `completion_checks` per goal.** The dogfood goal had none → `meta.get("completion_checks") == None` → no build ever ran → it reached Review with broken TypeScript.
2. **No project-level default checks** (every code goal in a repo should inherit "build passes").
3. **The checks run in `root_path`, not the worktree** (same wrong-dir bug as Problem 1) — so even if authored, `npm run build` would test the stale tree, not the worker's changes.
4. **Review is reached regardless** (`handle_goal_completion` moves to Review unconditionally; the verdict only gates *auto-approval*). A failing build doesn't block human-approvability — it just removes the auto-approve. So a human can still approve broken work.

So Problem 2 is **not** "no concept of done exists" — it's "the concept exists but is never populated, runs in the wrong tree, and doesn't hard-gate."

---

## 3. Proposed model — **one unified model, mostly assembly of existing parts**

The four problems collapse into **one coherent model**, and most of it is already on `main` or in PR #454. The model:

> **A goal carries a declared completion criterion. The engine runs the verifier and the criterion *against the tree where the work actually lives*, captures one canonical evidence record, gates Review on it, and the review surfaces read that one record. For content repos, "done" extends to a per-project publish sequence whose final step is a live-URL check.**

### 3a. One canonical evidence record (merge `verification` + `dispatch_evidence`)

Stop computing two diffs. The verifier should **consume `dispatch_evidence`** (worktree path + baseline + already-correct diffstat) instead of recomputing against `root_path`:
- Point `analyze_diff` and `checks::run_checks` at `dispatch_evidence.worktree_path` when present; fall back to `root_path` for the in-process engine (where in-place is correct).
- The Evidence panel and Discuss-with-Henry then read the **same** ground truth. **RULED (1): ONE record** — the verifier appends its verdict to the `dispatch_evidence` structure; one key, one card. See §6 for the exact shape.

**This single change fixes Problem 1 and the wrong-tree half of Problem 2 at once.**

### 3b. Declared completion criterion per goal + project defaults

**RULED (5): opt-in with per-goal-type defaults.** Goal-type drives the default check **and** auto-approve eligibility:
- **Code goals** default to `command_exit_zero` (the project build cmd).
- **Content goals** default to publish-sequence + `http_assert` live-check (§3d).
- **Prose/other goals with no build** get **no forced check** — never false-fail them. The user can override or clear any default.
- The disambiguator from §2: if `dispatch_evidence.commits` is empty, short-circuit to a **true "no work done"** verdict — never run the LLM over an empty diff and never emit "fail — no evidence" when work exists.
- Defaults are authored at goal creation/decomposition (orchestrator owns it); project-level overrides live in the project `metadata_json` (§3d, ruling 3).

### 3c. Gate Review on the verdict — **SOFT (ruled)**

**RULED (2): soft gate, loud warning.** The verdict is **advisory** — the goal still reaches Review and **approve stays enabled**, with a loud warning when a required check failed ("build is failing — approving ships broken work"). Never block approval on a Fail: the verifier false-failed correct work 4× this session, so hard-gating would block real work. **Revisit hard-gating only after #455 proves the verifier accurate.** Pairs with the informed-reject below. Auto-approve remains **default-OFF** (standing constraint) — a Pass does not auto-advance.

### 3d. Per-project publish sequence (Problem 3 — the only genuinely new build)

Projects are all-typed-columns with **no metadata bag** (`spectral_schema.rs:573-593`; struct `projects.rs:8-24`). #189 added `root_path/site_url/repo_url`. A publish sequence has **no home today → schema change required (v13 → v14).**

**RULED (3): add a general `metadata_json TEXT` column to `projects`** (mirrors `cards.metadata_json`), not a single-purpose column and not a separate table. The publish sequence is one key inside it; project-level check overrides (§3b) are another.

Model: an ordered list of post-commit/post-push commands, e.g.
```json
{
  "publish_sequence": [
    {"order":1,"command":"set -a; source .env.local; set +a; npx tsx scripts/reseed-threads.ts","timeout_secs":300},
    {"order":2,"command":"vercel --prod","timeout_secs":600}
  ]
}
```
**RULED (4): commands run in the worker's worktree context; secrets from the project's `.env.local` loaded at execution time.** Config stores the **command** (referencing `.env.local` by path) — **never the secret value**. Each step's exit code + **redacted** tail is recorded into the one evidence record. **The redaction guard (ruling 4) is mandatory and must cover both worker_summary and publish-step stdout** — see §6 for the design and the non-triviality flag.

**RULED (6): the final "live" criterion is `http_assert` asserting specific expected content** — slug **and** connection count on the right surface — **not just HTTP 200** (200-alone was proven insufficient this session). This is a `completion_check` type that already exists (`checks.rs`).

### 3e. Informed reject when already-pushed (Problem 4)

The decision/Evidence panel should **read `dispatch_evidence.push_target`** and, when non-null, warn on reject: "this commit is already on `origin/main` — rejecting does not un-ship it; you'll need a revert." Pure UI/decision-detail logic over data #454 already captures.

### Do the four collapse into one model? — **Yes.**

1 and 2 are the same machine (verifier = completion check) sharing one bug (wrong tree) — **one fix**. 3 extends the criterion to "live" via an existing check type + one schema field. 4 is the *output*: once 1–3 write one trustworthy evidence record, every surface reads it. The only thing that is genuinely *separate work* (not assembly) is 3d's schema + publish-runner.

---

## 4. Recommended build sequence (opinionated)

**Land PR #454 first** — it's the substrate (correct worktree evidence + Discuss-with-Henry). Everything below builds on it.

1. **🥇 Unify the verifier onto `dispatch_evidence` (fixes Problem 1).** *Quick win, highest value.* Point `analyze_diff`/`run_checks` at `dispatch_evidence.worktree_path`; short-circuit true-no-work when `commits[]` empty. This is the most dangerous bug (false-fail trains distrust) and is **small** — a cwd source + an early-return, no schema, no push-logic entanglement. Do this immediately after #454.
   - *Unblocks:* a trustworthy Evidence panel; makes every downstream check meaningful.

2. **🥈 Soft-gate Review + informed-reject (#458, Problem 4 + part of 2).** *Small.* Loud advisory warning on a Fail (approve stays enabled, ruling 2) + the already-pushed warning (§3e). No schema. Makes the review surface trustworthy.

3. **🥉 Author `completion_checks` per goal-type (#456, Problem 2).** *Medium.* Orchestrator seeds goal-type defaults at creation (code → build; content → publish+live; prose → none, ruling 5); user-overridable. Depends on #1 (checks must run in the worktree to be valid).

4. **Per-project publish sequence + live-check (#457, Problem 3).** *Largest — only true net-new.* Schema v14 (general `metadata_json` on `projects`, ruling 3), a publish-runner in the orchestrator post-push running in the worktree with `.env.local` (ruling 4), `http_assert` content-specific live-check (ruling 6), project-config UI, and the publish-step half of the redaction guard. Depends on #1 (evidence record) and #3 (check authoring). Do last; only piece needing a migration + new surface.

**Rationale for ordering:** #1 is the keystone — it is small, it is the most dangerous bug, and it is the precondition for #2/#3/#4 to operate on real data. #2 converts the now-correct signal into an enforced gate. #3 populates the criterion. #4 is the heavyweight that turns "pushed" into "live," and is the only one carrying schema + UI cost, so it goes last.

---

## 5. Decisions — RULED (Jesse, 2026-06-23)

1. **Unification shape — ONE record.** The verifier *consumes* `dispatch_evidence` and **appends its verdict to the same structure**. One metadata key, one card. (Supersedes the "keep two keys" recommendation in §3a — both surfaces and the verifier converge on a single record.)
2. **Review gate — SOFT, loud warning.** The verifier verdict is **advisory**; never block approval on a Fail (the verifier false-failed correct work 4× this session). Approve stays enabled with a loud warning. **Revisit hard-gating only after #455 proves the verifier accurate.**
3. **Publish-sequence storage — `metadata_json TEXT` column on `projects`** (schema **v14**), not a separate table and not a single-purpose `publish_sequence_json` column. (Broader than §3d's original Option A: a general project metadata bag, mirroring `cards.metadata_json`; the publish sequence is one key inside it.)
4. **Publish commands run in the worker's worktree context; secrets from the project's `.env.local` loaded at execution time.** Config stores the **command** (which references `.env.local` by path), **never the secret value**. **CRITICAL — redaction guard:** the #454 evidence-capture layer must **exclude/redact `DATABASE_URL` / connection-string / token lines from persisted worker stdout** (`dispatch_evidence.worker_summary`) and from any publish-step output. The publish feature must not leak prod credentials into the goal card. *(See §3d for the redaction design + the non-triviality flag.)*
5. **completion_checks — opt-in with per-goal-type defaults.** Code goals default to `command_exit_zero` (build); content goals default to publish-sequence + `http_assert` live-check; the user can override or clear. **No forced check on goals with no build** (don't false-fail prose goals). **Goal-type drives both the default check and auto-approve eligibility.**
6. **Live-check — `http_assert` is sufficient, but assert specific expected content** (slug + connection count on the right surface), **not just HTTP 200** (200-alone was proven insufficient this session).

**Standing constraint:** auto-approve stays **default-OFF throughout** (per the earlier redirect). The soft gate (ruling 2) means even a Pass verdict does not auto-advance unless auto-approve is explicitly enabled.

### Build sequence (confirmed): land #454 → **#455** (keystone) → **#458** → **#456** → **#457**.

---

## 6. Build plan — #455 (keystone), pending confirmation before code

**Goal:** the verifier reasons over the tree where the work actually lives, and emits ONE evidence record (ruling 1). No schema, no push-logic change.

1. **Source the worktree from `dispatch_evidence`.** In `run_for_goal_with` (`verification/mod.rs:119-125`), when the card has `metadata.dispatch_evidence.worktree_path` (external-CLI goals), use it as `working_dir` for both `analyze_diff` (`:175`) and `checks::run_checks` (`:133`). Fall back to `project.root_path` only when absent (in-process subagent — in-place is correct there).
2. **Reuse the already-correct diff.** `dispatch_evidence` already carries `baseline`, `commits[]`, `files_changed/insertions/deletions`, `diffstat` computed in the worktree (`goal_engine.rs:183` `collect_evidence`). Prefer feeding that to the verifier prompt over re-shelling `git diff` against the wrong tree.
3. **True-fail vs false-fail short-circuit — key on *work-detection*, not commit count (ruling, item-2 nuance).** Emit the deterministic **"no work produced"** verdict only when `dispatch_evidence` is present AND there are no commits AND the worktree has no uncommitted changes (clean `git status`). Rationale: the in-process `InternalSubagentEngine` works in-place and does *not* commit — but it returns `Success(None)` (no `dispatch_evidence`), so it already falls through to the `root_path` diff which captures its uncommitted changes; it is never subject to this short-circuit. External-CLI always commits, so in practice commits-empty ≈ no-work — but the clean-worktree guard makes that a conscious check rather than an assumption, so a goal type that legitimately leaves uncommitted worktree changes is never false-failed. Never run the LLM over an empty diff and never print "fail — no evidence" when work exists.
4. **ONE record (ruling 1).** The verifier appends its verdict fields onto the `dispatch_evidence` structure (one key) rather than writing a separate `verification` key. Migrate the Evidence panel read path (`client.ts:104-121`) and Discuss-with-Henry (`session_events.rs:628+`) to the unified key. *(Open sub-question for confirmation: keep the existing key name `verification` and fold `dispatch_evidence` into it, or keep `dispatch_evidence` and append a `verdict` sub-object? Recommend the latter — #454's writer/readers stay, the verifier just adds a `verdict` field.)*
5. **Auto-approve stays OFF (standing constraint).** Leave `henry_approve_after_pass` (`verification/mod.rs:244-246`) gated behind the existing L3/auto-approve policy, which remains default-off; #455 does not enable it.

**Confirmed (Jesse, 2026-06-23):** (a) append-`verdict` shape — verifier appends its verdict sub-object onto `dispatch_evidence`; keep #454's key/writer/readers. (b) worker_summary redaction lands in #455; publish-step redaction defers to #457 and prefers *capture-less* there (exit code + redacted tail + live-check result, not raw tool stdout). Heuristic-not-provable accepted as the honest ceiling.

**⛔ HOLD (Jesse, 2026-06-23): do not start #455 until #454 is dogfooded + merged to main.** #455 branches off #454 and rebases onto main once it lands. Plan is locked; awaiting the go signal.

**Redaction (ruling 4) — flagged as non-trivial; landing the guard *with* #455 since #455 is the first PR to make the captured stdout authoritative.** The only persisted free-text vectors are `dispatch_evidence.worker_summary` (`tail(stdout,4000)`, `goal_engine.rs:339`/`collect_evidence`) and, later, publish-step stdout (#457). A shared `redact_secrets(&str)` applied at capture time covers both. **It cannot be proven complete** (heuristic), so the safe design is *capture-less-and-redact*: line-wise drop/`****` for `scheme://user:pass@host` connection strings (`postgres`/`postgresql`/`mysql`/`mongodb(+srv)`/`redis`), `*_URL=`/`*_KEY=`/`*_TOKEN=`/`*_SECRET=`/`PASSWORD=` env assignments, and known token shapes (`sk-…`, JWT `eyJ…`, AWS `AKIA…`). For #457 publish steps, prefer persisting **exit code + redacted tail + the live-check result only**, not raw tool stdout. **Decision needed:** land redaction in #455 (recommended — worker_summary already leaks today) or defer the publish-step portion to #457.

---

## Appendix — key file:line anchors

## Appendix — key file:line anchors

- Engines / worktree: `goal_engine.rs:242-277` (`create_goal_worktree`), `:183` (`collect_evidence`), `GoalEvidence` struct `:91-118` (PR #454).
- Dispatch + baseline: `orchestrator.rs:546` (`dispatch_goal`), `:619-623` (project/root_path), `:641-650`/`:793-795` (baseline_commit).
- Completion + decision: `orchestrator.rs:704+` (tracker, #454 evidence persist), `:2504+` (`handle_goal_completion`, unconditional Review move), `:2524-2525` (hook fire), `:415-502` (#454 evidence formatters).
- Verifier: `verification/mod.rs:82-96` (install), `:100` (`run_for_goal_with`), `:119-125` (working_dir = root_path ❌), `:127-158` (checks), `:175-180`+`:507-515` (`analyze_diff` literal git commands), `:213-226` (check clamp), `:228-229` (write `verification`), `:244-246` (auto-approve on Pass).
- Checks: `verification/checks.rs:25-54` (`command_exit_zero`, `http_assert`, `file_exists`, `grep_absent`).
- Metadata seams: `cards.rs:730` (`set_goal_verification`), `cards.rs:759` (`set_goal_dispatch_evidence`, #454).
- Surfaces: `EvidenceDigest.tsx`, `decisions/client.ts:104-121` (reads `verification.evidence_digest`), `session_events.rs:628+` (Discuss-with-Henry reads `dispatch_evidence`, #454).
- Projects schema: `spectral_schema.rs:573-593` (table, no metadata_json), `:35` (`SPECTRAL_SCHEMA_VERSION = 13`); struct `projects.rs:8-24`.
