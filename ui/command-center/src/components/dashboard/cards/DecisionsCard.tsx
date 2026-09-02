/**
 * Decision Inbox — home card ("decisions" registry entry, Lane L4).
 *
 * THE canonical rendering of the pending-decision count (J3). Home is the
 * landing page and "what needs me" is a landing-page fact; the other placements
 * (Settings → Autonomy, the chat dock, the World's Petition Basin, the Council
 * card) are labelled references to this one, and they take their words from the
 * shared `summarizeDecisions` rather than writing their own.
 *
 * Glanceable only: count + plain words; the whole card is tappable and opens
 * the inbox overlay. No actions live on the card itself.
 */

import { useEffect, useState } from 'react';
import { font, radius, tabularNums, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { summarizeDecisions } from '../decisions/summary';
import { usePersona } from '../../settings/useSettings';
import { useCommandCenter } from '../../../lib/store';

interface Props {
  /** Active-goal count from the shared useLiveGoals source, so this stat agrees
   *  with the In-Flight card and Hero status. Falls back to the decisions
   *  payload's own count when not supplied (e.g. standalone use). */
  activeCount?: number;
}

export function DecisionsCard({ activeCount }: Props = {}) {
  const { colors, reduceMotion } = useTheme();
  const inbox = useDecisions();
  const { data } = inbox;
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'your agent';
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(false);

  const s = summarizeDecisions(data, agentName, activeCount);
  const { count } = s;
  const empty = s.allClear;

  // A reference elsewhere in the app asked to be brought here (Settings →
  // Autonomy's strip, per J3). Open the canonical surface, then clear the
  // request so it can never fire again later on its own.
  const pendingOpen = useCommandCenter(st => st.pendingDecisionInbox);
  const clearPendingDecisionInbox = useCommandCenter(st => st.clearPendingDecisionInbox);
  useEffect(() => {
    if (!pendingOpen) return;
    setOpen(true);
    clearPendingDecisionInbox();
  }, [pendingOpen, clearPendingDecisionInbox]);

  return (
    <>
      <div
        data-testid="decisions-card"
        role="button"
        tabIndex={0}
        aria-label="Open the decision inbox"
        onClick={() => setOpen(true)}
        onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setOpen(true); } }}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{
          height: '100%', boxSizing: 'border-box',
          borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${hover ? colors.borderHi : colors.border}`,
          boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
          padding: '18px 20px',
          display: 'flex', flexDirection: 'column',
          overflow: 'hidden',
          cursor: 'pointer',
          transition: reduceMotion ? 'none' : 'border-color 150ms ease',
        }}
      >
        {/* Kicker */}
        <div style={{
          fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600,
          letterSpacing: '0.10em', textTransform: 'uppercase',
          color: colors.textDim, marginBottom: 6,
        }}>
          Needs you
        </div>

        {empty ? (
          // One line and a small tick, top-aligned. "All clear" is genuinely
          // low-information — it should cost a line, not a whole card.
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, paddingTop: 2 }}>
            <span style={{
              width: 18, height: 18, borderRadius: '50%', flexShrink: 0,
              background: colors.success + '26', color: colors.success,
              display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: textSize.micro,
            }}>✓</span>
            <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>
              {s.allClearLabel}
            </span>
          </div>
        ) : (
          <>
            {/* Stat-style count */}
            <div style={{
              fontFamily: font.display, fontSize: textSize.display, fontWeight: 600,
              letterSpacing: '-0.02em', ...tabularNums,
              color: count > 0 ? colors.cyan : colors.text,
            }}>
              {data === null ? '—' : count}
            </div>
            <div style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text, marginTop: 2 }}>
              {s.headline}
            </div>
            {s.oldestLabel && (
              <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, marginTop: 6 }}>
                {s.oldestLabel}
              </div>
            )}
            {s.attentionLabel && (
              <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: '#e8a33d', marginTop: 6 }}>
                {s.attentionLabel}
              </div>
            )}
            {s.handledLabel && (
              <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, marginTop: 14 }}>
                {s.handledLabel} →
              </div>
            )}
            <div style={{ marginTop: 'auto', fontFamily: font.body, fontSize: textSize.caption, fontWeight: 500, color: colors.cyan }}>
              Review →
            </div>
          </>
        )}
      </div>

      {open && <DecisionInbox inbox={inbox} onClose={() => setOpen(false)} />}
    </>
  );
}
