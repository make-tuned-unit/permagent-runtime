/**
 * Governance → Approvals. A summary of pending Decision-Inbox items + the
 * current tool-approval posture (effective goose-mode). Reuses the REAL
 * Decision Inbox: the same `useDecisions()` data hook the home card uses, and
 * the same `DecisionInbox` overlay — this panel never forks that surface, it
 * links into it.
 */

import { useEffect, useState } from 'react';
import { api } from '../../lib/api';
import { font, tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useDecisions } from '../dashboard/decisions/useDecisions';
import { DecisionInbox } from '../dashboard/decisions/DecisionInbox';
import { formatAge } from '../dashboard/decisions/format';
import { Card, SectionLabel, StatRow } from './atoms';

/** Human-readable posture for the raw goose-mode value. */
function postureLabel(mode: string | null): { title: string; detail: string } {
  switch (mode) {
    case 'auto':
      return { title: 'Auto', detail: 'The agent runs tools without asking. Nothing is gated.' };
    case 'approve':
      return { title: 'Approve every tool', detail: 'Every tool call is routed to the Decision Inbox for your sign-off.' };
    case 'smart_approve':
      return { title: 'Smart approve', detail: 'Risky tool calls are routed to the Decision Inbox; safe ones run.' };
    case 'chat':
      return { title: 'Chat only', detail: 'The agent cannot run tools — conversation only.' };
    default:
      return { title: mode ?? '—', detail: 'Tool-approval posture.' };
  }
}

export function ApprovalsPanel() {
  const { colors } = useTheme();
  const inbox = useDecisions();
  const { data } = inbox;
  const [mode, setMode] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let active = true;
    api.getConfig()
      .then(cfg => { if (active) setMode((cfg.effective_goose_mode as string) ?? null); })
      .catch(() => { /* posture stays unknown; the panel still shows pending counts */ });
    return () => { active = false; };
  }, []);

  const pending = data?.total_pending ?? 0;
  const handled = data?.handled_count ?? 0;
  const goals = data?.goals_in_flight ?? 0;
  const oldest = data?.oldest_pending_at ?? null;
  const posture = postureLabel(mode);

  return (
    <>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <Card>
          <SectionLabel>Pending approvals — waiting on you</SectionLabel>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 18, marginTop: 10, flexWrap: 'wrap' }}>
            <div style={{ fontFamily: font.display, fontSize: 36, fontWeight: 600, letterSpacing: '-0.02em', color: pending > 0 ? colors.cyan : colors.text, ...tabularNums }}>
              {data === null ? '—' : pending}
            </div>
            <div style={{ fontSize: 13, color: colors.textMuted }}>
              {data === null
                ? 'Checking the Decision Inbox…'
                : pending === 0
                  ? 'Nothing needs your sign-off.'
                  : `${pending} item${pending === 1 ? '' : 's'} awaiting your decision`}
              {oldest && pending > 0 && (
                <span style={{ color: colors.textDim }}> · oldest {formatAge(oldest)}</span>
              )}
            </div>
          </div>
          <div style={{ display: 'flex', gap: 20, marginTop: 14, fontSize: 12, color: colors.textMuted }}>
            <span>{goals} goal{goals === 1 ? '' : 's'} in flight</span>
            {handled > 0 && <span>{handled} handled routinely</span>}
          </div>
          <button
            onClick={() => setOpen(true)}
            style={{
              marginTop: 16, height: 32, padding: '0 16px', borderRadius: 8,
              background: colors.cyan, border: 'none', color: colors.textOnAccent,
              cursor: 'pointer', fontFamily: font.body, fontSize: 12, fontWeight: 600,
              alignSelf: 'flex-start',
            }}
          >
            Open Decision Inbox →
          </button>
        </Card>

        <Card>
          <SectionLabel>Tool-approval posture</SectionLabel>
          <div style={{ marginTop: 12 }}>
            <StatRow
              left={posture.title}
              sub={posture.detail}
              right={
                <span style={{ fontSize: 11, fontFamily: font.mono, color: colors.textDim }}>{mode ?? 'unknown'}</span>
              }
            />
          </div>
          <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 10 }}>
            Change the posture in Settings → Autonomy. Gated tool calls arrive in the Decision Inbox above.
          </div>
        </Card>
      </div>

      {open && <DecisionInbox inbox={inbox} onClose={() => setOpen(false)} />}
    </>
  );
}
