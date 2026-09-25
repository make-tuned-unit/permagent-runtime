/**
 * Decision Inbox — layered evidence digest (Lane L4, amendment A3).
 *
 * Renders L2's machine-assembled EvidenceDigest
 * (crates/goose-server/src/verification/digest.rs:89-107), fetched lazily
 * from the goal card when the user expands "Evidence". Summary layer first
 * (server-built one-liners + dollars); raw check/diff/verifier output sits
 * collapsed beneath a "Show details" toggle.
 *
 * S2: raw layers render inside a <pre> as React text nodes only. Literal
 * **markdown** stays literal; URLs stay plain text; nothing is auto-linked.
 */

import { useEffect, useId, useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useCommandCenter } from '../../../lib/store';
import { Button } from '../../common/Button';
import { decisionsClient } from './client';
import type { DispatchEvidenceData, EvidenceDigestData, IndependentReviewDetail } from './types';
import { formatUsd } from './format';

/**
 * GM1 (dagK): every worker session id this panel knows about must be one
 * click from its transcript (R5, agent transparency) — the same
 * switchToSession + openChatDock pattern Automate's run rows use
 * (AutomateView.tsx RunDetail.openConversation). Session ids only ever show
 * up inside the raw `<pre>` disclosure otherwise (S2: plain text only), so
 * this button lives in the visible summary layer instead.
 */
function OpenTranscriptButton({ sessionId }: { sessionId: string }) {
  const { colors } = useTheme();
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const openChatDock = useCommandCenter(s => s.openChatDock);
  return (
    <Button
      colors={colors}
      variant="bare"
      type="button"
      flashSuccess={false}
      onClick={() => {
        switchToSession(sessionId).catch(err => console.error('[evidence] open session failed:', err));
        openChatDock();
      }}
      style={{
        '--pa-btn-fg': colors.cyan,
        '--pa-btn-fg-hover': colors.cyan,
        '--pa-btn-bg-hover': colors.cyanSoft,
        '--pa-btn-bg-active': colors.cyanGlow,
        '--pa-btn-pad': '0 4px',
        '--pa-btn-radius': `${radius.xs}px`,
        marginLeft: -4,
        alignSelf: 'flex-start',
        fontFamily: font.body,
        fontSize: textSize.micro,
      } as CSSProperties}
    >
      Open transcript
    </Button>
  );
}

export function EvidenceDigest({ projectId, goalId }: { projectId: string; goalId: string }) {
  const { colors } = useTheme();
  const [state, setState] = useState<
    | { kind: 'loading' }
    | { kind: 'none' }
    | { kind: 'ready'; digest: EvidenceDigestData | null; dispatch: DispatchEvidenceData | null }
  >({ kind: 'loading' });

  useEffect(() => {
    let cancelled = false;
    setState({ kind: 'loading' });
    // Two independent producers, fetched together: the deterministic dispatch
    // evidence (always present for a worktree+push goal) and the L2 verifier
    // digest (present only once the local grader has run).
    Promise.all([
      decisionsClient.evidence(projectId, goalId),
      decisionsClient.dispatchEvidence(projectId, goalId),
    ])
      .then(([digest, dispatch]) => {
        if (cancelled) return;
        setState(digest || dispatch ? { kind: 'ready', digest, dispatch } : { kind: 'none' });
      })
      .catch(() => { if (!cancelled) setState({ kind: 'none' }); });
    return () => { cancelled = true; };
  }, [projectId, goalId]);

  if (state.kind === 'loading') {
    return (
      <div style={{ marginTop: space.xl, fontSize: textSize.caption, color: colors.textDim, fontFamily: font.body }}>
        Loading evidence…
      </div>
    );
  }
  if (state.kind === 'none') {
    return (
      <div style={{ marginTop: space.xl, fontSize: textSize.caption, color: colors.textDim, fontFamily: font.body }}>
        No verification evidence has been recorded for this item yet.
      </div>
    );
  }
  return (
    <>
      {state.dispatch && <DispatchEvidenceView ev={state.dispatch} />}
      {state.digest && <DigestView digest={state.digest} />}
    </>
  );
}

// ── Dispatch evidence (deterministic proof-of-work) ─────────────────────────

function DispatchEvidenceView({ ev }: { ev: DispatchEvidenceData }) {
  const { colors } = useTheme();
  const [showDetails, setShowDetails] = useState(false);
  const detailsId = useId();

  const pushed = !!ev.push_target;
  const head = ev.head_commit ?? '(unknown)';
  const headline =
    ev.commits.length === 0
      ? 'Worker exited cleanly but produced no commits.'
      : `Commit ${head} ${pushed ? `pushed to ${ev.push_target}` : 'committed to worktree (not pushed)'}`;
  const diffLine = `${ev.files_changed} file${ev.files_changed === 1 ? '' : 's'} changed, +${ev.insertions} / -${ev.deletions}`;

  return (
    <div style={{ marginTop: space.xl }}>
      <div style={{
        borderRadius: radius.sm, border: `1px solid ${colors.border}`,
        padding: `${space.lg}px 14px`, fontFamily: font.body, fontSize: textSize.caption,
        color: colors.textMuted, display: 'flex', flexDirection: 'column', gap: space.sm,
      }}>
        <SummaryRow ok={ev.commits.length > 0} text={headline} />
        <div style={{ color: colors.text, fontWeight: 500 }}>{diffLine}</div>
        {/* Disclosure toggle for the raw layer below: nothing to await, so it
            keeps the element and takes the shared `.pa-btn` interaction rules
            rather than the Button primitive's pending/success machinery. */}
        <button
          type="button"
          className="pa-btn"
          aria-expanded={showDetails}
          aria-controls={detailsId}
          onClick={() => setShowDetails(o => !o)}
          style={{
            '--pa-btn-bg': 'transparent',
            '--pa-btn-fg': showDetails ? colors.cyan : colors.textDim,
            '--pa-btn-border': 'transparent',
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-fg-hover': showDetails ? colors.cyan : colors.textMuted,
            '--pa-btn-bg-active': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-radius': '0',
            '--pa-btn-weight': 400,
            alignSelf: 'flex-start',
            fontSize: textSize.micro, fontFamily: font.body,
          } as CSSProperties}
        >
          {showDetails ? 'Hide proof of work ▾' : 'Show proof of work ▸'}
        </button>
      </div>

      {showDetails && (
        <pre id={detailsId} style={{
          margin: '8px 0 0', borderRadius: radius.sm,
          background: colors.codeBg, padding: `${space.xl}px 14px`,
          fontFamily: font.mono, fontSize: textSize.micro, lineHeight: 1.6,
          color: colors.textMuted,
          whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
        }}>
          <Section label="COMMITS" color={colors.codeText} />
          {ev.commits.length ? ev.commits.join('\n') : '(none above baseline)'}
          {'\n\n'}
          <Section label="DIFF" color={colors.codeText} />
          {ev.diffstat.trim() || diffLine}
          {'\n\n'}
          <Section label="WORKTREE" color={colors.codeText} />
          {`${ev.worktree_path}\nbaseline: ${ev.baseline_commit}`}
          {ev.worker_summary.trim() && (
            <>
              {'\n\n'}
              <Section label="WORKER SUMMARY" color={colors.codeText} />
              {ev.worker_summary.trim()}
            </>
          )}
        </pre>
      )}
    </div>
  );
}

function DigestView({ digest }: { digest: EvidenceDigestData }) {
  const { colors } = useTheme();
  const [showDetails, setShowDetails] = useState(false);
  const detailsId = useId();

  const cs = digest.checks_summary;
  const checksOk = cs.total_count > 0 && cs.passed_count === cs.total_count;
  const verifierOk = digest.verifier.status === 'pass' && !digest.verifier.degraded_reason;
  // The cross-family second opinion. Absent on records written before the gate
  // existed, and on verdicts that never reached it.
  const review = digest.independent_review ?? null;

  // Dollars first; when no rate is configured the server sends cost_usd=null
  // with an explanatory note (digest.rs:165-191).
  const costLine =
    digest.costs.cost_usd != null
      ? `Cost so far: ${formatUsd(digest.costs.cost_usd)}`
      : digest.costs.accumulated_total_tokens != null
        ? `Cost so far: ${formatTokens(digest.costs.accumulated_total_tokens)} tokens (${digest.costs.note ?? 'no token rate configured'})`
        : `Cost so far: unknown (${digest.costs.note ?? 'worker token usage unavailable'})`;

  return (
    <div style={{ marginTop: space.xl }}>
      {/* Plain-language summary layer — server-built one-liners, verbatim */}
      <div style={{
        borderRadius: radius.sm, border: `1px solid ${colors.border}`,
        padding: `${space.lg}px 14px`, fontFamily: font.body, fontSize: textSize.caption,
        color: colors.textMuted, display: 'flex', flexDirection: 'column', gap: space.sm,
      }}>
        <SummaryRow ok={checksOk} text={cs.one_line} />
        <SummaryRow ok={verifierOk} text={digest.verifier_summary} />
        {review && <SummaryRow ok={review.decision === 'passed' && review.cross_family} text={review.one_line} />}
        <div style={{ color: colors.text, fontWeight: 500 }}>
          {costLine}
        </div>
        {/* Benchmark ask (dagK, Codex pattern): the actual changed FILES,
            visible without opening the raw disclosure — "N files changed"
            alone (the one-liner above, in `cs.one_line`'s neighbourhood) names
            a count, not what changed. Per-file hunks aren't in this digest
            (only aggregate insertions/deletions per file); showing lines
            here would fabricate detail the daemon never captured — flagged
            in the report as a possible follow-up capture point, not guessed
            at here. */}
        {digest.diff.per_file.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxs }}>
            {digest.diff.per_file.map(f => (
              <div key={f.path} style={{ display: 'flex', gap: space.sm, fontFamily: font.mono, fontSize: textSize.micro }}>
                <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {f.path}
                </span>
                <span style={{ color: colors.success, flexShrink: 0 }}>+{f.insertions}</span>
                <span style={{ color: colors.danger, flexShrink: 0 }}>-{f.deletions}</span>
              </div>
            ))}
          </div>
        )}
        {digest.costs.worker_session_id && <OpenTranscriptButton sessionId={digest.costs.worker_session_id} />}
        {/* Disclosure toggle for the raw layer below: nothing to await, so it
            keeps the element and takes the shared `.pa-btn` interaction rules
            rather than the Button primitive's pending/success machinery. */}
        <button
          type="button"
          className="pa-btn"
          aria-expanded={showDetails}
          aria-controls={detailsId}
          onClick={() => setShowDetails(o => !o)}
          style={{
            '--pa-btn-bg': 'transparent',
            '--pa-btn-fg': showDetails ? colors.cyan : colors.textDim,
            '--pa-btn-border': 'transparent',
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-fg-hover': showDetails ? colors.cyan : colors.textMuted,
            '--pa-btn-bg-active': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-radius': '0',
            '--pa-btn-weight': 400,
            alignSelf: 'flex-start',
            fontSize: textSize.micro, fontFamily: font.body,
          } as CSSProperties}
        >
          {showDetails ? 'Hide details ▾' : 'Show details ▸'}
        </button>
      </div>

      {/* Raw layer — plain text only */}
      {showDetails && (
        <pre id={detailsId} style={{
          margin: '8px 0 0', borderRadius: radius.sm,
          background: colors.codeBg, padding: `${space.xl}px 14px`,
          fontFamily: font.mono, fontSize: textSize.micro, lineHeight: 1.6,
          color: colors.textMuted,
          whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
        }}>
          <Section label="CHECKS" color={colors.codeText} />
          {checksText(digest)}
          {'\n\n'}
          <Section label="DIFF" color={colors.codeText} />
          {diffText(digest)}
          {'\n\n'}
          <Section label="VERIFIER" color={colors.codeText} />
          {verifierText(digest)}
          {'\n\n'}
          {review && (
            <>
              <Section label="INDEPENDENT REVIEW" color={colors.codeText} />
              {reviewText(review)}
              {'\n\n'}
            </>
          )}
          <Section label="COST" color={colors.codeText} />
          {costText(digest)}
        </pre>
      )}
    </div>
  );
}

// ── Raw-layer text assembly (joins server strings; adds no new claims) ──────

function checksText(d: EvidenceDigestData): string {
  if (d.checks.length === 0) return '(no automated checks were declared)';
  return d.checks
    .map(c => `[${c.status}] ${c.summary} (${c.type})\n${c.output_excerpt || '(no output)'}`)
    .join('\n\n');
}

function diffText(d: EvidenceDigestData): string {
  const { diff } = d;
  const lines = [
    `${diff.files_changed} file${diff.files_changed === 1 ? '' : 's'} changed, +${diff.insertions} / -${diff.deletions}`,
    ...diff.per_file.map(f => `${f.path}  +${f.insertions} / -${f.deletions}`),
  ];
  if (d.out_of_path_files.length > 0) {
    lines.push('', 'Out-of-path files:', ...d.out_of_path_files);
  }
  return lines.join('\n');
}

function verifierText(d: EvidenceDigestData): string {
  const v = d.verifier;
  const lines = [`status: ${v.status} (model: ${v.model})`];
  if (v.degraded_reason) lines.push(`degraded: ${v.degraded_reason}`);
  if (v.rationale) lines.push(v.rationale);
  return lines.join('\n');
}

// Who reviewed, from which family, through which lenses, and what they found.
// Joins server strings; adds no claim the server did not make — in particular it
// never calls a same-family review independent.
function reviewText(r: IndependentReviewDetail): string {
  const lines = [
    `decision: ${r.decision} (${r.mode} rubric)`,
    r.reviewer
      ? `reviewer: ${r.reviewer} [${r.source}] — family ${r.reviewer_family || '?'} vs worker ${r.worker_family || '?'}${r.cross_family ? '' : ' (SAME family: not an independent cross-family review)'}`
      : 'reviewer: none could be chosen',
  ];
  if (r.lenses.length > 0) lines.push(`lenses: ${r.lenses.join(', ')}`);
  if (r.checked) lines.push(`checked: ${r.checked}`);
  if (r.estimated_cost_usd != null) lines.push(`estimated cost: ${formatUsd(r.estimated_cost_usd)}`);
  else if (r.reviewer) lines.push('estimated cost: unknown (the reviewer model has no published price)');
  if (r.reason) lines.push(`reason: ${r.reason}`);
  if (r.findings.length > 0) lines.push('', 'Findings:', ...r.findings.map(f => `- ${f}`));
  return lines.join('\n');
}

function costText(d: EvidenceDigestData): string {
  const c = d.costs;
  const lines: string[] = [];
  if (c.cost_usd != null) lines.push(`${formatUsd(c.cost_usd)} to date`);
  if (c.accumulated_total_tokens != null) lines.push(`${formatTokens(c.accumulated_total_tokens)} tokens`);
  lines.push(`attempts: ${c.attempt_count}`);
  if (c.worker_session_id) lines.push(`worker session: ${c.worker_session_id}`);
  if (c.note) lines.push(c.note);
  return lines.join('\n');
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

function SummaryRow({ ok, text }: { ok: boolean; text: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', gap: space.md, alignItems: 'baseline' }}>
      <span style={{ color: ok ? colors.success : colors.warning, flexShrink: 0 }}>
        {ok ? '✓' : '•'}
      </span>
      <span>{text}</span>
    </div>
  );
}

function Section({ label, color }: { label: string; color: string }) {
  return <span style={{ color, fontWeight: 600 }}>{label}{'\n'}</span>;
}
