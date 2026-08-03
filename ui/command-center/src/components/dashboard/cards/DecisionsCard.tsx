/**
 * Decision Inbox — home card ("decisions" registry entry, Lane L4).
 *
 * Glanceable only: count + plain words; the whole card is tappable and opens
 * the inbox overlay. No actions live on the card itself.
 */

import { useState } from 'react';
import { font, radius, tabularNums } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { formatAge } from '../decisions/format';
import { usePersona } from '../../settings/useSettings';

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
  const agentName = persona?.display_name ?? 'Aria';
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(false);

  const count = data?.total_pending ?? 0;
  const handled = data?.handled_count ?? 0;
  const goals = activeCount ?? data?.goals_in_flight ?? 0;
  const oldest = data?.oldest_pending_at ?? null;

  const empty = data !== null && count === 0;

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
          fontFamily: font.body, fontSize: 11, fontWeight: 600,
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
              display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 11,
            }}>✓</span>
            <span style={{ fontSize: 12, color: colors.textMuted }}>
              All clear — {goals} goal{goals === 1 ? '' : 's'} in flight
            </span>
          </div>
        ) : (
          <>
            {/* Stat-style count */}
            <div style={{
              fontFamily: font.display, fontSize: 32, fontWeight: 600,
              letterSpacing: '-0.02em', ...tabularNums,
              color: count > 0 ? colors.cyan : colors.text,
            }}>
              {data === null ? '—' : count}
            </div>
            <div style={{ fontFamily: font.body, fontSize: 13, fontWeight: 600, color: colors.text, marginTop: 2 }}>
              {data === null
                ? `Checking with ${agentName}…`
                : `${agentName} needs ${count} answer${count === 1 ? '' : 's'}`}
            </div>
            {oldest && (
              <div style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, marginTop: 6 }}>
                oldest waiting {formatAge(oldest)}
              </div>
            )}
            {handled > 0 && (
              <div style={{ fontFamily: font.body, fontSize: 11, color: colors.textMuted, marginTop: 14 }}>
                {agentName} handled {handled} routine item{handled === 1 ? '' : 's'} overnight →
              </div>
            )}
            <div style={{ marginTop: 'auto', fontFamily: font.body, fontSize: 12, fontWeight: 500, color: colors.cyan }}>
              Review →
            </div>
          </>
        )}
      </div>

      {open && <DecisionInbox inbox={inbox} onClose={() => setOpen(false)} />}
    </>
  );
}
