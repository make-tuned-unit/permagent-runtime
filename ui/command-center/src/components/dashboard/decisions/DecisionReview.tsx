/**
 * Decision Review — one decision at a time, read top to bottom (D1, U6/U7).
 *
 *   ② eyebrow   human kind name · age                 [Henry suggests …]
 *   ③ title     the goal's full title (2-line clamp, Tooltip when cut)
 *   ④ cause     ≤2 plain sentences — never the raw `detail`
 *   ⑤ If you choose   one row per button, from the same record as ⑨
 *   ⑥ Evidence  inline digest + "Open full evidence…"
 *   ⑦ Technical details   verbatim detail, ids, raw error (collapsed)
 *   ⑧ Ask Henry  inline composer, thread above it
 *
 * The header (①) and the action bar (⑨) are rendered by DecisionInbox into the
 * shell's header and footer slots; all three read `useDecisionReviewState`,
 * which lives above the shell so a body remount keeps drafts and holds.
 *
 * A reversible verdict is held for the Undo toast's lifetime, the queue
 * auto-advances, and Z (or the toast's Undo) brings it back. Irreversible
 * actions go through ConfirmDialog instead (U12/U13, R4).
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { apiFetch, type CouncilLatest } from '../../../lib/api';
import { useCommandCenter } from '../../../lib/store';
import { toast } from '../../../lib/notifications';
import { concentric, duration, ease, font, radius, space, tabularNums, textSize, type } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { AnimatedNumber } from '../../common/AnimatedNumber';
import { Button } from '../../common/Button';
import { SectionLabel } from '../../common/Card';
import { Chip } from '../../common/Chip';
import { ConfirmDialog } from '../../common/ConfirmDialog';
import { Skeleton, SkeletonText } from '../../common/Skeleton';
import { Tooltip } from '../../common/Tooltip';
import { DecisionActionBar } from './DecisionItem';
import { DiscussionPane } from './DiscussionPane';
import { EvidenceDigest } from './EvidenceDigest';
import {
  decisionPlan, effectRows, pushedRejectWarning, recommendedLabel, toolArgumentsText,
  type VerdictAction,
} from './decisionEffects';
import {
  absoluteTime, ageLabel, decisionTitle, evidenceTargetFor, humanDuration, kindLabel,
  plainCause, technicalDetails, unblockReason,
} from './decisionEvidence';
import { decisionsClient } from './client';
import { commandForKey } from './DecisionReview.keymap';
import { useHeldAnswers } from './DecisionReview.undo';
import { useDecisionQueue } from './useDecisionQueue';
import type { useDecisions } from './useDecisions';
import type { AnswerBody, Decision } from './types';
import { checkApprovalOf, draftText, stagedSummary } from './types';

/** Horizontal body padding; the technical block's corner is concentric with it. */
export const REVIEW_PAD_X = space.xxxl;

interface FieldState { decisionId: string; actionId: string; text: string }
interface ConfirmState { decision: Decision; action: VerdictAction; body: AnswerBody | null }

/**
 * Owned by DecisionInbox: DetailModal remounts its body when placement
 * changes for goal evidence, and the review must keep identity, drafts and
 * held answers across that.
 */
export function useDecisionReviewState(inbox: ReturnType<typeof useDecisions>, agentName: string) {
  const all = useMemo(() => inbox.data?.decisions ?? [], [inbox.data]);
  const [announcement, setAnnouncement] = useState('');
  const [refocus, setRefocus] = useState<string | null>(null);
  const titles = useRef(new Map<string, string>());
  for (const d of all) titles.current.set(d.id, decisionTitle(d));
  const titleOf = (id: string) => titles.current.get(id) ?? 'This decision';

  const held = useHeldAnswers({
    send: inbox.answer,
    onSent: (_id, result) => {
      if (result.effect_error) {
        toast('Answer recorded — but the follow-through failed', result.effect_error, undefined, { tone: 'danger', force: true });
      }
    },
    onConflict: id => {
      held.release(id);
      void inbox.refresh();
      toast("That answer wasn't accepted", `${titleOf(id)}: it may have been answered elsewhere, or verification isn't ready yet. The inbox has been refreshed.`, undefined, { force: true });
    },
    onFailed: (id, error) => {
      setRefocus(id);
      const why = error instanceof Error && error.message ? `${error.message}. ` : '';
      toast("Couldn't send your answer", `${why}${titleOf(id)} is back in your queue.`, undefined, { tone: 'danger', force: true });
    },
  });
  const { hidden, settle } = held;
  useEffect(() => { settle(new Set(all.map(d => d.id))); }, [all, settle]);

  const visible = useMemo(() => all.filter(d => !hidden.has(d.id)), [all, hidden]);
  const queue = useDecisionQueue({ decisions: visible });
  const d = queue.ordered.find(item => item.id === queue.focusedId);
  useEffect(() => {
    if (refocus && queue.ordered.some(item => item.id === refocus)) {
      queue.focus(refocus);
      setRefocus(null);
    }
  });
  useEffect(() => {
    if (!d && queue.ordered.length > 0) queue.focus(queue.ordered[0].id);
  });

  const [field, setField] = useState<FieldState | null>(null);
  const [confirm, setConfirm] = useState<ConfirmState | null>(null);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [report, setReport] = useState<{ id: string; data?: CouncilLatest; error?: string } | null>(null);
  const reportRequest = useRef(0);
  const [evidenceGoal, setEvidenceGoal] = useState(false);
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  const askRef = useRef<HTMLTextAreaElement>(null);

  const plan = useMemo(() => (d ? decisionPlan(d, agentName) : null), [d, agentName]);
  const fieldAction = d && plan && field?.decisionId === d.id
    ? plan.actions.find(a => a.id === field.actionId) ?? null
    : null;

  // External resolution: say so (live region) instead of piling up <p>s (U11).
  const previous = useRef<Decision | undefined>(d);
  useEffect(() => {
    const old = previous.current;
    if (old && !all.some(item => item.id === old.id) && !hidden.has(old.id)) {
      setAnnouncement(`${decisionTitle(old)} is no longer pending.`);
    }
    if (old?.id !== d?.id) {
      reportRequest.current++;
      setReport(null);
    }
    previous.current = d;
  }, [d, all, hidden]);

  useEffect(() => { if (fieldAction) fieldRef.current?.focus(); }, [fieldAction]);

  const undo = useCallback((id?: string) => {
    const restored = held.undo(id);
    if (restored) {
      setRefocus(restored);
      setAnnouncement(`${titleOf(restored)} is back in your queue.`);
    }
    return restored;
  }, [held]);

  const hold = (target: Decision, action: VerdictAction, body: AnswerBody) => {
    held.hold(target.id, body, action.doneLabel);
    setField(null);
    setShortcutsOpen(false);
    setAnnouncement(`${action.doneLabel}. Press Z to undo.`);
    toast(action.doneLabel, decisionTitle(target), undefined, {
      force: true,
      action: { label: 'Undo', onAction: () => { undo(target.id); } },
    });
  };

  /** A verdict was chosen (click, key or Return). */
  const choose = (action: VerdictAction, text?: string) => {
    if (!d) return;
    if (action.id === 'cancel_goal') { setConfirm({ decision: d, action, body: null }); return; }
    if (action.field && text === undefined) {
      setField({ decisionId: d.id, actionId: action.id, text: action.field === 'edit' ? draftText(d) ?? '' : '' });
      return;
    }
    if (!action.body) return;
    const body: AnswerBody = { ...action.body };
    const trimmed = text?.trim() ?? '';
    if (action.field === 'input' || action.field === 'edit') {
      if (!trimmed) return;
      body.input_text = trimmed;
    } else if (trimmed) {
      body.note = trimmed;
    }
    const staged = d.staged_answer;
    if (!body.note && staged?.note && staged.answer === body.answer) body.note = staged.note;
    if (action.confirm) { setConfirm({ decision: d, action, body }); return; }
    hold(d, action, body);
  };

  const submitField = () => {
    if (fieldAction && field) choose(fieldAction, field.text);
  };

  const later = () => {
    setField(null);
    queue.later();
  };

  const runConfirm = async () => {
    if (!confirm) return;
    const { decision: target, action, body } = confirm;
    if (action.id === 'cancel_goal') {
      await decisionsClient.cancelGoal(target.project_id!, target.goal_id!);
      await inbox.refresh();
      toast('Goal cancelled', decisionTitle(target), undefined, { force: true });
    } else if (body) {
      const result = await inbox.answer(target.id, body);
      if (!result.ok) {
        await inbox.refresh();
        toast("That answer wasn't accepted", 'It may have been answered elsewhere. The inbox has been refreshed.', undefined, { force: true });
      } else if (result.effect_error) {
        toast('Answer recorded — but the follow-through failed', result.effect_error, undefined, { tone: 'danger', force: true });
      } else {
        toast(action.doneLabel, decisionTitle(target), undefined, { force: true });
      }
    }
    setConfirm(null);
    return true;
  };

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
  const closeReport = () => { reportRequest.current++; setReport(null); };

  const complete = (inbox.data?.total_pending ?? 0) <= all.length;
  return {
    all, queue, d, plan, complete, announcement, held, undo,
    field, setField, fieldAction, fieldRef, askRef,
    confirm, setConfirm, runConfirm,
    shortcutsOpen, setShortcutsOpen,
    report, openReport, closeReport,
    evidenceGoal, setEvidenceGoal,
    choose, submitField, later,
    discardStaged: inbox.discardStaged,
  };
}

export type DecisionReviewState = ReturnType<typeof useDecisionReviewState>;

// ── ① Header ────────────────────────────────────────────────────────────

const quietBare = (colors: ReturnType<typeof useTheme>['colors']): CSSProperties => ({
  '--pa-btn-fg': colors.textMuted,
  '--pa-btn-fg-hover': colors.text,
  '--pa-btn-bg-hover': colors.fillHover,
  '--pa-btn-bg-active': colors.fillActive,
  '--pa-btn-pad': `${space.xs}px ${space.md}px`,
  fontFamily: font.body,
} as CSSProperties);

export function ReviewHeader({ state }: { state: DecisionReviewState }) {
  const { colors } = useTheme();
  const { queue, complete, report } = state;
  const position = complete ? queue.position : null;
  return (
    <div tabIndex={-1} style={{ display: 'flex', alignItems: 'center', gap: space.md, minWidth: 0, outline: 'none' }}>
      <h2 style={{ margin: 0, ...type.heading, fontFamily: font.display, color: colors.text, whiteSpace: 'nowrap' }}>Needs you</h2>
      {position && (
        <>
          <span style={{ ...type.small, ...tabularNums, color: colors.textMuted, whiteSpace: 'nowrap' }}>
            <AnimatedNumber value={position.index + 1} /> of <AnimatedNumber value={position.total} />
          </span>
          <span style={{ display: 'inline-flex', gap: space.xxs }}>
            <Tooltip content="Previous" shortcut="K">
              <Button colors={colors} variant="bare" aria-label="Previous decision" aria-keyshortcuts="K"
                disabled={!!report || position.index === 0} onClick={queue.prev} style={quietBare(colors)}>‹</Button>
            </Tooltip>
            <Tooltip content="Next" shortcut="J">
              <Button colors={colors} variant="bare" aria-label="Next decision" aria-keyshortcuts="J"
                disabled={!!report || position.index === position.total - 1} onClick={queue.next} style={quietBare(colors)}>›</Button>
            </Tooltip>
          </span>
        </>
      )}
    </div>
  );
}

// ── ⑨ Action bar ────────────────────────────────────────────────────────

export function ReviewActionBar({ state }: { state: DecisionReviewState }) {
  const { d, plan, fieldAction, field, setField, fieldRef, choose, submitField, later, shortcutsOpen, setShortcutsOpen, confirm } = state;
  const [pushTarget, setPushTarget] = useState<string | null>(null);
  const reviewGoal = d?.kind === 'approve_review' && d.project_id && d.goal_id ? [d.project_id, d.goal_id] as const : null;
  useEffect(() => {
    setPushTarget(null);
    if (!reviewGoal) return;
    let cancelled = false;
    decisionsClient.dispatchEvidence(reviewGoal[0], reviewGoal[1])
      .then(ev => { if (!cancelled) setPushTarget(ev?.push_target ?? null); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [reviewGoal?.[0], reviewGoal?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!d || !plan) return null;
  return (
    <DecisionActionBar
      plan={plan}
      fieldAction={fieldAction}
      fieldText={field?.text ?? ''}
      fieldRef={fieldRef}
      onFieldText={text => setField(f => (f ? { ...f, text } : f))}
      onCloseField={() => setField(null)}
      onSubmitField={submitField}
      onAction={action => choose(action)}
      onLater={later}
      shortcutsOpen={shortcutsOpen}
      onToggleShortcuts={() => setShortcutsOpen(o => !o)}
      fieldWarning={fieldAction?.body?.answer === 'reject' ? pushedRejectWarning(d.kind, pushTarget) : null}
      disabled={!!confirm}
    />
  );
}

// ── Body ────────────────────────────────────────────────────────────────

/** Whether a 2-line clamp is actually cutting the text (R3: Tooltip only then). */
function useClamped(text: string) {
  const ref = useRef<HTMLHeadingElement>(null);
  const [clamped, setClamped] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const next = el.scrollHeight - el.clientHeight > 1;
      setClamped(prev => (prev === next ? prev : next));
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [text]);
  return { ref, clamped };
}

export function DecisionReview({ inbox, agentName, state, active = true }: {
  inbox: ReturnType<typeof useDecisions>;
  agentName: string;
  state: DecisionReviewState;
  active?: boolean;
}) {
  const { colors } = useTheme();
  const { queue, d, plan, complete, report, confirm, shortcutsOpen, evidenceGoal } = state;
  const host = useRef<HTMLDivElement>(null);
  const goalDetail = useCommandCenter(s => s.goalDetail);

  const handleKey = (event: KeyboardEvent) => {
    if (!active || !complete || report || confirm || (evidenceGoal && goalDetail)) return;
    const command = commandForKey(event);
    if (!command) return;
    // Undo still works once the last decision has been answered.
    if (command.type === 'undo') { event.preventDefault(); state.undo(); return; }
    if (!d || !plan) return;
    if (shortcutsOpen && command.type !== 'shortcuts') state.setShortcutsOpen(false);
    switch (command.type) {
      case 'verdict': {
        const action = plan.actions.find(a => a.key === command.key);
        if (!action) return;
        event.preventDefault();
        state.choose(action);
        return;
      }
      case 'default': {
        const action = plan.actions.find(a => a.returnDefault);
        if (!action) return;
        event.preventDefault();
        state.choose(action);
        return;
      }
      case 'later': event.preventDefault(); state.later(); return;
      case 'next': event.preventDefault(); queue.next(); return;
      case 'prev': event.preventDefault(); queue.prev(); return;
      case 'ask': event.preventDefault(); state.askRef.current?.focus(); return;
      case 'shortcuts': event.preventDefault(); state.setShortcutsOpen(o => !o); return;
    }
  };
  const keyRef = useRef(handleKey);
  keyRef.current = handleKey;
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      const element = host.current;
      if (!element) return;
      // The whole panel, not just the body: focus lands on a header control
      // when the sheet opens, and the action bar lives in the footer.
      const scope = element.closest('[role="dialog"]');
      const target = event.target;
      const inScope = target instanceof Node && !!scope?.contains(target);
      const unfocused = target === document.body || target === document.documentElement;
      if (inScope || unfocused) keyRef.current(event);
    };
    document.addEventListener('keydown', listener);
    return () => document.removeEventListener('keydown', listener);
  }, []);

  if (!active) return null;
  const position = complete ? queue.position : null;
  return (
    // tabIndex -1 + no outline: a click on plain text focuses THIS box rather
    // than the shell panel, which would otherwise draw the global keyboard
    // ring round the whole sheet the moment J/K are pressed. Not a Tab stop.
    <div ref={host} data-testid="decision-review" tabIndex={-1} style={{ color: colors.text, fontFamily: font.body, minWidth: 0, outline: 'none' }}>
      <div role="status" aria-live="polite" className="sr-only">
        {position ? `Decision ${position.index + 1} of ${position.total}. ` : ''}{state.announcement}
      </div>
      {!complete && <LoadingShape onRetry={() => { void inbox.showAll(); }} />}
      {complete && d && plan && (report
        ? <CouncilReport state={state} />
        : <DecisionBody key={d.id} d={d} agentName={agentName} state={state} />)}
      {confirm && (
        <ConfirmDialog
          title={confirm.action.confirm?.title ?? confirm.action.label}
          consequence={confirm.action.confirm?.consequence ?? confirm.action.effect}
          confirmLabel={confirm.action.confirm?.confirmLabel ?? confirm.action.label.replace(/…$/, '')}
          cancelLabel="Keep it"
          failureLabel="That didn't go through"
          onConfirm={state.runConfirm}
          onCancel={() => state.setConfirm(null)}
        />
      )}
    </div>
  );
}

/** R2: the loading state is the final layout, shimmering. */
export function LoadingShape({ onRetry }: { onRetry?: () => void }) {
  const { colors } = useTheme();
  return (
    <div data-testid="decision-loading" style={{ display: 'flex', flexDirection: 'column', gap: space.huge }}>
      <span className="sr-only" role="status">Loading all pending decisions…</span>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
        <Skeleton width={180} height={type.label.fontSize} />
        <Skeleton width="85%" height={type.title.fontSize} />
        <SkeletonText lines={2} lastLineWidth="55%" />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
        <Skeleton width={110} height={type.label.fontSize} />
        <SkeletonText lines={3} lastLineWidth="70%" />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
        <Skeleton width={90} height={type.label.fontSize} />
        <Skeleton width="45%" />
      </div>
      {onRetry && (
        <div style={{ ...type.caption, color: colors.textDim }}>
          Still loading?{' '}
          <Button colors={colors} variant="bare" flashSuccess={false} onClick={onRetry} style={quietBare(colors)}>Retry</Button>
        </div>
      )}
    </div>
  );
}

function CouncilReport({ state }: { state: DecisionReviewState }) {
  const { colors } = useTheme();
  const report = state.report!;
  return (
    <section aria-label="Council report" className="pa-enter" style={{ display: 'flex', flexDirection: 'column', gap: space.xl }}>
      <div>
        <Button colors={colors} variant="bare" onClick={state.closeReport} style={quietBare(colors)}>← Back to decision</Button>
      </div>
      {report.error ? (
        <div role="alert" style={{ display: 'flex', gap: space.md, alignItems: 'center', ...type.small, color: colors.danger }}>
          {report.error}
          <Button colors={colors} variant="ghost" onClick={() => state.openReport(report.id)}>Retry</Button>
        </div>
      ) : !report.data ? (
        <SkeletonText lines={4} />
      ) : report.data.report ? (
        <>
          <h3 style={{ margin: 0, ...type.heading, fontFamily: font.display }}>{report.data.report.headline}</h3>
          <p style={{ margin: 0, ...type.body, color: colors.textMuted, whiteSpace: 'pre-wrap' }}>{report.data.report.markdown}</p>
        </>
      ) : (
        <p style={{ margin: 0, ...type.small, color: colors.textMuted }}>No report exists for this Council session.</p>
      )}
    </section>
  );
}

function DecisionBody({ d, agentName, state }: { d: Decision; agentName: string; state: DecisionReviewState }) {
  const { colors, reduceMotion } = useTheme();
  const [techOpen, setTechOpen] = useState(false);
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const goalDetail = useCommandCenter(s => s.goalDetail);
  const closeGoalDetail = useCommandCenter(s => s.closeGoalDetail);
  const plan = state.plan!;
  const title = decisionTitle(d);
  const { ref: titleRef, clamped } = useClamped(title);
  const target = evidenceTargetFor(d);
  const suggestion = recommendedLabel(d);
  const check = checkApprovalOf(d);
  const toolArgs = toolArgumentsText(d);
  const technical = technicalDetails(d);
  const staged = d.staged_answer ?? null;
  const stagedAction = staged ? plan.actions.find(a => a.body?.answer === staged.answer && a.role !== 'danger') : null;
  const intelItems = d.kind === 'project_intel_proposal' && Array.isArray(d.payload?.items)
    ? (d.payload!.items as unknown[]).filter((item): item is Record<string, unknown> => typeof item === 'object' && item !== null)
    : [];
  const leadFacts: string[] = [];
  const spent = typeof d.payload?.spent === 'number' ? d.payload.spent : null;
  if (unblockReason(d) === 'wallclock_cap' && spent !== null) leadFacts.push(`${humanDuration(spent)} used`);
  if (unblockReason(d) === 'attempt_cap' && spent !== null) leadFacts.push(`${spent} attempt${spent === 1 ? '' : 's'}`);

  const hasEvidence = target.kind === 'dispatch' || target.kind === 'council_report' || target.kind === 'budget'
    || !!check || !!toolArgs || intelItems.length > 0;
  const titleEl = (
    <h2
      ref={titleRef}
      tabIndex={clamped ? -1 : undefined}
      style={{
        margin: 0, ...type.title, fontFamily: font.display, color: colors.text,
        display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden',
        overflowWrap: 'anywhere',
      } as CSSProperties}
    >
      {title}
    </h2>
  );
  const pre: CSSProperties = {
    margin: 0, borderRadius: concentric(radius.lg, REVIEW_PAD_X), background: colors.codeBg, color: colors.codeText,
    padding: `${space.lg}px ${space.xl}px`, fontFamily: font.mono, fontSize: textSize.caption, lineHeight: '18px',
    maxHeight: 240, overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
  };

  return (
    <article data-testid={`decision-${d.id}`} className="pa-enter" style={{ display: 'flex', flexDirection: 'column', gap: space.huge, minWidth: 0 }}>
      {/* ②–④ The question */}
      <header style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xl, minHeight: type.label.fontSize + space.md }}>
          <SectionLabel>
            {kindLabel(d)}
            {' · '}
            <Tooltip content={absoluteTime(d.created_at)}>
              <span tabIndex={0} style={{ outline: 'none' }}>{ageLabel(d.created_at)}</span>
            </Tooltip>
          </SectionLabel>
          {suggestion && (
            <Chip kind="static" tone="accent" title={`${agentName}'s recommended option`}>
              {`${agentName} suggests: ${suggestion}`}
            </Chip>
          )}
        </div>
        {clamped ? <Tooltip content={title}>{titleEl}</Tooltip> : titleEl}
        <p data-testid="decision-cause" style={{ margin: 0, ...type.body, color: colors.textMuted, maxWidth: '64ch' }}>
          {plainCause(d, agentName)}
        </p>
      </header>

      {staged && stagedAction && (
        <div data-testid={`staged-${d.id}`} style={{ display: 'flex', alignItems: 'center', gap: space.md, flexWrap: 'wrap', ...type.small, color: colors.textMuted }}>
          <span>
            <strong style={{ color: colors.text, fontWeight: 600 }}>{stagedSummary(staged)}</strong>
            {` · ${ageLabel(staged.staged_at)}. Nothing has happened yet — press ${stagedAction.label} to commit it.`}
            {staged.note ? ` Said with it: ${staged.note}` : ''}
          </span>
          <Button colors={colors} variant="bare" onClick={async () => { await state.discardStaged(d.id); }} style={quietBare(colors)}>
            Discard
          </Button>
        </div>
      )}

      {/* ⑤ Consequences — the same record the buttons read */}
      <section aria-label="If you choose" style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
        <SectionLabel>If you choose</SectionLabel>
        <div data-testid="effect-rows" style={{ display: 'grid', gridTemplateColumns: 'minmax(max-content, 160px) minmax(0, 1fr)', columnGap: space.xl, rowGap: space.sm, ...type.small }}>
          {effectRows(plan).map(row => (
            <div key={row.label} style={{ display: 'contents' }}>
              <span style={{ fontWeight: 600, color: colors.text }}>{row.label}</span>
              <span style={{ color: colors.textMuted }}>{row.effect}</span>
            </div>
          ))}
        </div>
      </section>

      {/* ⑥ Evidence */}
      {hasEvidence && (
        <section aria-label="Evidence" style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md }}>
            <SectionLabel>Evidence</SectionLabel>
            {target.kind === 'dispatch' && (
              <Button colors={colors} variant="ghost" onClick={() => { state.setEvidenceGoal(true); openGoalDetail(target.projectId, target.goalId); }}>
                {target.label}
              </Button>
            )}
            {target.kind === 'council_report' && (
              <Button colors={colors} variant="ghost" onClick={() => state.openReport(target.councilSessionId)}>{target.label}</Button>
            )}
          </div>
          {state.evidenceGoal && goalDetail && (
            <Button colors={colors} variant="bare" onClick={() => { closeGoalDetail(); state.setEvidenceGoal(false); }}>← Back to decision</Button>
          )}
          {target.kind === 'dispatch' && (
            <EvidenceDigest compact projectId={target.projectId} goalId={target.goalId} leadFacts={leadFacts} />
          )}
          {target.kind === 'budget' && <div style={{ ...type.small, color: colors.textMuted }}>{target.detail}</div>}
          {check && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
              <pre style={pre}>{check.command}</pre>
              <div style={{ ...type.caption, color: colors.textMuted }}>
                in <span style={{ fontFamily: font.mono, overflowWrap: 'anywhere' }}>{check.cwd}</span>
              </div>
              {check.reason && <div style={{ ...type.small, color: colors.textMuted }}>{check.reason}</div>}
            </div>
          )}
          {toolArgs && <pre aria-label="Tool arguments" style={pre}>{toolArgs}</pre>}
          {intelItems.map((item, index) => (
            <div key={index} style={{ ...type.small, color: colors.textMuted }}>
              <span style={{ color: colors.text, fontWeight: 600 }}>{typeof item.name === 'string' ? item.name : 'Unnamed'}</span>
              {typeof item.note === 'string' ? ` — ${item.note}` : ''}
            </div>
          ))}
        </section>
      )}

      {/* ⑦ Technical details — level 2 */}
      {(technical.detail || technical.ids.length > 0 || technical.lastError) && (
        <details data-testid="technical-details" onToggle={e => setTechOpen(e.currentTarget.open)} style={{ ...type.small }}>
          <summary
            className="pa-btn"
            style={{
              listStyle: 'none', justifyContent: 'flex-start', gap: space.sm,
              '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.text,
              '--pa-btn-bg-hover': colors.fillHover, '--pa-btn-bg-active': colors.fillActive,
              '--pa-btn-pad': `${space.xs}px ${space.sm}px`, marginLeft: -space.sm,
              fontSize: textSize.small, fontFamily: font.body,
            } as CSSProperties}
          >
            <span aria-hidden="true" style={{ display: 'inline-block', transform: techOpen ? 'rotate(90deg)' : 'none', transition: reduceMotion ? 'none' : `transform ${duration.snappy}ms ${ease.snappy}` }}>▸</span>
            Technical details
          </summary>
          <div style={{ ...pre, maxHeight: 'none', marginTop: space.md, color: colors.codeText, display: 'flex', flexDirection: 'column', gap: space.md }}>
            {technical.detail && <div>{technical.detail}</div>}
            {technical.lastError && <div>Last error: {technical.lastError}</div>}
            {technical.ids.length > 0 && (
              <div style={{ display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: space.xl, color: colors.textMuted }}>
                {technical.ids.map(id => (
                  <div key={`${id.label}:${id.value}`} style={{ display: 'contents' }}>
                    <span>{id.label}</span>
                    <span style={{ color: colors.codeText, overflowWrap: 'anywhere' }}>{id.value}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </details>
      )}

      {/* ⑧ Ask Henry */}
      <DiscussionPane decisionId={d.id} headline={d.headline} agentName={agentName} inputRef={state.askRef} />
    </article>
  );
}
