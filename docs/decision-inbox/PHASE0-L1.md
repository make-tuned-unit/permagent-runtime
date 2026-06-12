# Decision Inbox — Phase 0, Lane L1 (Daemon Core)

> Committed verbatim by the coordinator on L1's behalf (lane agent was permission-blocked from writing report files). Content authored by Lane L1.

Branch: `inbox/daemon-core` @ 77288cb45 (origin/main). Worktree: `/Users/jessesharratt/dev/permagent-worktrees/di-daemon`. All paths repo-relative; line numbers cite the 77288cb45 tree.

## 0. Environment proof

```
$ git -C /Users/jessesharratt/dev/permagent-worktrees/di-daemon worktree list (excerpt)
/Users/jessesharratt/dev/permagent-runtime                    0522cd2ab [main]
/Users/jessesharratt/dev/permagent-worktrees/di-daemon        77288cb45 [inbox/daemon-core]
/Users/jessesharratt/dev/permagent-worktrees/di-henry         77288cb45 [inbox/henry-loop]
/Users/jessesharratt/dev/permagent-worktrees/di-ui            77288cb45 [inbox/home-card]
/Users/jessesharratt/dev/permagent-worktrees/di-verify        77288cb45 [inbox/verification]

$ git log --oneline -2
77288cb45 fix(ci): stage sherpa-onnx prebuilt libs outside target to survive rust-cache pruning (#263) (#284)
985fde8a3 refactor(brain): SafeBrain newtype enforces spawn_blocking at compile time (#277)
```

(`pwd` itself was blocked by a sandbox permission denial on `cd`-compound commands; `git -C` against the worktree succeeded and proves the environment.)

## 1. Goal state machine map (as-is)

**Where goals live**
- No goals table. A goal is a `cards` row with `card_type='goal'` — DDL `crates/goose/src/session/spectral_schema.rs:642-658` (`CHECK (card_type IN ('standard','goal','social_post'))`, `metadata_json TEXT NOT NULL DEFAULT '{}'`); duplicated in the v7→v8 migration at `spectral_schema.rs:1089`.
- State is positional: `cards.column_id` → `board_columns.state_binding` ∈ {triage, ready, in_progress, review, complete}, seeded per project by `cards::seed_goal_columns` (`crates/goose/src/cards.rs:128`).
- Pure state machine: `crates/goose/src/goal_state.rs` — `GoalState` (13-19), `GoalAction` (60-71), `validate_transition` (120-145). Transitions: Triage→Ready, Ready→InProgress, InProgress→Review, Review→Complete (approve), Review→InProgress (reject). Complete terminal.
- All goal metadata is untyped JSON in `cards.metadata_json`: `attempt_count`, `goal_state`, `needs_human_attention`, `last_error`, `worker_key`, `worker_session_id`, `dispatched_at`, `completed_at`, `depends_on`, `acceptance_criteria`, `tags`, `review_notes` — written at `orchestrator.rs:588-613`, `1204-1284`, `2246-2339`.

**Advance actions / where approve-reject lands**
- `MAX_GOAL_ATTEMPTS: u64 = 3` — `crates/goose/src/agents/platform_extensions/orchestrator.rs:30`.
- `goal_advance` MCP tool → `handle_goal_advance` (`orchestrator.rs:1144-1320`). Reject (1207-1269): at `attempt_count + 1 >= 3` sets `needs_human_attention=true` + `last_error` and moves to Triage (1220-1262); otherwise attempt++ and bounce to InProgress. Approve (1272-1279): stores `review_notes`, then cascades `dispatch_eligible_goals` (1311-1314). **This is the only approve/reject today, and it is an LLM tool call — no recorded human decision artifact exists.**
- `dispatch_goal` (`orchestrator.rs:426-643`): Ready precondition (450-459), worker select (462), subagent spawn (544-562) + completion tracker (568-585), attempt++ and InProgress move (588-631).
- `handle_goal_completion` (`orchestrator.rs:2218-2353`): no-ops if card left InProgress (2228-2244); success → Review (2249-2279); failure at `attempt_count >= 3` → Triage + `needs_human_attention` (2292-2323); else **silent retry posture** — stays InProgress with `last_error` (2324-2347).
- `dispatch_eligible_goals` (`orchestrator.rs:1974-2071`): promotes Triage goals with all deps Complete to Ready (2045-2048), dispatches Ready (2056-2067), honors `roadmap_paused` (1980-1983).
- Startup resume `resume_in_progress_goals`/`resume_single_goal` (`orchestrator.rs:2360-2548`, spawned at 288-300): dead session → attempt++ then Ready, or Triage + `needs_human_attention` at cap (2483-2544); alive session → polling tracker that **assumes success** on idle (2438-2464).

**`needs_human_attention` set sites (exhaustive):** `orchestrator.rs:1226-1229`, `2298-2301`, `2485-2492`. Read in `goal_status` (1373-1376) and board summary (2150-2168). Nothing clears it except raw metadata writes.

## 2. Advance paths + choke point

| # | Path | File:line | Validated? |
|---|------|-----------|------------|
| P1 | MCP `goal_advance` | orchestrator.rs:1144-1320 | yes |
| P2 | dispatch paths (`dispatch_goal`, `create_roadmap`, `resume_roadmap`, approve-cascade) | orchestrator.rs:426, 1539, 1684, 1313 | partial |
| P3 | background trackers + startup resume | orchestrator.rs:568-585, 2218-2353, 2360-2548 | hardcoded |
| P4 | MCP `card_create` auto_dispatch | project_manager.rs:527-578 | partial |
| P5 | **MCP `card_move`** | project_manager.rs:595-633 | **NO — any goal to any column incl. Complete** |
| P6 | **HTTP PATCH `/api/projects/{pid}/cards/{cid}`** | routes/cards.rs:298-326 | **NO — arbitrary `column_id` AND `metadata_json` (can clear `needs_human_attention`, forge `attempt_count`)** |
| P7 | **HTTP POST `.../cards/reorder`** | routes/cards.rs:347-364 | **NO** |
| P8 | HTTP DELETE card | routes/cards.rs:328-345 | NO (silent goal deletion) |
| P9 | scheduler/cron | — | none exists (grep over crates/ found no scheduler code touching cards/goals) |

Routes registered `routes/cards.rs:368-395`, merged `routes/mod.rs:106`. All of P1-P8 converge on exactly three functions in `crates/goose/src/cards.rs`: `update_card` (498; column write 533, metadata write 558), `move_card` (588; UPDATE 622), `reorder_cards`. Grep confirms no other `UPDATE cards` writers in the workspace (only `updated_at` triggers, spectral_schema.rs:678/1127).

**Choke point proposal — enforce at the `cards.rs` data layer:**
1. New `crates/goose/src/goal_transition.rs` exposing the sole legal mutator `advance_goal_checked(pool, card_id, action, actor, decision: Option<DecisionProof>)` — one SQLite transaction: re-read → `validate_transition` → `risk_policy` tier lookup → tier gate (Tier 1 needs a resolved decision by henry-policy or jesse; Tier 2 needs `acted_by='jesse'`) → write → audit append.
2. `DecisionProof` = non-Copy/non-Clone newtype with private constructor, mintable only by the decisions module after verifying an answered `decisions` row — the SafeBrain (#277) compile-time pattern. No handler, including the orchestrator's own tools, can fabricate it.
3. `move_card`/`update_card`/`reorder_cards` refuse column changes on `card_type='goal'` rows and writes to protected metadata keys (`goal_state`, `needs_human_attention`, `attempt_count`, `last_error`, budget keys, `completed_at`) unless routed through the guard. P8 goal deletion classed Tier 2.
4. Defense in depth: `BEFORE UPDATE OF column_id ON cards` trigger aborting goal moves into complete-bound columns absent a matching answered decision row (fires for any connection, incl. raw sqlx/sqlite3).

**S1 analysis:** every daemon-mediated path (HTTP, MCP tool, background task) reaches goal state only through `cards.rs`; gating there + the proof token makes ungated Tier-2 transitions impossible for any tool call. Residual: a shell-capable worker can edit `permagent.db` directly with sqlite3 (and could even DROP the trigger) — a host-level capability no daemon hook can prevent; mitigated by the append-only audit hash chain (tampering is detectable: Complete goal with no audit row = alarm) and by classing data-root/orchestrator/policy edits as Tier-2 action classes. Judged a scoped caveat, not a swarm STOP condition — flagged for Jesse to ratify the scoping.

**TIER ENFORCEMENT BYPASS-PROOF AT DAEMON LAYER: YES** — for all daemon-mediated paths (HTTP + MCP tools + background tasks) via data-layer gating + unforgeable `DecisionProof`; explicit, detectable-but-not-preventable exception: direct SQLite file writes by shell-capable processes (host layer, outside daemon scope).

## 3. Table designs

Schema version serializes through Jesse; a parked swarm holds a WIP v8→v9, so v9 is not assumed:
```rust
pub const DECISION_INBOX_SCHEMA_VERSION: i32 = TBD; // assigned by Jesse; do NOT assume 9
```
Current `SPECTRAL_SCHEMA_VERSION: i32 = 8` (`spectral_schema.rs:13`); runner pattern `session_manager.rs:593-627` (`if version < N { migrate_… }`); fresh installs via `init_spectral_db` (`spectral_schema.rs:18`); migration body follows idempotent `migrate_v7_to_v8` template (`spectral_schema.rs:1050-1172`).

**`decisions`:**
```sql
CREATE TABLE decisions (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL CHECK (kind IN
                    ('approve_review','unblock','choice','risk_gate','malformed')),
    goal_id       TEXT REFERENCES cards(id) ON DELETE SET NULL,   -- NULL for goal-less risk_gates
    project_id    TEXT REFERENCES projects(id) ON DELETE CASCADE,
    tier          INTEGER NOT NULL CHECK (tier IN (0,1,2)),
    payload_json  TEXT NOT NULL DEFAULT '{}',                     -- schema-validated on write (S2)
    rank          REAL,                                           -- ranked-field passthrough
    status        TEXT NOT NULL DEFAULT 'open'
                  CHECK (status IN ('open','answered','expired','superseded')),
    answer        TEXT CHECK (answer IN ('approve','reject','choice','input')),
    answer_note   TEXT,
    acted_by      TEXT CHECK (acted_by IN ('jesse','henry-policy','system')), -- S5
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    resolved_at   TEXT,
    CHECK (status != 'answered' OR (answer IS NOT NULL AND acted_by IS NOT NULL AND resolved_at IS NOT NULL))
);
CREATE INDEX idx_decisions_open ON decisions(status, rank DESC, created_at) WHERE status = 'open';
CREATE INDEX idx_decisions_goal ON decisions(goal_id);
```
S2: per-kind typed serde payloads (`deny_unknown_fields`): `approve_review {evidence_digest, diff_paths[], completion_check}`, `unblock {reason: token_budget|attempt_cap|wallclock_cap|stuck, spent, cap}`, `choice {question, options[2..=8], default?}`, `risk_gate {action_class, description, requested_by}`. Validation failure → row inserted as `kind='malformed'` with `{"original_kind":…, "raw":…, "error":…}` — never coerced.

**`decision_audit` (S3, append-only):**
```sql
CREATE TABLE decision_audit (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    decision_id     TEXT NOT NULL,
    goal_id         TEXT,
    acted_by        TEXT NOT NULL,
    tier            INTEGER NOT NULL,
    outcome         TEXT NOT NULL,   -- created|approve|reject|choice|input|expired|superseded
    evidence_digest TEXT,            -- sha256 of evidence bundle shown to approver
    prev_hash       TEXT,            -- hash chain: row_hash of seq-1
    row_hash        TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TRIGGER trg_decision_audit_no_update BEFORE UPDATE ON decision_audit
    BEGIN SELECT RAISE(ABORT, 'decision_audit is append-only'); END;
CREATE TRIGGER trg_decision_audit_no_delete BEFORE DELETE ON decision_audit
    BEGIN SELECT RAISE(ABORT, 'decision_audit is append-only'); END;
```

**`risk_policy` (trust dial):**
```sql
CREATE TABLE risk_policy (
    action_class TEXT PRIMARY KEY,
    tier         INTEGER NOT NULL CHECK (tier IN (0,1,2)),
    rationale    TEXT,
    updated_by   TEXT NOT NULL DEFAULT 'system',
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
```
Seeds: `goal_complete_confined`=0 (completion_check passed AND diff confined to declared paths AND read-only/reversible class); `goal_approve_standard`=1, `goal_retry_within_budget`=1 (Henry, with recorded rationale); Tier 2: `merge_to_main`/`push_main`, `schema_migration`, `user_data_deletion` (incl. goal-card deletion), `network_external`, `spend`, `secrets_access`, `permission_change`, `orchestrator_edit`, `policy_edit`. Policy writes happen only by answering a `risk_gate(action_class='policy_edit')` decision with `acted_by='jesse'` — policy changes are Tier-2 by construction. Unknown action_class → Tier 2 (fail-closed).

## 4. Budget design (S4)

Token spend is measurable today at `crates/goose/src/agents/reply_parts.rs:473-522` (`update_session_metrics` accumulates per-call usage) persisting to `sessions.accumulated_{total,input,output}_tokens` (`session_manager.rs:68-73`, builders 180-206). Goal linkage: `worker_session_id` in goal metadata (`orchestrator.rs:598-601`); goal spend = Σ accumulated_total_tokens over its worker sessions (PI-8: keep per-attempt `worker_session_ids[]` history — today only latest survives).

Budget fields live in `cards.metadata_json` as a protected `budget` object (consistent with existing goal metadata, no cards DDL change): `token_budget`, `attempt_cap` (default 3, replaces the three hardcoded `MAX_GOAL_ATTEMPTS` comparisons at orchestrator.rs:1220, 2292, 2483), `wallclock_cap_secs` (vs `first_dispatched_at`), `spent_tokens`. Enforcement points: `dispatch_goal` precondition; `handle_goal_completion` failure branch (orchestrator.rs:2324-2347) — the silent-retry posture is replaced: on any exhaustion, emit a `kind='unblock'` decision and park the goal, never silent retry; same check in the resume path (2474-2544).

## 5. API sketch (handlers only; route registration is the coordinator's commit)

New `crates/goose-server/src/routes/decisions.rs`, modeled on `routes/cards.rs:368-395`, merged by coordinator in `routes/mod.rs` (merge block mod.rs:79-106), behind the existing Bearer middleware.

- `GET /api/decisions` — open items ordered `rank DESC NULLS LAST, created_at ASC`; item `{id, kind, tier, goal_id, goal_title, project_id, payload, rank, created_at}` (rank is a passthrough; Henry lane computes it).
- `POST /api/decisions/{id}/answer` — body `{answer: approve|reject|choice|input, note?, choice_id?, input_text?}`; atomic `UPDATE … WHERE id=? AND status='open'` (0 rows → 409), audit append, mint `DecisionProof`, execute gated effect; effect failure recorded in audit outcome. acted_by attribution: HTTP → 'jesse' (single-operator token today); Henry answers Tier-1 in-process as 'henry-policy'; timers 'system' (S5).
- `GET /api/decisions/history?limit&before=<seq>` — resolved decisions joined with audit rows, cursor on `audit.seq`.

## 6. Overlap risks with L3

1. `with_instructions` block orchestrator.rs:212-278 (esp. 221-222, 251-262) hardcodes 3-attempt + approval-gate prose that L1's behavior changes invalidate — L3 owns the text; L1 should land handlers first, L3 rebase prompts after.
2. `list_tools` orchestrator.rs:1738-1845: `goal_advance` description (1790-1799) must change in lockstep with the gated handler; `call_tool` match (1854-1875) sits between both regions.
3. `GoalAdvanceParams` (orchestrator.rs:112-121) feeds L3's schemas but L1's handler — treat as shared/frozen, changes via coordinator.
4. project_manager.rs instructions (~175) promote `card_move`; once hardened, prompts must say goal cards can't be moved manually.

## 7. Proposed issues

- PI-1 (security): HTTP PATCH card allows arbitrary `column_id` + `metadata_json` on goals — full state-machine bypass (routes/cards.rs:298-326).
- PI-2 (security): `card_move` MCP tool and reorder endpoint move goals without validation (project_manager.rs:595-633; routes/cards.rs:347-364).
- PI-3 (correctness): restart-resume assumes success for alive-then-idle sessions, promoting possibly-failed work to Review (orchestrator.rs:2438-2464).
- PI-4 (correctness): attempt-count off-by-one across paths (dispatch increments 589-609; completion checks `>= 3` at 2292; reject checks `+1 >= 3` at 1220).
- PI-5 (robustness): `update_card` per-field UPDATEs not transactional (cards.rs:509-566).
- PI-6 (race): `handle_goal_completion` TOCTOU (2228-2339); whole-blob metadata writes race between tracker and HTTP PATCH.
- PI-7 (security): DELETE card endpoint deletes goals ungated (routes/cards.rs:328-345) — should be Tier-2 `user_data_deletion`.
- PI-8 (data): per-attempt worker-session history needed for accurate token accounting.

## 8. Bottom line

TIER ENFORCEMENT BYPASS-PROOF AT DAEMON LAYER: **YES** — all goal-state writes funnel through `cards.rs` (`update_card`:498, `move_card`:588, `reorder_cards`); gating there with an unforgeable `DecisionProof` (SafeBrain pattern) makes ungated Tier-2 transitions impossible for every daemon-mediated path including the orchestrator's own tools. Residual: direct SQLite file writes by shell-capable processes are host-level, outside the daemon layer — made detectable via the audit hash chain and DB triggers, with enabling action classes themselves Tier-2. Not a STOP condition; scoping flagged for Jesse's ratification.
