/**
 * Overnight Digest — Morning Briefing's ambient tail (§2.1, §3
 * `overnight_digest`). A real digest: finished sessions, newly-stalled goals
 * and new decisions since midnight, each a named line (1-2 examples plus a
 * count), clickable — not just a bare number (coordinator, Phase-1
 * presentation pass). A quiet night is said as one, not rendered as blank
 * rows (P8 calm tech).
 */
import { radius, space, font, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { useDashboardInsights, type DigestItem } from './useDashboardInsights';
import { useCallback, useState } from 'react';

export function OvernightDigestCard() {
  const { colors } = useTheme();
  const { data, loading, error, refresh } = useDashboardInsights();
  const digest = data?.overnightDigest;
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const openChatDock = useCommandCenter(s => s.openChatDock);
  const inbox = useDecisions();
  const [showInbox, setShowInbox] = useState(false);

  const openSession = useCallback((id: string) => {
    switchToSession(id).catch(err => console.error('[overnight] open session failed:', err));
    openChatDock();
  }, [switchToSession, openChatDock]);

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
      <SectionTitle title="Overnight" />
      {error ? (
        <EmptyNote hint="Couldn't reach the daemon">Couldn't load the overnight digest</EmptyNote>
      ) : loading && !data ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : !digest || digest.quiet ? (
        <EmptyNote>Quiet night</EmptyNote>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          <DigestGroup label="Finished" items={digest.finished} colors={colors} onOpen={i => openSession(i.id)} />
          <DigestGroup label="Newly stalled" items={digest.stalledNew} colors={colors} onOpen={i => i.projectId && openGoalDetail(i.projectId, i.id)} />
          <DigestGroup label="New decisions" items={digest.newDecisions} colors={colors} onOpen={() => setShowInbox(true)} />
        </div>
      )}
      {showInbox && <DecisionInbox inbox={inbox} onClose={() => { setShowInbox(false); refresh(); }} />}
    </div>
  );
}

function DigestGroup({ label, items, colors, onOpen }: {
  label: string; items: DigestItem[]; colors: ReturnType<typeof useTheme>['colors']; onOpen: (item: DigestItem) => void;
}) {
  if (items.length === 0) return null;
  const shown = items.slice(0, 2);
  const more = items.length - shown.length;
  return (
    <div>
      <div style={{
        fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xs,
      }}>
        {label} · {items.length}
      </div>
      {shown.map(item => (
        <button
          key={item.id}
          type="button"
          onClick={() => onOpen(item)}
          style={{
            display: 'block', width: '100%', maxWidth: 480, textAlign: 'left',
            background: 'transparent', border: 'none', padding: `${space.xxs}px 0`,
            cursor: 'pointer', fontFamily: font.body, fontSize: textSize.caption, color: colors.text,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}
        >
          {item.title}
        </button>
      ))}
      {more > 0 && (
        <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim }}>
          +{more} more
        </div>
      )}
    </div>
  );
}
