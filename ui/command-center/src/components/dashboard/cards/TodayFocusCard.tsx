/**
 * Today's Focus — Morning Briefing's centerpiece (HOME_TEMPLATES_RESEARCH.md
 * §2.1, §3 `today_focus`). Deterministic ranking on the daemon (open
 * decisions oldest-first, then stalled goals oldest-first, annotated with
 * shared-blocker size) — this card only renders the answer and explains why
 * (R5 / HAX G11), never picks anything itself.
 *
 * Distinct from "Needs You" (the decisions queue/count): this card adds the
 * ranking's WHY plus the next 2 runners-up, so it is not a second rendering
 * of the same single fact (coordinator, Phase-1 presentation pass).
 */
import { radius, space, font, textSize, duration, ease } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { useDashboardInsights, type TodayFocusItem } from './useDashboardInsights';
import { useState } from 'react';

export function TodayFocusCard() {
  const { colors } = useTheme();
  const { data, loading, error, refresh } = useDashboardInsights();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const inbox = useDecisions();
  const [showInbox, setShowInbox] = useState(false);

  const focus = data?.todayFocus;
  const top = focus?.top ?? null;

  const openItem = (item: TodayFocusItem) => {
    if (item.kind === 'decision') setShowInbox(true);
    else if (item.kind === 'goal' && item.projectId) openGoalDetail(item.projectId, item.refId);
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
      ) : !top ? (
        // §3 honest-data rule: never force a pick. Empty inbox and no stalled
        // goals is a genuinely good state, said as one.
        <EmptyNote hint="Nothing is waiting on you right now">
          Nothing urgent — pick anything
        </EmptyNote>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md, minHeight: 0 }}>
          <FocusRow item={top} primary onOpen={openItem} />
          {focus!.runnersUp.length > 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.xs }}>
              <div style={{
                fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em',
                textTransform: 'uppercase', color: colors.textDim,
              }}>
                Next up
              </div>
              {focus!.runnersUp.map(item => (
                <FocusRow key={item.refId} item={item} onOpen={openItem} />
              ))}
            </div>
          )}
        </div>
      )}

      {showInbox && <DecisionInbox inbox={inbox} onClose={() => { setShowInbox(false); refresh(); }} />}
    </div>
  );
}

function FocusRow({ item, primary, onOpen }: { item: TodayFocusItem; primary?: boolean; onOpen: (item: TodayFocusItem) => void }) {
  const { colors, reduceMotion } = useTheme();
  const [hover, setHover] = useState(false);
  const [pressed, setPressed] = useState(false);
  return (
    <button
      type="button"
      onClick={() => onOpen(item)}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => { setHover(false); setPressed(false); }}
      onPointerDown={() => setPressed(true)}
      onPointerUp={() => setPressed(false)}
      style={{
        textAlign: 'left', cursor: 'pointer',
        display: 'flex', flexDirection: 'column',
        gap: space.xxs, border: 'none', borderRadius: radius.md,
        padding: space.lg,
        background: pressed ? colors.fillActive : hover ? colors.fillHover : 'transparent',
        transition: reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}`,
      }}
    >
      <div style={{
        fontFamily: font.display, fontSize: primary ? textSize.heading : textSize.small, fontWeight: 600,
        color: colors.text, lineHeight: 1.3,
        display: '-webkit-box', WebkitLineClamp: primary ? 3 : 2, WebkitBoxOrient: 'vertical',
        overflow: 'hidden',
      }}>
        {item.title}
      </div>
      <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.4 }}>
        {item.reason}
      </div>
      {primary && (
        <div style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, color: colors.cyan, marginTop: space.xxs }}>
          {item.kind === 'decision' ? 'Open in the Decision Inbox →' : 'Open goal detail →'}
        </div>
      )}
    </button>
  );
}
