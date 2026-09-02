/**
 * The Actions lens — what to DO about the data. The Analytics lens answers
 * "what happened"; this answers "so what".
 *
 * Split out of GrowView.tsx (R9), unchanged. It owns the board's fetches, the
 * server-side `generating` flag and the category tab strip; the cards, the
 * verify control and the measurement rail are their own files beside it.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { usePollWhenVisible } from '../../lib/usePollWhenVisible';
import { useToolOnScreen } from '../../lib/useToolOnScreen';
import { FiLoader } from 'react-icons/fi';
import { Button } from '../common/Button';
import type { Project } from '../projects/types';
import { groupActionsByCategory } from './growActionTabs';
import { SEGMENT_STRIP_PAD, SEGMENT_STRIP_RADIUS, segmentedTab } from './growStyles';
import { SUMMARY_CLASS, growLabel } from './growChrome';
import { CARD_PAD, CARD_R } from './growGeometry';
import { WINDOW_DAYS } from './growthWindows';
import { SkeletonCards } from './GrowStateBlocks';
import { ActionCard } from './ActionCard';
import { GrowthInboxSection } from './GrowthInbox';
import type { GrowthActionsData, GrowthInboxData, LoadState } from './growTypes';
import { GENERATION_POLL_MS, VERDICT_POLL_MS } from './growPolling';

/**
 * Actions — what to DO about the data. The Analytics lens answers "what
 * happened"; this answers "so what".
 *
 * Two sources, deliberately distinguished so the user can weigh them
 * differently: the deterministic growth inbox (computed server-side from real
 * signals, NO model involved) and the agent's read of the analytics. The
 * second is labelled as a model's reading and every item carries the figure it
 * was drawn from, because an ungrounded suggestion that looks like analysis is
 * worse than no suggestion.
 */
export function GrowActions({ project, colors }: { project: Project; colors: ThemeColors }) {
  const [inbox, setInbox] = useState<GrowthInboxData | null>(null);
  const [inboxState, setInboxState] = useState<LoadState>('loading');
  const inboxGen = useRef(0);

  const loadInbox = useCallback((id: string) => {
    const generation = ++inboxGen.current;
    setInboxState('loading');
    apiFetch<GrowthInboxData>(`/api/projects/${encodeURIComponent(id)}/growth-inbox`)
      .then((d) => {
        if (generation !== inboxGen.current) return;
        setInbox(d);
        setInboxState('ready');
      })
      .catch(() => {
        if (generation !== inboxGen.current) return;
        setInbox(null);
        setInboxState('error');
      });
  }, []);

  const [actions, setActions] = useState<GrowthActionsData | null>(null);
  const [actionsState, setActionsState] = useState<LoadState>('loading');
  const actionsGen = useRef(0);

  /**
   * The review is running ON THE SERVER.
   *
   * This replaced a `useState(false)` set by the click handler. That flag was
   * component-local, and this component unmounts when the user leaves the tab
   * (`lens === 'actions' && <GrowActions …>`) or switches project — so the flag
   * was destroyed while the review carried on, and coming back showed an idle
   * button over a review that was still running. The result then landed in the
   * database with nothing on screen to say it had.
   *
   * The truth now lives where the work does. Every GET reports it, so a remount
   * reconciles instead of guessing.
   */
  const serverGenerating = actions?.generating ?? false;
  /**
   * The click, before the server has answered.
   *
   * Only bridges the round trip between pressing the button and the POST's
   * reply — the spinner must be on screen from the moment the click is
   * registered, and without this it would appear a request later. It is NOT
   * the source of truth and never outlives the request: if this component
   * unmounts mid-flight, the server's flag is what the next mount reads.
   */
  const [pending, setPending] = useState(false);
  const busyGenerating = pending || serverGenerating;

  /** `silent` keeps the current cards on screen while re-reading. The poll
   *  below runs every few seconds during a review; dropping the board to
   *  skeletons each time would make the panel flash for as long as the review
   *  takes. */
  const loadActions = useCallback((id: string, opts?: { silent?: boolean }) => {
    const generation = ++actionsGen.current;
    if (!opts?.silent) setActionsState('loading');
    apiFetch<GrowthActionsData>(`/api/projects/${encodeURIComponent(id)}/growth-actions`)
      .then((d) => {
        if (generation !== actionsGen.current) return;
        setActions(d);
        setActionsState('ready');
      })
      .catch(() => {
        if (generation !== actionsGen.current) return;
        if (!opts?.silent) setActionsState('error');
      });
  }, []);

  // Regeneration is explicit. It spends a model call, and actions that
  // reshuffle on every render cannot be acted on.
  //
  // The POST now returns as soon as the review has been STARTED — the work runs
  // in a task on the daemon that no longer belongs to this request — so the
  // reply carries the board as it stands with `generating: true`. `pending` is
  // set first and synchronously so the spinner is on screen from the moment the
  // click is registered rather than one round trip later; the server's flag
  // takes over from it as soon as the reply lands.
  const generate = useCallback((id: string) => {
    if (busyGenerating) return;
    setPending(true);
    apiFetch<GrowthActionsData>(
      `/api/projects/${encodeURIComponent(id)}/growth-actions/generate`,
      { method: 'POST' },
    )
      .then((d) => { setActions(d); setActionsState('ready'); })
      .catch(() => setActionsState('error'))
      .finally(() => setPending(false));
  }, [busyGenerating]);

  useEffect(() => {
    loadInbox(project.id);
    loadActions(project.id);
    return () => { ++inboxGen.current; ++actionsGen.current; };
  }, [project.id, loadInbox, loadActions]);

  // While a review is running, keep asking. This is what makes returning to the
  // tab mid-run show it still running and, when it lands, show the new actions
  // with nothing to press: the flag is on the server, so a remount reads it
  // from the GET above and this poll carries it to completion.
  //
  // It runs ONLY while `generating` is true — it is not a background poll of
  // the panel, and it stops the moment the review does.
  useEffect(() => {
    if (!serverGenerating) return;
    const t = setInterval(() => loadActions(project.id, { silent: true }), GENERATION_POLL_MS);
    return () => clearInterval(t);
  }, [serverGenerating, project.id, loadActions]);

  // Multi-client liveness (#629): the daemon emits `project_changed` when a
  // review finishes, which `livenessSync` turns into a `projectsRev` bump. This
  // is the fast path — the poll above is the belt that still works if the
  // socket is down. Skipped on the first render so it does not double the load
  // the mount effect already did.
  const projectsRev = useCommandCenter((st) => st.projectsRev);
  const seenRev = useRef(projectsRev);
  useEffect(() => {
    if (seenRev.current === projectsRev) return;
    seenRev.current = projectsRev;
    loadActions(project.id, { silent: true });
  }, [projectsRev, project.id, loadActions]);

  // The verdicts (R1.4). The nightly sweep that judges the 7/14/28-day windows
  // is the one writer on this board with no event of its own: it writes
  // `growth_action_outcomes` rows and returns, so `projectsRev` never bumps and
  // the fast path above never fires for the single fact this whole measurement
  // loop exists to produce. Until that emitter exists, the honest substitute is
  // a slow poll — and it is a poll with the two gates the law asks for: only
  // while this panel is the surface on screen, and only while nothing faster is
  // already covering it.
  const onScreen = useToolOnScreen('grow');
  usePollWhenVisible(
    () => loadActions(project.id, { silent: true }),
    VERDICT_POLL_MS,
    onScreen && !serverGenerating,
  );

  const hasActions = (actions?.actions?.length ?? 0) > 0;
  const tracking = actions?.tracking ?? [];
  const archived = actions?.archived ?? [];
  const dismissed = actions?.dismissed ?? [];
  const droppedRestated = actions?.droppedAsRestatement ?? 0;
  const droppedNoTarget = actions?.droppedForNoTarget ?? 0;
  const droppedPresent = actions?.droppedAsAlreadyPresent ?? 0;
  const onChanged = useCallback(() => loadActions(project.id), [loadActions, project.id]);

  const actionGroups = groupActionsByCategory(actions?.actions ?? []);
  const [categoryTab, setCategoryTab] = useState<string | null>(null);
  const [focusCategory, setFocusCategory] = useState<string | null>(null);
  const groupKeys = actionGroups.map((g) => g.key).join(',');
  useEffect(() => {
    if (!groupKeys) {
      setCategoryTab(null);
      return;
    }
    const keys = groupKeys.split(',');
    if (!categoryTab || !keys.includes(categoryTab)) {
      setCategoryTab(keys[0]);
    }
  }, [groupKeys, categoryTab]);
  const selectedGroup = actionGroups.find((g) => g.key === categoryTab) ?? actionGroups[0] ?? null;

  return (
    <>
      {/* Deterministic moves — no model in the loop. */}
      <GrowthInboxSection
        colors={colors}
        state={inboxState}
        inbox={inbox}
        onRetry={() => loadInbox(project.id)}
        projectName={project.name}
      />

      <section style={{ marginTop: space.md }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: space.lg, marginBottom: space.lg }}>
          <h3 style={{ ...growLabel(colors), margin: 0 }}>
            From your analytics
          </h3>
          <div style={{ flex: 1 }} />
          {actions?.generatedAt && (
            <span style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono }}>
              {new Date(actions.generatedAt).toLocaleString()}
            </span>
          )}
          {/* Two clicks cannot start two reviews: the button is disabled for as
              long as either half of `busyGenerating` holds, and the daemon
              refuses a second review for a project that already has one running
              (`begin_review`). The disabled attribute is the courtesy; the
              server is the rule. */}
          {/* `pending` is the review the SERVER says is running, not an awaited
              click — `generate` fires and reconciles through the poll. It buys
              the same three things this button hand-rolled: `aria-busy`, the
              dimming, and a MOVING affordance rather than a text swap, since
              "Reviewing…" alone is indistinguishable from a stuck button and
              this one can be on screen for the length of a model call. The
              spinner is now the primitive's own `.pa-spin` element. */}
          <Button
            colors={colors}
            onClick={() => generate(project.id)}
            disabled={busyGenerating}
            pending={busyGenerating}
            style={{
              '--pa-btn-bg': colors.surface,
              '--pa-btn-fg': colors.text,
              '--pa-btn-border': colors.border,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-pad': `${space.sm}px ${space.xl}px`,
              '--pa-btn-radius': `${radius.md}px`,
              fontFamily: font.body, fontSize: textSize.caption, gap: space.sm,
            } as CSSProperties}
          >
            {busyGenerating
              ? 'Reviewing your analytics…'
              : hasActions ? 'Review again' : 'Review my analytics'}
          </Button>
        </div>

        {/* Said out loud, because the one thing the user could not tell before
            was whether anything was still happening. The review runs on the
            daemon, so this is true whether or not this tab is open. */}
        {busyGenerating && (
          <div style={{
            fontSize: textSize.micro, color: colors.textDim, marginBottom: space.lg,
            display: 'flex', alignItems: 'center', gap: space.sm,
          }}>
            <FiLoader size={11} className="pa-spin" aria-hidden />
            <span>
              Your agent is reading the last {actions?.periodDays ?? 30} days. This keeps running
              if you leave the tab — come back and the new actions will be here.
            </span>
          </div>
        )}

        {actionsState === 'loading' && <SkeletonCards colors={colors} count={2} height={92} />}
        {actionsState === 'error' && (
          <div style={{ fontSize: textSize.caption, color: colors.danger }}>Couldn&rsquo;t load actions.</div>
        )}

        {/* An empty list ALWAYS explains itself — silence is indistinguishable
            from breakage, and this panel is allowed to have nothing to say. */}
        {actionsState === 'ready' && !hasActions && !busyGenerating && (
          <div style={{
            fontSize: textSize.caption, color: colors.textMuted, background: colors.bgDeeper,
            border: `1px solid ${colors.border}`, borderRadius: CARD_R, padding: CARD_PAD,
          }}>
            {/* An empty Actions list with a full Tracking list is not "nothing
                to say" — it is "everything you were offered is now being
                measured", and saying the wrong one of those reads as data
                loss. */}
            {tracking.length > 0
              ? 'Nothing is waiting on you. Everything you took on is being measured below.'
              : actions?.reason ?? 'No review yet — run one to see what your data suggests.'}
          </div>
        )}

        {actionGroups.length > 0 && (
          <div
            role="tablist"
            aria-label="Action category"
            style={{
              display: 'flex', gap: SEGMENT_STRIP_PAD, flexWrap: 'wrap',
              background: colors.bgDeeper, borderRadius: SEGMENT_STRIP_RADIUS, padding: SEGMENT_STRIP_PAD,
              marginBottom: space.lg,
            }}
          >
            {actionGroups.map((group) => {
              const selected = selectedGroup?.key === group.key;
              return (
                // role="tab": the element stays, the interaction rules arrive
                // through `.pa-btn` (see the lens tabs above).
                <button
                  key={group.key}
                  className="pa-btn"
                  role="tab"
                  aria-selected={selected}
                  tabIndex={0}
                  onClick={() => setCategoryTab(group.key)}
                  onFocus={() => setFocusCategory(group.key)}
                  onBlur={() => setFocusCategory(null)}
                  style={{
                    ...segmentedTab(colors, selected),
                    boxShadow: focusCategory === group.key ? `0 0 0 2px ${colors.borderHi}` : 'none',
                  }}
                >
                  {group.label} ({group.actions.length})
                </button>
              );
            })}
          </div>
        )}

        <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
          {(selectedGroup?.actions ?? []).map((a, i) => (
            // Identity first: the durable id survives a regeneration that
            // rewords the title, so an in-flight verify stays attached to the
            // card it was started from rather than jumping to a neighbour.
            <ActionCard
              key={a.identity?.id ?? `${a.title}-${i}`}
              project={project}
              action={a}
              colors={colors}
              lane="actions"
              onChanged={onChanged}
              showCategory={false}
            />
          ))}
        </div>

        {/* Completed — what we shipped, and whether it worked.

            Not collapsed and not the archive: this is shipped work with
            verdicts still to come, and the user asked for it precisely so a
            verified action would leave the decision list without leaving
            their sight. #1053 deliberately kept these rows on the active
            board so nothing in flight could silently vanish; that guarantee
            is honoured by MOVING them here, where every one is still
            rendered with its evidence, its prediction, its baseline and its
            windows. The heading says "Completed" because that is what
            happened to the WORK — measurement continuing is a property of
            the verdict, not a reason to still call it undone, which is why
            the subhead carries that nuance instead of the heading. */}
        {tracking.length > 0 && (
          <section style={{ marginTop: space.xxl + space.xs }}>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: space.lg, marginBottom: space.lg }}>
              <h3 style={{ ...growLabel(colors), margin: 0 }}>Completed ({tracking.length})</h3>
              <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
                Shipped — still being measured, at {WINDOW_DAYS.join(', ')} days against the
                traffic before them.
              </span>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
              {tracking.map((a, i) => (
                <ActionCard
                  key={a.identity?.id ?? `tracking-${a.title}-${i}`}
                  project={project}
                  action={a}
                  colors={colors}
                  lane="tracking"
                  onChanged={onChanged}
                />
              ))}
            </div>
          </section>
        )}

        {/* The shelf. Collapsed, because filed work is a record the user goes
            looking for rather than something competing with the board — but
            present, because an archive you cannot open is a delete. */}
        {archived.length > 0 && (
          <details style={{ marginTop: space.xl }}>
            <summary className={SUMMARY_CLASS} style={growLabel(colors)}>Archived ({archived.length})</summary>
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg, marginTop: space.lg }}>
              {archived.map((a, i) => (
                <ActionCard
                  key={a.identity?.id ?? `archived-${a.title}-${i}`}
                  project={project}
                  action={a}
                  colors={colors}
                  lane="shelf"
                  onChanged={onChanged}
                />
              ))}
            </div>
          </details>
        )}

        {/* Advice the user turned down. Its own section rather than the archive
            because the two are opposites to the agent: dismissed text stays on
            the board and can never be proposed again, archived text is
            released. Collapsed for the same reason as the archive — a refusal
            is a record, not work. */}
        {dismissed.length > 0 && (
          <details style={{ marginTop: space.xl }}>
            <summary className={SUMMARY_CLASS} style={growLabel(colors)}>Dismissed ({dismissed.length})</summary>
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg, marginTop: space.lg }}>
              {dismissed.map((a, i) => (
                <ActionCard
                  key={a.identity?.id ?? `dismissed-${a.title}-${i}`}
                  project={project}
                  action={a}
                  colors={colors}
                  lane="shelf"
                  onChanged={onChanged}
                />
              ))}
            </div>
          </details>
        )}

        {hasActions && (
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.lg }}>
            Read from your own analytics by your agent, over the last{' '}
            {actions?.periodDays ?? 30} days. Each item cites the figure it came from — check it
            before acting.
          </div>
        )}

        {/* Both guards can silently withhold advice — the reword guard drops a
            suggestion the user never sees, and an untargeted action is discarded
            outright. Counting them out loud is what keeps that auditable; a
            drop nobody is told about is indistinguishable from a model that had
            less to say. */}
        {(droppedRestated > 0 || droppedNoTarget > 0 || droppedPresent > 0) && (
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.sm }}>
            Last review dropped {droppedRestated + droppedNoTarget + droppedPresent} suggestion(s):{' '}
            {[
              droppedRestated > 0
                ? `${droppedRestated} restated something already on your board`
                : null,
              droppedNoTarget > 0
                ? `${droppedNoTarget} made no measurable prediction`
                : null,
              droppedPresent > 0
                ? `${droppedPresent} already in the repo`
                : null,
            ].filter(Boolean).join(', ')}.
          </div>
        )}
      </section>
    </>
  );
}
