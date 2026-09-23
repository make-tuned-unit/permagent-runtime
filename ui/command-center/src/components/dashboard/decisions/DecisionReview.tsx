import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { apiFetch, type CouncilLatest } from '../../../lib/api';
import { useCommandCenter } from '../../../lib/store';
import { radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { Tooltip } from '../../common/Tooltip';
import { DecisionActions, badgeFor } from './DecisionItem';
import { DiscussionPane } from './DiscussionPane';
import { EvidenceDigest } from './EvidenceDigest';
import { effectSummaryFor } from './decisionEffects';
import { evidenceTargetFor } from './decisionEvidence';
import { formatAge } from './format';
import { decisionsClient } from './client';
import { useDecisionQueue } from './useDecisionQueue';
import type { useDecisions } from './useDecisions';

/** Owned by DecisionInbox: DetailModal remounts its body when placement changes
 * for goal evidence, but this review opening must retain identity and drafts. */
export function useDecisionReviewState(inbox: ReturnType<typeof useDecisions>) {
  const queue = useDecisionQueue({ decisions: inbox.data?.decisions ?? [] });
  const d = queue.ordered.find(item => item.id === queue.focusedId);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [notices, setNotices] = useState<string[]>([]);
  const [conflictId, setConflictId] = useState<string | null>(null);
  const [report, setReport] = useState<{ id: string; data?: CouncilLatest; error?: string } | null>(null);
  const reportRequest = useRef(0);
  const answers = useRef(new Map<string, ReturnType<typeof inbox.answer>>());
  const previous = useRef(d);
  const [evidenceGoal, setEvidenceGoal] = useState(false);
  return { queue, d, notes, setNotes, notices, setNotices, conflictId, setConflictId,
    report, setReport, reportRequest, answers, previous, evidenceGoal, setEvidenceGoal };
}

export function DecisionReview({ inbox, agentName, state, active = true }: {
  inbox: ReturnType<typeof useDecisions>;
  agentName: string;
  state: ReturnType<typeof useDecisionReviewState>;
  active?: boolean;
}) {
  const { colors } = useTheme();
  const { queue, d, notes, setNotes, notices, setNotices, conflictId, setConflictId,
    report, setReport, reportRequest, answers, previous, evidenceGoal, setEvidenceGoal } = state;
  const host = useRef<HTMLDivElement>(null);
  const noteId = useId();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const goalDetail = useCommandCenter(s => s.goalDetail);
  const closeGoalDetail = useCommandCenter(s => s.closeGoalDetail);
  const [narrow, setNarrow] = useState(true);
  useEffect(() => {
    const element = host.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    // Match the shell's 640px narrow-sheet breakpoint, measured on this container.
    const observer = new ResizeObserver(entries => setNarrow(entries[0].contentRect.width < 640));
    observer.observe(element);
    return () => observer.disconnect();
  }, [active]);
  useEffect(() => {
    const old = previous.current;
    if (old && !queue.ordered.some(item => item.id === old.id)) {
      setNotices(items => [...items, `${old.headline} is no longer pending.`]);
    }
    if (old?.id !== d?.id) {
      reportRequest.current++;
      setReport(null);
    }
    previous.current = d;
  }, [d, queue.ordered]);
  useEffect(() => {
    if (!conflictId) return;
    const stillOpen = inbox.data?.decisions.some(item => item.id === conflictId);
    setNotices(items => [...items, stillOpen
      ? 'Decision is still open. Verification may not be ready; review the refreshed evidence before retrying.'
      : 'Decision was already resolved.']);
    setConflictId(null);
  }, [conflictId, inbox.data]);
  const openReport = async (id: string) => {
    const request = ++reportRequest.current;
    setReport({ id });
    try {
      const data = await apiFetch<CouncilLatest>(`/api/council/${encodeURIComponent(id)}`);
      if (request === reportRequest.current) setReport({ id, data });
    } catch {
      if (request === reportRequest.current) setReport({ id, error: 'Could not load this Council report.' });
    }
  };
  const keyboard = (event: ReactKeyboardEvent<HTMLDivElement> | KeyboardEvent) => {
    if ((inbox.data?.total_pending ?? 0) > queue.ordered.length || !active || report || (evidenceGoal && goalDetail) || event.defaultPrevented || ('nativeEvent' in event ? event.nativeEvent.isComposing : event.isComposing)
      || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const target = event.target;
    if (target instanceof HTMLElement && target.closest(
      'input, textarea, select, button, a, summary, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="combobox"], [role="slider"], [role="menu"]',
    )) return;
    if (event.key === 'ArrowRight' || event.key === 'j') { event.preventDefault(); queue.next(); }
    if (event.key === 'ArrowLeft' || event.key === 'k') { event.preventDefault(); queue.prev(); }
  };
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      const element = host.current;
      // The modal initially focuses its shell. Contained panes never capture
      // keys from the still-interactive page outside their own dialog.
      if (element && event.target instanceof Node && !element.contains(event.target)
        && element.closest('[role="dialog"]')?.contains(event.target)) keyboard(event);
    };
    document.addEventListener('keydown', listener);
    return () => document.removeEventListener('keydown', listener);
  });
  const complete = (inbox.data?.total_pending ?? 0) <= queue.ordered.length;
  const position = complete ? queue.position : null;
  const effect = d ? effectSummaryFor(d) : null;
  const target = d ? evidenceTargetFor(d) : null;
  const badge = d ? badgeFor(d, colors) : null;
  if (!active) return null;
  return <div ref={host} data-testid="decision-review" tabIndex={0} onKeyDown={keyboard}
    style={{ padding: space.xxl, background: colors.surface, color: colors.text, fontSize: textSize.body, minWidth: 0, overflowWrap: 'anywhere' }}>
    <div role="status" aria-live="polite" className="sr-only" style={{ color: colors.textMuted, fontSize: textSize.caption }}>
      {position ? `Decision ${position.index + 1} of ${position.total}` : ''}
    </div>
    {notices.map((notice, index) => <p key={index} role="status">{notice}</p>)}
    {!complete && <p>Loading all pending decisions… <Button colors={colors} onClick={() => inbox.showAll()} flashSuccess={false}>Retry</Button></p>}
    {complete && !d && queue.ordered.length > 0 && <Button colors={colors} onClick={() => queue.focus(queue.ordered[0].id)}>Review next pending decision</Button>}
    {d && position && <>
      <header style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md, marginBottom: space.xxl }}>
        <strong style={{ fontSize: textSize.title }}>Needs you</strong>
        <span>{position.index + 1} of {position.total}</span>
        <span aria-hidden="true" style={{ display: 'flex', gap: space.xs, alignItems: 'center' }}>
          {queue.ordered.slice(Math.max(0, position.index - 5), Math.max(0, position.index - 5) + 6).map(item =>
            <span key={item.id} style={{ width: space.md, height: space.md, borderRadius: radius.pill,
              background: item.id === d.id ? colors.cyan : colors.borderHi }} />)}
          {position.total > 6 && <span>+{position.total - 6}</span>}
        </span>
        <Tooltip content="Previous decision"><Button colors={colors} aria-label="Previous decision" disabled={!!report || position.index === 0} onClick={queue.prev}>←</Button></Tooltip>
        <Tooltip content="Next decision"><Button colors={colors} aria-label="Next decision" disabled={!!report || position.index === position.total - 1} onClick={queue.next}>→</Button></Tooltip>
        <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>← → or J / K</span>
      </header>
      {report ? <section aria-label="Council report">
        <Button colors={colors} onClick={() => { reportRequest.current++; setReport(null); }}>← Back to decision</Button>
        <p>Council session {report.id}</p>
        {report.error ? <><p role="alert">{report.error}</p><Button colors={colors} onClick={() => openReport(report.id)}>Retry</Button></>
          : !report.data ? <p>Loading report…</p>
          : report.data.report ? <><h3 style={{ fontSize: textSize.heading }}>{report.data.report.headline}</h3><p style={{ whiteSpace: 'pre-wrap' }}>{report.data.report.markdown}</p></>
          : <p>No report exists for this Council session.</p>}
      </section> : <div style={{ display: 'grid', gridTemplateColumns: narrow ? 'minmax(0, 1fr)' : 'minmax(0, 7fr) minmax(0, 5fr)', gap: space.xxl }}>
        <section data-testid={`decision-${d.id}`} style={{ minWidth: 0, display: 'flex', flexDirection: 'column', gap: space.xl }}>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md, fontSize: textSize.caption }}>
            <span style={{ color: badge?.color, background: badge?.bg, padding: space.xs, borderRadius: radius.sm }}>{badge?.label}</span>
            <span>tier {d.tier}{d.tier === 2 ? ' · needs you' : ''}</span>
            <span style={{ color: colors.textMuted }}>filed {formatAge(d.created_at)} ago{d.goal_title ? ` · ${d.goal_title}` : ''}
              {target?.kind === 'council_report' ? ` · Council session ${target.councilSessionId}` : ''}</span>
          </div>
          <h2 style={{ margin: 0, fontSize: textSize.title }}>{d.headline}</h2>
          <p style={{ margin: 0, whiteSpace: 'pre-wrap' }}>{d.detail}</p>
          <section style={{ background: colors.bg, borderRadius: radius.md, padding: space.xl }}>
            <h3 style={{ fontSize: textSize.caption }}>IF YOU APPROVE</h3>
            {effect?.known ? <><ol style={{ paddingLeft: space.huge }}>{effect.lines.map((line, index) => <li key={index}>{line.replace(/^\d+\.\s*/, '')}</li>)}</ol>
              {effect.missing?.length ? <details><summary>Missing details</summary>missing: {effect.missing.join(', ')}</details> : null}</>
              : <p>{effect && !effect.known ? effect.text : ''}</p>}
          </section>
          <section style={{ background: colors.bg, borderRadius: radius.md, padding: space.xl }}>
            <h3 style={{ fontSize: textSize.caption }}>EVIDENCE</h3>
            {target?.kind === 'council_report' && <Button colors={colors} onClick={() => openReport(target.councilSessionId)}>{target.label}</Button>}
            {target?.kind === 'dispatch' && <><EvidenceDigest projectId={target.projectId} goalId={target.goalId} />
              <Button colors={colors} onClick={() => { setEvidenceGoal(true); openGoalDetail(target.projectId, target.goalId); }}>{target.label}</Button>
              <p style={{ fontSize: textSize.caption, color: colors.textMuted }}>Close goal detail to return to this decision. Your note stays here.</p>
              {evidenceGoal && goalDetail && <Button colors={colors} onClick={() => { closeGoalDetail(); setEvidenceGoal(false); }}>← Back to decision</Button>}
            </>}
            {target?.kind === 'pause_episode' && <p>Job {target.jobId} · pause episode {target.episodeId} — no viewer yet.</p>}
            {target?.kind === 'budget' && <details><summary>{target.label}</summary>{target.detail}</details>}
            {target?.kind === 'none' && <p>{target.reason}</p>}
          </section>
          <label htmlFor={noteId}>Note</label>
          <textarea id={noteId} value={notes[d.id] ?? ''} rows={2} placeholder="Optional — goes in the audit trail"
            onChange={event => setNotes(current => ({ ...current, [d.id]: event.target.value }))}
            style={{ boxSizing: 'border-box', width: '100%', resize: 'vertical', padding: space.md, borderRadius: radius.sm,
              border: `1px solid ${colors.border}`, background: colors.inputBg, color: colors.text, fontSize: textSize.body }} />
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md }}>
            <DecisionActions key={d.id} decision={d} agentName={agentName} note={notes[d.id] ?? ''} embeddedReview
              onNoteChange={note => setNotes(current => ({ ...current, [d.id]: note }))}
              onAnswered={async body => {
                let request = answers.current.get(d.id);
                if (!request) {
                  request = inbox.answer(d.id, body);
                  answers.current.set(d.id, request);
                }
                let result: Awaited<ReturnType<typeof inbox.answer>>;
                try { result = await request; }
                catch (error) { answers.current.delete(d.id); throw error; }
                if (!result.ok) answers.current.delete(d.id);
                if (result.ok) setNotices(items => [...items, `${d.headline}: answer recorded.${result.effect_error ? ` Follow-through failed: ${result.effect_error}` : result.effect ? ` ${result.effect}` : ''}`]);
                else { await inbox.refresh(); setConflictId(d.id); }
                return result;
              }} onConflict={() => { void inbox.refresh(); }} onDiscardStaged={inbox.discardStaged}
              onCancelGoal={d.project_id && d.goal_id ? async () => { await decisionsClient.cancelGoal(d.project_id!, d.goal_id!); await inbox.refresh(); } : undefined} />
            <Button colors={colors} onClick={queue.later}>Later</Button>
          </div>
        </section>
        <DiscussionPane key={d.id} decisionId={d.id} headline={d.headline} agentName={agentName} />
      </div>}
    </>}
  </div>;
}
