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
- The Evidence panel and Discuss-with-Henry then read the **same** ground truth. (Decide: fold `dispatch_evidence` fields into the `verification` record, or have the verifier read `dispatch_evidence` as input and keep one output key. Recommend the latter — smaller change, #454's writer stays.)

**This single change fixes Problem 1 and the wrong-tree half of Problem 2 at once.**

### 3b. Declared completion criterion per goal + project defaults

- Author `completion_checks` when a goal is created/decomposed (the orchestrator already owns goal-card creation). Minimal v1: a code goal gets `[{command_exit_zero: "<project build cmd>"}]`.
- **Project-level default checks** so authors don't repeat themselves. This needs a home on the project (see 3d / Problem 3 — same schema change).
- The disambiguator from §2: if `dispatch_evidence.commits` is empty, short-circuit to a **true "no work done"** verdict — never run the LLM over an empty diff and never emit "fail — no evidence" when work exists.

### 3c. Gate Review on the verdict (informed, not silent)

Today the verdict only gates auto-approval. Options (needs a Jesse ruling):
- **Hard gate:** a failed required check (e.g. build) sends the goal to **InProgress (rework)** or **Parked**, not Review — it never becomes human-approvable.
- **Soft gate (recommended v1):** goal still reaches Review, but the decision is **marked blocked**: approve is disabled / warns loudly ("build is failing — approving ships broken work"). Pairs with the informed-reject below.

### 3d. Per-project publish sequence (Problem 3 — the only genuinely new build)

Projects are all-typed-columns with **no `metadata_json`** (`spectral_schema.rs:573-593`; struct `projects.rs:8-24`). #189 added `root_path/site_url/repo_url`. A publish sequence has **no home today → schema change required (v13 → v14).**

Model: an ordered list of post-commit/post-push commands stored on the project, e.g.
```json
[
  {"order":1,"command":"set -a; source .env.local; set +a; npx tsx scripts/reseed-threads.ts","cwd":".","timeout_secs":300},
  {"order":2,"command":"vercel --prod","cwd":".","timeout_secs":600}
]
```
The orchestrator runs these **after** the worker's commit+push, in the project repo (these touch prod infra, not the worktree), and records each step's exit/stdout into the evidence record. The **final criterion is "live"**: reuse the existing `http_assert` check (`checks.rs`) — `curl site_url` returns 200 and contains the slug. That is the real definition of done for a content goal, and it's a `completion_check` type that already exists.

**Storage decision (needs ruling):** add `publish_sequence_json TEXT DEFAULT '[]'` to `projects` (Option A, mirrors `cards.metadata_json`, minimal migration) vs. a dedicated `project_publish_steps` table (Option B, relational, more overhead). **Recommend Option A.**

### 3e. Informed reject when already-pushed (Problem 4)

The decision/Evidence panel should **read `dispatch_evidence.push_target`** and, when non-null, warn on reject: "this commit is already on `origin/main` — rejecting does not un-ship it; you'll need a revert." Pure UI/decision-detail logic over data #454 already captures.

### Do the four collapse into one model? — **Yes.**

1 and 2 are the same machine (verifier = completion check) sharing one bug (wrong tree) — **one fix**. 3 extends the criterion to "live" via an existing check type + one schema field. 4 is the *output*: once 1–3 write one trustworthy evidence record, every surface reads it. The only thing that is genuinely *separate work* (not assembly) is 3d's schema + publish-runner.

---

## 4. Recommended build sequence (opinionated)

**Land PR #454 first** — it's the substrate (correct worktree evidence + Discuss-with-Henry). Everything below builds on it.

1. **🥇 Unify the verifier onto `dispatch_evidence` (fixes Problem 1).** *Quick win, highest value.* Point `analyze_diff`/`run_checks` at `dispatch_evidence.worktree_path`; short-circuit true-no-work when `commits[]` empty. This is the most dangerous bug (false-fail trains distrust) and is **small** — a cwd source + an early-return, no schema, no push-logic entanglement. Do this immediately after #454.
   - *Unblocks:* a trustworthy Evidence panel; makes every downstream check meaningful.

2. **🥈 Hard-/soft-gate Review on the verdict + informed-reject (Problem 4 + part of 2).** *Small.* Decide gate semantics (§3c) and add the already-pushed warning (§3e). No schema. Makes "complete" mean "verified."

3. **🥉 Author `completion_checks` per goal (Problem 2).** *Medium.* Orchestrator seeds a build check on code-goal creation. Depends on #1 (checks must run in the worktree to be valid). Quick win for the common case (`npm run build`).

4. **Per-project publish sequence + live-check (Problem 3).** *Largest — only true net-new.* Schema v14 (`publish_sequence_json` on `projects`), a publish-runner in the orchestrator post-push, `http_assert` live-check as the final criterion, project-config UI to author the sequence + default checks. Depends on #1 (evidence record) and #3 (check authoring). Do last; it's the only piece needing a migration and a new surface.

**Rationale for ordering:** #1 is the keystone — it is small, it is the most dangerous bug, and it is the precondition for #2/#3/#4 to operate on real data. #2 converts the now-correct signal into an enforced gate. #3 populates the criterion. #4 is the heavyweight that turns "pushed" into "live," and is the only one carrying schema + UI cost, so it goes last.

---

## 5. Open decisions needing a Jesse ruling

1. **Verifier ↔ evidence unification shape:** verifier *reads* `dispatch_evidence` and keeps writing one `verification` key (recommended), **or** fold both into one merged record? (Affects whether the Evidence panel's read path changes.)
2. **Review gate semantics (§3c):** hard gate (failed build → InProgress/Parked, never approvable) vs. soft gate (reaches Review but approve is blocked/warned). Recommend soft for v1.
3. **Publish-sequence storage (§3d):** `publish_sequence_json` column on `projects` (Option A, recommended) vs. dedicated `project_publish_steps` table (Option B). Either way it's **schema v13 → v14** — confirm we're opening a migration.
4. **Where publish-sequence commands run:** in the project `root_path` (they touch prod DB/Vercel, not the worktree) — confirm. And **secrets**: the Reckonize sequence sources `.env.local`; do publish commands run with the repo's own env, and is that acceptable for autonomous dispatch, or does it require a human-approved gate per project?
5. **Default completion checks:** should every code goal auto-inherit a project-default build check, or is it opt-in per goal? (Drives #3 vs #4 scope.)
6. **Live-check authority:** is `http_assert site_url contains slug` sufficient as "done = live," or does static-vs-dynamic page regeneration (the home-listing-needs-`vercel --prod` case) need a per-page assertion list?

---

## Appendix — key file:line anchors

- Engines / worktree: `goal_engine.rs:242-277` (`create_goal_worktree`), `:183` (`collect_evidence`), `GoalEvidence` struct `:91-118` (PR #454).
- Dispatch + baseline: `orchestrator.rs:546` (`dispatch_goal`), `:619-623` (project/root_path), `:641-650`/`:793-795` (baseline_commit).
- Completion + decision: `orchestrator.rs:704+` (tracker, #454 evidence persist), `:2504+` (`handle_goal_completion`, unconditional Review move), `:2524-2525` (hook fire), `:415-502` (#454 evidence formatters).
- Verifier: `verification/mod.rs:82-96` (install), `:100` (`run_for_goal_with`), `:119-125` (working_dir = root_path ❌), `:127-158` (checks), `:175-180`+`:507-515` (`analyze_diff` literal git commands), `:213-226` (check clamp), `:228-229` (write `verification`), `:244-246` (auto-approve on Pass).
- Checks: `verification/checks.rs:25-54` (`command_exit_zero`, `http_assert`, `file_exists`, `grep_absent`).
- Metadata seams: `cards.rs:730` (`set_goal_verification`), `cards.rs:759` (`set_goal_dispatch_evidence`, #454).
- Surfaces: `EvidenceDigest.tsx`, `decisions/client.ts:104-121` (reads `verification.evidence_digest`), `session_events.rs:628+` (Discuss-with-Henry reads `dispatch_evidence`, #454).
- Projects schema: `spectral_schema.rs:573-593` (table, no metadata_json), `:35` (`SPECTRAL_SCHEMA_VERSION = 13`); struct `projects.rs:8-24`.
