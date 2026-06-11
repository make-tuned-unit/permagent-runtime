/**
 * Decision Inbox — mock server (Lane L4).
 *
 * Selected by VITE_DECISIONS_MOCK=1 (see client.ts). In-memory fixtures with
 * ~300ms latency, real answer mutation, and a 409 already-resolved path.
 * Fixture text deliberately includes literal **markdown** and an unlinked URL
 * to prove the S2 plain-text rendering rule.
 */

import type {
  AnswerBody,
  Decision,
  DecisionHistoryResponse,
  DecisionsClient,
  DecisionsResponse,
} from './types';
import { DecisionConflictError } from './types';

const LATENCY_MS = 300;
const sleep = (ms: number) => new Promise<void>(r => setTimeout(r, ms));

function minutesAgo(min: number): string {
  return new Date(Date.now() - min * 60_000).toISOString();
}

// ── Pending fixtures (ranked order) ─────────────────────────────────────────

const PENDING: Decision[] = [
  {
    id: 'dec-001',
    tier: 2,
    kind: 'approval',
    headline: 'Ship the change: decision queue saving',
    detail: 'Henry finished the work that lets your decisions survive a restart.',
    effect_summary:
      'Approving will: publish the change and let Henry start connecting it to the inbox.',
    recommendation: { label: 'Approve', confidence: 'high' },
    agent_name: 'Henry',
    created_at: minutesAgo(192),
    evidence: {
      summary: {
        checks_passed: 4,
        checks_total: 4,
        tests_passed: 412,
        verifier_confirmed: true,
        verifier_summary:
          "Henry's reviewer confirmed the work matches what was approved",
        cost_usd: 4.02,
      },
      raw: {
        checks: [
          'cargo check --workspace ............ ok (2m 04s)',
          'cargo clippy --all-targets ......... ok, 0 warnings',
          'cargo test --workspace ............. 412 passed, 0 failed, 1 skipped (test_codex_provider)',
          'cargo fmt --check .................. clean',
        ].join('\n'),
        diff: [
          '14 files changed, +482 / -97',
          'crates/goose/src/decisions/*.rs (new)',
          'crates/goose-server/src/routes/decisions.rs (new)',
        ].join('\n'),
        verifier: [
          'worker di-daemon/w3: plan matches approved scope.',
          'No public API changes outside /api/decisions.',
          'Escalation text is untrusted — **do not render markdown** and',
          'https://evil.example/phish?x=1 must NOT become a clickable link.',
        ].join('\n'),
        cost_detail: '$4.02 to date (3 sessions, 1.9M tokens)',
      },
    },
  },
  {
    id: 'dec-002-conflict',
    tier: 2,
    kind: 'approval',
    headline: 'Spend a little more: voice speed experiment wants $10 extra',
    detail:
      'The experiment paused at its budget limit just before the last measurement.',
    effect_summary:
      "Approving will: raise this experiment's budget from $15 to $25 so it can finish today.",
    recommendation: { label: 'Approve', confidence: 'medium' },
    agent_name: 'Henry',
    created_at: minutesAgo(100),
    evidence: {
      summary: {
        checks_passed: 2,
        checks_total: 2,
        tests_passed: 0,
        verifier_confirmed: true,
        verifier_summary:
          'Henry checked the spend so far matches the plan he proposed',
        cost_usd: 14.61,
      },
      raw: {
        checks: 'budget guard ....... ok\nspend ledger ....... ok ($14.61 of $15.00)',
        diff: 'no code changes — budget change only',
        verifier:
          'worker di-henry/w1: sweep is 80% complete; remaining runs estimated $7.40.',
        cost_detail: '$14.61 to date (6 sessions, 5.2M tokens)',
      },
    },
  },
  {
    id: 'dec-003',
    tier: 2,
    kind: 'choice',
    headline: 'Pick a path: where to keep your decision history',
    detail:
      'Two workable options with different trade-offs; Henry needs you to choose one.',
    effect_summary:
      'Choosing will: set how past decisions are stored before any history is written.',
    recommendation: { label: 'Keep it in the existing database', confidence: 'high' },
    agent_name: 'Henry',
    created_at: minutesAgo(75),
    options: [
      {
        id: 'opt-existing-db',
        label: 'Keep it in the existing database',
        effect_summary:
          'Choosing this will: reuse the database you already back up — simplest, ready today.',
      },
      {
        id: 'opt-separate-file',
        label: 'Keep it in a separate file',
        effect_summary:
          'Choosing this will: keep history out of the main database, but add a new file to back up.',
      },
    ],
    evidence: {
      summary: {
        checks_passed: 1,
        checks_total: 1,
        tests_passed: 0,
        verifier_confirmed: true,
        verifier_summary: 'Henry compared both options against the backup setup you already have',
        cost_usd: 0.84,
      },
      raw: {
        checks: 'design review ....... ok (both options sketched and compared)',
        diff: 'no code changes yet — decision precedes implementation',
        verifier:
          'worker di-daemon/w2: existing permagent.db already covered by the nightly backup job (#274).',
        cost_detail: '$0.84 to date (1 session, 310K tokens)',
      },
    },
  },
  {
    id: 'dec-004',
    tier: 2,
    kind: 'unblock',
    headline: 'Answer a question so work can continue: which database version',
    detail: 'One of Henry’s workers is paused until you answer.',
    specific_ask:
      'Which Postgres version is the prod target — 15 or 16? The migration syntax differs.',
    effect_summary:
      'Answering will: unblock one of Henry’s workers, idle for 48 minutes.',
    recommendation: { label: 'Postgres 16', confidence: 'medium' },
    created_at: minutesAgo(48),
    options: [
      {
        id: 'opt-pg16',
        label: 'Postgres 16',
        effect_summary: 'Choosing this will: write the migrations for Postgres 16.',
      },
      {
        id: 'opt-pg15',
        label: 'Postgres 15',
        effect_summary: 'Choosing this will: write the migrations for Postgres 15.',
      },
    ],
    evidence: {
      summary: {
        checks_passed: 0,
        checks_total: 0,
        tests_passed: 0,
        verifier_confirmed: false,
        verifier_summary: 'Nothing to verify yet — work is paused on your answer',
        cost_usd: 1.1,
      },
      raw: {
        checks: '(none — blocked before checks could run)',
        diff: '(none yet)',
        verifier: [
          'worker di-daemon/w3 blocked 48m on phase 2 migrations.',
          'Repo pins no PG version; infra notes mention 16.3.',
        ].join('\n'),
        cost_detail: '$1.10 to date (1 session, 240K tokens)',
      },
    },
  },
  // ── Filler tier-2 approvals to exercise the 10-item cap + "+M more" ──
  ...[
    ['dec-005', 'Ship the change: clearer error words in the daemon log', 41],
    ['dec-006', 'Ship the change: faster startup for the morning check', 36],
    ['dec-007', 'Tidy up: remove two unused settings left from last month', 30],
    ['dec-008', 'Ship the change: nightly backup keeps one extra copy', 24],
    ['dec-009', 'Ship the change: the world view loads its map sooner', 18],
    ['dec-010', 'Tidy up: retire an old test that no longer proves anything', 11],
  ].map(([id, headline, age]): Decision => ({
    id: id as string,
    tier: 2,
    kind: 'approval',
    headline: headline as string,
    detail: 'Henry finished this and is waiting on your go-ahead.',
    effect_summary: 'Approving will: publish the change to the shared branch.',
    recommendation: { label: 'Approve' },
    agent_name: 'Henry',
    created_at: minutesAgo(age as number),
    evidence: {
      summary: {
        checks_passed: 4,
        checks_total: 4,
        tests_passed: 412,
        verifier_confirmed: true,
        verifier_summary:
          "Henry's reviewer confirmed the work matches what was approved",
        cost_usd: 0.6,
      },
      raw: {
        checks: 'cargo check / clippy / test / fmt ... all ok',
        diff: '2 files changed, +18 / -9',
        verifier: 'scope matches; no surprises.',
        cost_detail: '$0.60 to date (1 session, 180K tokens)',
      },
    },
  })),
  // Overflow items beyond the 10 shown (drives "+3 more")
  ...[
    ['dec-011', 'Ship the change: spelling fixes in the setup guide', 9],
    ['dec-012', 'Tidy up: fold two duplicate helper functions into one', 7],
    ['dec-013', 'Ship the change: the doctor command checks one more thing', 4],
  ].map(([id, headline, age]): Decision => ({
    id: id as string,
    tier: 2,
    kind: 'approval',
    headline: headline as string,
    detail: 'Henry finished this and is waiting on your go-ahead.',
    effect_summary: 'Approving will: publish the change to the shared branch.',
    recommendation: { label: 'Approve' },
    agent_name: 'Henry',
    created_at: minutesAgo(age as number),
  })),
];

// ── Tier-1 history fixtures (Henry handled these himself) ──────────────────

const HANDLED: Decision[] = [
  ['h-01', 'Retried a flaky check on a finished change — it passed the second time', 120],
  ['h-02', 'Cleared a stale lock that was holding up the verification worker', 240],
  ['h-03', 'Answered a worker question from a decision you already made (database pool size)', 360],
  ['h-04', 'Restarted a worker that had gone quiet for 20 minutes', 410],
  ['h-05', 'Refreshed a login that was about to expire', 480],
  ['h-06', 'Filed away the finished “world map speedup” goal and its notes', 520],
  ['h-07', 'Skipped a duplicate task that two workers had both picked up', 610],
].map(([id, resolution, age]): Decision => ({
  id: id as string,
  tier: 1,
  kind: 'fyi',
  headline: resolution as string,
  detail: '',
  effect_summary: '',
  resolution: resolution as string,
  created_at: minutesAgo(age as number),
  resolved_at: minutesAgo((age as number) - 2),
}));

// ── Mock client ─────────────────────────────────────────────────────────────

// Dev/screenshot knob: ?decisions=empty starts the mock with nothing pending.
const startEmpty =
  typeof window !== 'undefined' &&
  new URLSearchParams(window.location.search).get('decisions') === 'empty';

const pending: Decision[] = startEmpty ? [] : [...PENDING];
const resolved: Decision[] = [...HANDLED];

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
      handled_count: resolved.filter(d => d.tier === 1).length,
      goals_in_flight: 6,
      oldest_pending_at: oldest,
    };
  },

  async answer(id: string, body: AnswerBody): Promise<Decision> {
    await sleep(LATENCY_MS);
    const idx = pending.findIndex(d => d.id === id);
    if (idx === -1) throw new DecisionConflictError();
    // 409 path: this fixture was "answered in another window" — resolve it
    // out from under the caller and report the conflict.
    if (id === 'dec-002-conflict') {
      const [gone] = pending.splice(idx, 1);
      resolved.unshift({
        ...gone,
        resolved_at: new Date().toISOString(),
        resolution: 'Answered from another window',
      });
      throw new DecisionConflictError();
    }
    const [d] = pending.splice(idx, 1);
    const verb =
      body.answer === 'approve' ? 'Approved' :
      body.answer === 'reject' ? 'Rejected' :
      body.answer === 'choice'
        ? `Chose “${d.options?.find(o => o.id === body.choice_id)?.label ?? 'an option'}”`
        : 'Answered';
    const done: Decision = {
      ...d,
      resolved_at: new Date().toISOString(),
      resolution: body.note ? `${verb} — note: ${body.note}` : verb,
    };
    resolved.unshift(done);
    return done;
  },

  async history(): Promise<DecisionHistoryResponse> {
    await sleep(LATENCY_MS);
    return { items: resolved.map(d => ({ ...d })) };
  },
};
