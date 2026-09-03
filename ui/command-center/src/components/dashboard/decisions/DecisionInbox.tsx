/**
 * Decision Inbox — overlay (Lane L4).
 *
 * Rendered from inside dashboard/ so the workspaces/ mount point stays
 * untouched, on `DetailModal` — the app's one modal shell — rather than on a
 * copy of it. Ranked Tier-2 + unblock items, "+M more" overflow, collapsed
 * Tier-1 group, history view.
 * Zero batch/multi-select affordances anywhere — answers are one at a time.
 */

import { useState, useCallback, useId, type CSSProperties } from 'react';
import { useLiveGoals } from '../../../lib/useLiveGoals';
import { font, radius, ease, duration, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { DetailModal } from '../../common/DetailModal';
import { StateBlock } from '../../common/StateBlock';
import type { useDecisions } from './useDecisions';
import type { HistoryItem } from './types';
import { resolutionText, deadLetterText } from './types';
import { DecisionItem } from './DecisionItem';
import { decisionsClient } from './client';
import { formatAge, withAlpha } from './format';
import { usePersona } from '../../settings/useSettings';

import { Tooltip } from '../../common/Tooltip';
interface Props {
  inbox: ReturnType<typeof useDecisions>;
  onClose: () => void;
}

export function DecisionInbox({ inbox, onClose }: Props) {
  const { colors, reduceMotion } = useTheme();
  const { data, loading, error } = inbox;
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'your agent';
  const [view, setView] = useState<'list' | 'history'>('list');
  const [history, setHistory] = useState<HistoryItem[] | null>(null);
  /** The history read failed. It used to be `catch { setHistory([]) }`, which
   *  moved the failure into the empty branch by hand: a daemon that never
   *  answered rendered as "Nothing here yet" — i.e. as an audit trail saying
   *  nothing had ever been decided. */
  const [historyError, setHistoryError] = useState(false);
  const [tier1Open, setTier1Open] = useState(false);
  const tier1Id = useId();

  const loadHistory = useCallback(async () => {
    setHistoryError(false);
    try {
      setHistory(await inbox.loadHistory());
    } catch { setHistoryError(true); }
  }, [inbox]);

  const openHistory = useCallback(async () => {
    setView('history');
    if (history === null) {
      try {
        setHistory(await inbox.loadHistory());
        setHistoryError(false);
      } catch { setHistoryError(true); }
    }
  }, [history, inbox]);

  const toggleTier1 = useCallback(async () => {
    const next = !tier1Open;
    setTier1Open(next);
    if (next && history === null) {
      try {
        setHistory(await inbox.loadHistory());
        setHistoryError(false);
      } catch { setHistoryError(true); }
    }
  }, [tier1Open, history, inbox]);

  const decisions = data?.decisions ?? [];
  const total = data?.total_pending ?? 0;
  const handled = data?.handled_count ?? 0;
  const attentionGoals = data?.attention_goals ?? [];
  // #464/#515: the live shared source (event-driven, parked/archived excluded)
  // — the summary number is only a fallback and now shares its definition.
  const { activeCount: liveGoalCount, loaded: liveGoalsLoaded } = useLiveGoals();
  const goals = liveGoalsLoaded ? liveGoalCount : (data?.goals_in_flight ?? 0);
  const moreCount = total - decisions.length;
  const tier1Rows = (history ?? []).filter(d => d.tier === 1);

  return (
    // The chrome is `DetailModal`'s. It used to be a byte-for-byte copy of that
    // shell — the same `radius.lg`, the same shadow pair, the same 86vh — with
    // none of its keyboard floor: no focus trap, no Escape, no `role="dialog"`,
    // and focus abandoned wherever it was when the inbox opened. On the surface
    // that exists to be checked several times a day.
    <DetailModal
      title={view === 'history' ? 'History' : 'Decision inbox'}
      onClose={onClose}
      width="min(720px, 92vw)"
      badge={view === 'list' && total > 0
        ? { label: `${total} pending`, color: colors.cyan, bg: colors.cyanSoft }
        : null}
      headerLeft={view === 'history' ? (
        <Button
          colors={colors}
          variant="bare"
          type="button"
          onClick={() => setView('list')}
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': colors.border,
            '--pa-btn-pad': '2px 4px',
            '--pa-btn-radius': `${radius.xs}px`,
            '--pa-btn-weight': 400,
            fontFamily: font.body, fontSize: textSize.caption,
          } as CSSProperties}
        >
          ← Back
        </Button>
      ) : undefined}
      bodyStyle={{ padding: '6px 0' }}
      footer={view === 'list' ? (
        <Button
          colors={colors}
          variant="bare"
          type="button"
          className="hover:underline"
          onClick={openHistory}
          style={{
            '--pa-btn-fg': colors.textDim,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-bg-active': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-radius': '0',
            '--pa-btn-weight': 400,
            fontSize: textSize.caption, fontFamily: font.body,
          } as CSSProperties}
        >
          History →
        </Button>
      ) : undefined}
    >
      <div data-testid="inbox-body">
          {view === 'history' ? (
            <HistoryList items={history} failed={historyError} onRetry={loadHistory} />
          ) : loading && !data ? (
            <div style={{ padding: '48px 18px', textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
              Checking with {agentName}…
            </div>
          ) : error && !data ? (
            /* Cold load failure — must NOT read as the "No decisions needed"
               all-clear (2026-07 wiring audit D4): a dead daemon looked
               identical to a clear inbox. */
            <div style={{ padding: '48px 18px', textAlign: 'center' }}>
              <div style={{ fontSize: textSize.small, color: colors.textMuted, marginBottom: space.xs }}>
                Couldn't reach the decision inbox.
              </div>
              <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: 14 }}>
                This is a connection problem, not an empty inbox.
              </div>
              {/* No success tick: `refresh` swallows its own failure, so a
                  tick would claim a load that never landed. The list replacing
                  this dead end is the only honest confirmation. */}
              <Button
                colors={colors}
                type="button"
                flashSuccess={false}
                onClick={() => inbox.refresh()}
                style={{
                  '--pa-btn-bg': 'transparent',
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-border': colors.borderHi,
                  '--pa-btn-bg-hover': colors.cyanSoft,
                  '--pa-btn-fg-hover': colors.cyan,
                  '--pa-btn-border-hover': colors.cyan,
                  '--pa-btn-bg-active': colors.cyanGlow,
                  '--pa-btn-pad': '5px 14px',
                  '--pa-btn-radius': `${radius.md}px`,
                  '--pa-btn-weight': 600,
                  fontFamily: font.body, fontSize: textSize.caption,
                } as CSSProperties}
              >
                Retry
              </Button>
            </div>
          ) : total === 0 && attentionGoals.length === 0 ? (
            <div style={{ padding: '48px 18px', textAlign: 'center' }}>
              <div style={{
                width: 34, height: 34, borderRadius: '50%',
                background: withAlpha(colors.success, 0.15), color: colors.success,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: textSize.heading, margin: '0 auto 10px',
              }}>✓</div>
              <div style={{ fontSize: textSize.small, color: colors.textMuted, marginBottom: space.xs }}>No decisions needed.</div>
              <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
                {goals} goal{goals === 1 ? '' : 's'} in flight.
              </div>
            </div>
          ) : (
            <>
              {/* Parked goals — needs_human_attention finally MEANS attention
                  (wave-1 item 1): before this bucket the flag only hid the
                  goal from every active list. */}
              {attentionGoals.length > 0 && (
                <div style={{ padding: `${space.lg}px 18px ${space.xs}px` }}>
                  <div style={{
                    fontFamily: font.body, fontSize: 10, fontWeight: 700,
                    letterSpacing: '0.10em', textTransform: 'uppercase',
                    // "Needs attention" is the warning semantic (D8), not a
                    // bespoke amber — the dashboard's DecisionsCard made the
                    // same call for the same bucket (one concept, one color).
                    color: colors.warning, marginBottom: space.sm,
                  }}>
                    Parked goals — waiting on you
                  </div>
                  {attentionGoals.map(g => (
                    <div
                      key={g.id}
                      style={{
                        display: 'flex', alignItems: 'baseline', gap: space.md,
                        padding: '6px 0',
                        borderBottom: `1px solid ${colors.border}`,
                      }}
                    >
                      <span style={{ fontSize: textSize.caption, color: colors.text, flex: 1 }}>{g.title}</span>
                      <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim }}>
                        {g.state_binding}
                      </span>
                      {g.reason && (
                        <Tooltip content={g.reason}>
                          <span tabIndex={0} style={{ outline: 'none' }}>
                            <span style={{ fontSize: textSize.micro, color: colors.textMuted, maxWidth: 260, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                              {g.reason}
                            </span>
                          </span>
                        </Tooltip>
                      )}
                    </div>
                  ))}
                </div>
              )}
              {decisions.map(d => (
                <DecisionItem
                  key={d.id}
                  decision={d}
                  onAnswer={inbox.answer}
                  onConflictSettled={inbox.refresh}
                  onDiscardStaged={inbox.discardStaged}
                  onCancelGoal={
                    d.goal_id && d.project_id
                      ? async () => {
                          await decisionsClient.cancelGoal(d.project_id!, d.goal_id!);
                          inbox.refresh();
                        }
                      : undefined
                  }
                />
              ))}

              {moreCount > 0 && (
                /* No success tick: `showAll` swallows its own failure, and the
                   rest of the list appearing is the confirmation. */
                <Button
                  colors={colors}
                  variant="bare"
                  type="button"
                  flashSuccess={false}
                  onClick={() => { inbox.showAll(); }}
                  style={{
                    '--pa-btn-fg': colors.cyan,
                    '--pa-btn-fg-hover': colors.cyan,
                    '--pa-btn-bg-hover': colors.cyanSoft,
                    '--pa-btn-bg-active': colors.cyanSoft,
                    '--pa-btn-pad': '10px',
                    '--pa-btn-radius': '0',
                    display: 'flex', width: '100%',
                    fontFamily: font.body, fontSize: textSize.caption,
                  } as CSSProperties}
                >
                  +{moreCount} more decision{moreCount === 1 ? '' : 's'}
                </Button>
              )}

              {handled > 0 && (
                <>
                  {/* Disclosure toggle for the routine-items group below, with
                      its own three-part row (dot, label, chevron) laid out by
                      the button itself. It keeps the element and takes the
                      shared `.pa-btn` interaction rules instead of the Button
                      primitive, whose single label span would collapse that
                      row and whose pending/success machinery has nothing to
                      say about opening a list. */}
                  <button
                    type="button"
                    className="pa-btn"
                    aria-expanded={tier1Open}
                    aria-controls={tier1Id}
                    onClick={toggleTier1}
                    style={{
                      '--pa-btn-bg': 'transparent',
                      '--pa-btn-fg': colors.text,
                      '--pa-btn-border': 'transparent',
                      '--pa-btn-bg-hover': colors.cyanSoft,
                      '--pa-btn-fg-hover': colors.text,
                      '--pa-btn-bg-active': colors.cyanSoft,
                      '--pa-btn-pad': '12px 18px',
                      '--pa-btn-radius': '0',
                      gap: space.lg, width: '100%',
                      justifyContent: 'flex-start',
                      borderTop: `1px solid ${colors.border}`,
                      textAlign: 'left', fontFamily: font.body,
                      fontSize: 'inherit', lineHeight: 'inherit',
                    } as CSSProperties}
                  >
                    <span style={{
                      width: 8, height: 8, borderRadius: '50%',
                      background: colors.success, flexShrink: 0,
                    }} />
                    <span style={{ fontSize: textSize.small, fontWeight: 500, color: colors.text, flex: 1 }}>
                      {agentName} handled {handled} routine item{handled === 1 ? '' : 's'} overnight
                    </span>
                    <span style={{
                      color: colors.textDim, fontSize: textSize.micro,
                      transform: tier1Open ? 'rotate(90deg)' : 'none',
                      // D9: a control-state change, so the snappy spring
                      // (240ms) — not the old bare bezier with no duration
                      // token behind it.
                      transition: reduceMotion ? 'none' : `transform ${duration.snappy}ms ${ease.snappy}`,
                    }}>▸</span>
                  </button>
                  {tier1Open && (
                    <div id={tier1Id}>
                      {history === null ? (
                        <div style={{ padding: `${space.lg}px 18px ${space.lg}px 36px`, fontSize: textSize.caption, color: colors.textDim }}>
                          Loading…
                        </div>
                      ) : (
                        tier1Rows.map(row => (
                          <div
                            key={row.id}
                            style={{
                              display: 'flex', alignItems: 'center', gap: space.lg,
                              padding: `${space.md}px 18px ${space.md}px 36px`, fontSize: textSize.caption,
                              color: colors.textMuted,
                              borderTop: `1px solid ${colors.border}`,
                              fontFamily: font.body,
                            }}
                          >
                            <span style={{
                              flex: 1, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                            }}>
                              {row.headline}
                            </span>
                            <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>
                              {formatAge(row.created_at)}
                            </span>
                            <Button
                              colors={colors}
                              variant="bare"
                              type="button"
                              className="hover:underline"
                              onClick={openHistory}
                              style={{
                                '--pa-btn-fg': colors.cyan,
                                '--pa-btn-fg-hover': colors.cyan,
                                '--pa-btn-bg-hover': 'transparent',
                                '--pa-btn-bg-active': 'transparent',
                                '--pa-btn-pad': '0',
                                '--pa-btn-radius': '0',
                                '--pa-btn-weight': 400,
                                fontSize: textSize.micro, fontFamily: font.body,
                                flexShrink: 0,
                              } as CSSProperties}
                            >
                              audit →
                            </Button>
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </>
              )}
            </>
          )}
      </div>
    </DetailModal>
  );
}

/** Read-only audit list: every resolved decision, incl. Tier-1 auto-handled. */
function HistoryList({ items, failed, onRetry }: {
  items: HistoryItem[] | null;
  failed: boolean;
  onRetry: () => void;
}) {
  const { colors } = useTheme();
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'your agent';
  if (failed) {
    return (
      <StateBlock
        tone="error"
        compact
        title="Couldn't load the decision history."
        detail="This is a connection problem, not an empty record — decisions you have already answered are still there."
        onRetry={onRetry}
      />
    );
  }
  if (items === null) {
    return (
      <div style={{ padding: '32px 18px', textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
        Loading…
      </div>
    );
  }
  if (items.length === 0) {
    return <StateBlock tone="empty" compact title="Nothing here yet." />;
  }
  return (
    <div>
      {items.map(item => (
        <div
          key={item.id}
          style={{
            padding: `${space.xl}px 18px`,
            borderBottom: `1px solid ${colors.border}`,
            fontFamily: font.body,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
            <span style={{
              fontFamily: font.mono, fontSize: 10, letterSpacing: '0.06em',
              textTransform: 'uppercase', borderRadius: radius.xs, padding: `${space.xxs}px ${space.sm}px`,
              flexShrink: 0,
              color: item.tier === 1 ? colors.success : colors.cyan,
              background: item.tier === 1 ? withAlpha(colors.success, 0.15) : colors.cyanSoft,
            }}>
              {item.tier === 1 ? agentName : 'you'}
            </span>
            <span style={{
              fontSize: textSize.small, fontWeight: 500, color: colors.text, flex: 1,
              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
            }}>
              {item.headline}
            </span>
            <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>
              {formatAge(item.created_at)}
            </span>
          </div>
          <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: space.xs }}>
            {resolutionText(item, agentName)}
          </div>
          {/* CASE A fix #4: the answer succeeded but the gated effect never
              took effect after every retry — the decisions row alone reads
              identically to a decision whose effect actually ran, so this is
              the one place that says otherwise. */}
          {deadLetterText(item) && (
            <div
              role="alert"
              style={{
                marginTop: space.sm, fontSize: textSize.caption, color: colors.danger,
                borderRadius: radius.md, border: `1px solid ${colors.danger}`,
                background: withAlpha(colors.danger, 0.08), padding: `${space.sm}px ${space.lg}px`,
              }}
            >
              {deadLetterText(item)}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
