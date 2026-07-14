/**
 * Decision Inbox — mock server (Lane L4).
 *
 * Selected by VITE_DECISIONS_MOCK=1 (see client.ts). Speaks the SAME shapes
 * as the real client (post-swap wire vocabulary: kinds approve_review/choice/
 * unblock/risk_gate, payload-carried options, card-carried evidence digest).
 * In-memory fixtures with ~300ms latency, real answer mutation, and a 409
 * already-resolved path. Fixture text deliberately includes literal
 * **markdown** and an unlinked URL to prove the S2 plain-text rendering rule.
 */

import type {
  AnswerBody,
  AnswerOutcome,
  Decision,
  DecisionHistoryResponse,
  DecisionsClient,
  DecisionsResponse,
  DispatchEvidenceData,
  EvidenceDigestData,
  HistoryItem,
} from './types';
import { DecisionConflictError } from './types';

const LATENCY_MS = 300;
const sleep = (ms: number) => new Promise<void>(r => setTimeout(r, ms));

function minutesAgo(min: number): string {
  return new Date(Date.now() - min * 60_000).toISOString();
}

function openDecision(partial: Partial<Decision> & Pick<Decision, 'id' | 'kind' | 'headline' | 'created_at'>): Decision {
  return {
    goal_id: null,
    project_id: null,
    tier: 2,
    detail: '',
    payload: {},
    rank: null,
    status: 'open',
    answer: null,
    answer_note: null,
    answer_choice_id: null,
    answer_input: null,
    acted_by: null,
    resolved_at: null,
    goal_title: null,
    ...partial,
  };
}

// ── Pending fixtures (ranked order) ─────────────────────────────────────────

const PENDING: Decision[] = [
  openDecision({
    id: 'dec-001',
    kind: 'approve_review',
    goal_id: 'goal-queue-saving',
    project_id: 'proj-permagent',
    goal_title: 'Decision queue saving',
    rank: 0.95,
    headline: 'Ship the change: decision queue saving',
    detail:
      'Worker for goal goal-queue-saving reported success; the goal moved to Review. ' +
      'Inspect the work and answer approve (Review → Complete) or reject (Review → InProgress for rework).',
    created_at: minutesAgo(192),
  }),
  openDecision({
    id: 'dec-002-conflict',
    kind: 'risk_gate',
    rank: 0.9,
    headline: 'Spend a little more: voice speed experiment wants $10 extra',
    detail:
      "The experiment paused at its budget limit just before the last measurement.\n\nRequested by: Henry",
    payload: {
      action_class: 'budget_increase',
      description: 'Raise the experiment budget from $15 to $25',
      requested_by: 'Henry',
    },
    created_at: minutesAgo(100),
  }),
  openDecision({
    id: 'dec-003',
    kind: 'choice',
    rank: 0.8,
    headline: 'Pick a path: where to keep your decision history',
    detail:
      'Two workable options with different trade-offs; Henry needs you to choose one.\n' +
      'Options:\n' +
      '- opt-existing-db: Keep it in the existing database — reuses the database you already back up; simplest, ready today.\n' +
      '- opt-separate-file: Keep it in a separate file — keeps history out of the main database, but adds a new file to back up.',
    payload: {
      question: 'Pick a path: where to keep your decision history',
      options: [
        { id: 'opt-existing-db', label: 'Keep it in the existing database' },
        { id: 'opt-separate-file', label: 'Keep it in a separate file' },
      ],
      default: 'opt-existing-db',
    },
    created_at: minutesAgo(75),
  }),
  openDecision({
    id: 'dec-004',
    kind: 'unblock',
    goal_id: 'goal-pg-migrations',
    project_id: 'proj-permagent',
    goal_title: 'Phase 2 migrations',
    rank: 0.7,
    headline: 'Answer a question so work can continue: which database version',
    detail:
      'Which Postgres version is the prod target — 15 or 16? The migration syntax differs.\n\n' +
      'Requested by: one of Henry’s workers (session w3, idle 48 minutes)',
    payload: { reason: 'stuck' },
    created_at: minutesAgo(48),
  }),
  // Draft-carrying proposal — exercises "approve with edits" (payload.draft):
  // revise the text and accept (answer='edit'); the daemon learns the
  // draft→revision delta (edit-as-training). Fixture keeps `detail` == `draft`
  // so the edit box shows exactly what the user is revising.
  openDecision({
    id: 'dec-004b-draft',
    kind: 'automation_proposal',
    project_id: 'proj-permagent',
    rank: 0.68,
    headline: 'Automate your morning git sync?',
    detail: "You've run `git status && git pull` 3 times this week.",
    payload: {
      normalized_command: 'git status && git pull',
      occurrence_count: 3,
      exemplars: ['git status && git pull'],
      draft: "You've run `git status && git pull` 3 times this week.",
    },
    created_at: minutesAgo(44),
  }),
  // ── Filler approvals to exercise the 10-item cap + "+M more" ──
  ...[
    ['dec-005', 'Ship the change: clearer error words in the daemon log', 41],
    ['dec-006', 'Ship the change: faster startup for the morning check', 36],
    ['dec-007', 'Tidy up: remove two unused settings left from last month', 30],
    ['dec-008', 'Ship the change: nightly backup keeps one extra copy', 24],
    ['dec-009', 'Ship the change: the world view loads its map sooner', 18],
    ['dec-010', 'Tidy up: retire an old test that no longer proves anything', 11],
  ].map(([id, headline, age]) =>
    openDecision({
      id: id as string,
      kind: 'approve_review',
      goal_id: `goal-${id}`,
      project_id: 'proj-permagent',
      headline: headline as string,
      detail: 'Henry finished this and is waiting on your go-ahead.',
      created_at: minutesAgo(age as number),
    }),
  ),
  // Overflow items beyond the 10 shown (drives "+3 more")
  ...[
    ['dec-011', 'Ship the change: spelling fixes in the setup guide', 9],
    ['dec-012', 'Tidy up: fold two duplicate helper functions into one', 7],
    ['dec-013', 'Ship the change: the doctor command checks one more thing', 4],
  ].map(([id, headline, age]) =>
    openDecision({
      id: id as string,
      kind: 'approve_review',
      headline: headline as string,
      detail: 'Henry finished this and is waiting on your go-ahead.',
      created_at: minutesAgo(age as number),
    }),
  ),
];

// ── Evidence digests (served via the goal card in the real daemon) ─────────

const DISPATCH_EVIDENCE: Record<string, DispatchEvidenceData> = {
  'goal-queue-saving': {
    worktree_path: '/Users/jesse/dev/.permagent-goal-worktrees/cli-f69980d2',
    baseline_commit: 'a1190cd',
    head_commit: '7d4f9ea',
    commits: ['7d4f9ea Create thread: Canada renewable energy and emissions targets'],
    diffstat:
      ' src/data/sources.ts            |  12 +\n src/data/threads/the-switch.ts | 1140 +++++++++\n src/data/threads/index.ts      |   7 +',
    files_changed: 3,
    insertions: 1159,
    deletions: 0,
    push_target: 'origin/main',
    worker_summary: 'Created the thread with 3 sources, committed and pushed to origin/main.',
  },
};

const DIGESTS: Record<string, EvidenceDigestData> = {
  'goal-queue-saving': {
    version: 1,
    goal_id: 'goal-queue-saving',
    goal_title: 'Decision queue saving',
    checks_summary: {
      passed_count: 4,
      total_count: 4,
      tests_run: 412,
      one_line: 'All 4 automated checks passed (412 tests)',
    },
    verifier_summary: "Henry's reviewer confirmed the work matches what was approved",
    costs: {
      cost_usd: 4.02,
      accumulated_total_tokens: 1_900_000,
      worker_session_id: 'di-daemon-w3',
      attempt_count: 1,
      note: null,
    },
    checks: [
      {
        type: 'command_exit_zero',
        summary: 'cargo check --workspace',
        status: 'pass',
        output_excerpt: 'exit: 0\nstdout: Finished dev profile in 2m 04s',
      },
      {
        type: 'command_exit_zero',
        summary: 'cargo clippy --all-targets',
        status: 'pass',
        output_excerpt: 'exit: 0\nstdout: 0 warnings',
      },
      {
        type: 'command_exit_zero',
        summary: 'cargo test --workspace',
        status: 'pass',
        output_excerpt:
          'exit: 0\nstdout: 412 passed; 0 failed; 1 skipped (test_codex_provider)\n' +
          'note: escalation text is untrusted — **do not render markdown** and\n' +
          'https://evil.example/phish?x=1 must NOT become a clickable link.',
      },
      {
        type: 'command_exit_zero',
        summary: 'cargo fmt --check',
        status: 'pass',
        output_excerpt: 'exit: 0',
      },
    ],
    diff: {
      files_changed: 14,
      insertions: 482,
      deletions: 97,
      per_file: [
        { path: 'crates/goose/src/decisions.rs', insertions: 310, deletions: 12 },
        { path: 'crates/goose-server/src/routes/decisions.rs', insertions: 172, deletions: 85 },
      ],
    },
    out_of_path_files: [],
    verifier: {
      status: 'pass',
      rationale: 'Plan matches approved scope. No public API changes outside /api/decisions.',
      model: 'qwen2.5:7b',
      degraded_reason: null,
    },
    started_at: minutesAgo(200),
    finished_at: minutesAgo(195),
  },
};

// ── History fixtures (resolved rows + audit join) ───────────────────────────

let seq = 100;
function historyItem(
  base: Partial<Decision> & Pick<Decision, 'id' | 'kind' | 'headline'>,
  ageMin: number,
  actedBy: string,
  answer: string,
): HistoryItem {
  return {
    ...openDecision({ created_at: minutesAgo(ageMin), ...base }),
    tier: actedBy === 'henry-policy' ? 1 : 2,
    status: 'answered',
    answer,
    acted_by: actedBy,
    resolved_at: minutesAgo(ageMin - 2),
    seq: seq--,
    outcome: answer,
    audit_acted_by: actedBy,
    audit_created_at: minutesAgo(ageMin - 2),
  };
}

const HISTORY: HistoryItem[] = [
  ['h-01', 'Retried a flaky check on a finished change — it passed the second time', 120],
  ['h-02', 'Cleared a stale lock that was holding up the verification worker', 240],
  ['h-03', 'Answered a worker question from a decision you already made (database pool size)', 360],
  ['h-04', 'Restarted a worker that had gone quiet for 20 minutes', 410],
  ['h-05', 'Refreshed a login that was about to expire', 480],
  ['h-06', 'Filed away the finished "world map speedup" goal and its notes', 520],
  ['h-07', 'Skipped a duplicate task that two workers had both picked up', 610],
].map(([id, headline, age]) =>
  historyItem(
    { id: id as string, kind: 'unblock', headline: headline as string },
    age as number,
    'henry-policy',
    'approve',
  ),
);

// ── Mock client ─────────────────────────────────────────────────────────────

// Dev/screenshot knob: ?decisions=empty starts the mock with nothing pending.
const startEmpty =
  typeof window !== 'undefined' &&
  new URLSearchParams(window.location.search).get('decisions') === 'empty';

const pending: Decision[] = startEmpty ? [] : [...PENDING];
const resolved: HistoryItem[] = [...HISTORY];

const LIST_CAP = 10;

export const mockDecisionsClient: DecisionsClient = {
  async list(opts?: { all?: boolean }): Promise<DecisionsResponse> {
    await sleep(LATENCY_MS);
    const oldest = pending.length
      ? pending.reduce((a, b) => (a.created_at < b.created_at ? a : b)).created_at
      : null;
    return {
      decisions: (opts?.all ? pending : pending.slice(0, LIST_CAP)).map(d => ({ ...d })),
      total_pending: pending.length,
      handled_count: resolved.length,
      goals_in_flight: 6,
      oldest_pending_at: oldest,
    };
  },

  async answer(id: string, body: AnswerBody): Promise<AnswerOutcome> {
    await sleep(LATENCY_MS);
    const idx = pending.findIndex(d => d.id === id);
    if (idx === -1) throw new DecisionConflictError();
    // 409 path: this fixture was "answered in another window" — resolve it
    // out from under the caller and report the conflict.
    if (id === 'dec-002-conflict') {
      const [gone] = pending.splice(idx, 1);
      resolved.unshift({
        ...gone,
        status: 'answered',
        answer: 'approve',
        acted_by: 'jesse',
        resolved_at: new Date().toISOString(),
        seq: seq--,
        outcome: 'approve',
        audit_acted_by: 'jesse',
        audit_created_at: new Date().toISOString(),
      });
      throw new DecisionConflictError();
    }
    const [d] = pending.splice(idx, 1);
    const done: HistoryItem = {
      ...d,
      status: 'answered',
      answer: body.answer,
      answer_note: body.note ?? null,
      answer_choice_id: body.choice_id ?? null,
      answer_input: body.input_text ?? null,
      acted_by: 'jesse',
      resolved_at: new Date().toISOString(),
      seq: seq--,
      outcome: body.answer,
      audit_acted_by: 'jesse',
      audit_created_at: new Date().toISOString(),
    };
    resolved.unshift(done);
    const effect =
      d.kind === 'approve_review' && body.answer === 'approve'
        ? 'goal approved: Review → Complete'
        : d.kind === 'automation_proposal' && body.answer === 'edit'
          ? 'automation proposal approved with edits'
          : d.kind === 'automation_proposal' && body.answer === 'approve'
            ? 'automation proposal approved'
            : null;
    return { decision: done, effect, effect_error: null };
  },

  async history(): Promise<DecisionHistoryResponse> {
    await sleep(LATENCY_MS);
    return {
      items: resolved.map(d => ({ ...d })),
      next_before: resolved.length ? resolved[resolved.length - 1].seq : null,
    };
  },

  async evidence(_projectId: string, goalId: string): Promise<EvidenceDigestData | null> {
    await sleep(LATENCY_MS);
    return DIGESTS[goalId] ?? null;
  },

  async dispatchEvidence(
    _projectId: string,
    goalId: string,
  ): Promise<DispatchEvidenceData | null> {
    await sleep(LATENCY_MS);
    return DISPATCH_EVIDENCE[goalId] ?? null;
  },

  async cancelGoal(_projectId: string, _goalId: string): Promise<void> {
    await sleep(LATENCY_MS);
  },
};
