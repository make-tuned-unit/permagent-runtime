/**
 * Today's Focus — Morning Briefing's centerpiece (HOME_TEMPLATES_RESEARCH.md
 * §2.1, §3 `today_focus`). Deterministic ranking on the daemon (age of the
 * oldest open decision, then the longest-held stalled goal) — this card only
 * renders the answer and explains why (R5 / HAX G11), never picks anything
 * itself.
 */
import { radius, space, font, textSize, duration, ease } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { useDashboardInsights } from './useDashboardInsights';
import { useState } from 'react';

export function TodayFocusCard() {
  const { colors, reduceMotion } = useTheme();
  const { data, loading, error, refresh } = useDashboardInsights();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const inbox = useDecisions();
  const [showInbox, setShowInbox] = useState(false);
  const [hover, setHover] = useState(false);
  const [pressed, setPressed] = useState(false);

  const focus = data?.todayFocus;
  const hasFocus = Boolean(focus?.kind);

  const open = () => {
    if (!focus?.kind) return;
    if (focus.kind === 'decision') setShowInbox(true);
    else if (focus.kind === 'goal' && focus.projectId && focus.refId) {
      openGoalDetail(focus.projectId, focus.refId);
    }
  };

  return (
    <div style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: space.xxl,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <SectionTitle title="Today's Focus" />

      {error ? (
        <EmptyNote hint="Couldn't reach the daemon">Couldn't load today's focus</EmptyNote>
      ) : loading && !data ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : !hasFocus ? (
        // §3 honest-data rule: never force a pick. Empty inbox and no stalled
        // goals is a genuinely good state, said as one.
        <EmptyNote hint="Nothing is waiting on you right now">
          Nothing urgent — pick anything
        </EmptyNote>
      ) : (
        <button
          type="button"
          onClick={open}
          onMouseEnter={() => setHover(true)}
          onMouseLeave={() => { setHover(false); setPressed(false); }}
          onPointerDown={() => setPressed(true)}
          onPointerUp={() => setPressed(false)}
          style={{
            flex: 1, minHeight: 0, textAlign: 'left', cursor: 'pointer',
            display: 'flex', flexDirection: 'column', justifyContent: 'center',
            gap: space.md, border: 'none', borderRadius: radius.md,
            padding: space.lg,
            background: pressed ? colors.fillActive : hover ? colors.fillHover : 'transparent',
            transition: reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}`,
          }}
        >
          <div style={{
            fontFamily: font.display, fontSize: textSize.heading, fontWeight: 600,
            color: colors.text, lineHeight: 1.3,
            display: '-webkit-box', WebkitLineClamp: 3, WebkitBoxOrient: 'vertical',
            overflow: 'hidden',
          }}>
            {focus!.title}
          </div>
          {focus!.reason && (
            <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.4 }}>
              {focus!.reason}
            </div>
          )}
          <div style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, color: colors.cyan, marginTop: 'auto' }}>
            {focus!.kind === 'decision' ? 'Open in the Decision Inbox →' : 'Open goal detail →'}
          </div>
        </button>
      )}

      {showInbox && <DecisionInbox inbox={inbox} onClose={() => { setShowInbox(false); refresh(); }} />}
    </div>
  );
}
