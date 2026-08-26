import { useEffect, useState } from 'react';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { font } from '../../styles/tokens';
import { formatCostMeter, type SubagentCostIncl } from '../../lib/costMeter';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { api } from '../../lib/api';

/**
 * Always-on Build statusline: `$0.42 · 47k↑ 12k↓ · cache saved $0.28 · 31% ctx · <model>`
 * and, when the session has spawned children, `· incl. N subagents $X`.
 *
 * Reads `liveTokens` (chat SSE) and `codingSpend` (harness announcement when
 * present). Child spend comes from `GET /api/sessions/{id}/cost` for the
 * active chat session (or the coding session id when that is the authority).
 */
export function CostStatusline() {
  const { colors } = useTheme();
  const liveTokens = useCommandCenter((s) => s.liveTokens);
  const codingSpend = useCommandCenter((s) => s.codingSpend);
  const chatSessionId = useCommandCenter((s) => s.chatSessionId);
  const { goals } = useLiveGoals();
  const [subagents, setSubagents] = useState<SubagentCostIncl | null>(null);

  const rollupSessionId = codingSpend?.sessionId ?? chatSessionId;

  useEffect(() => {
    if (!rollupSessionId) {
      setSubagents(null);
      return;
    }
    let cancelled = false;
    api
      .getSessionCost(rollupSessionId)
      .then((cost) => {
        if (cancelled) return;
        if (cost.perChild.length === 0) {
          setSubagents(null);
          return;
        }
        setSubagents({ count: cost.perChild.length, totalUsd: cost.childrenTotal });
      })
      .catch(() => {
        if (!cancelled) setSubagents(null);
      });
    return () => {
      cancelled = true;
    };
  }, [rollupSessionId, liveTokens?.accumulatedCostUsd, codingSpend?.sessionUsd]);

  const meter = formatCostMeter(liveTokens, codingSpend, subagents);
  const routeNote = goals.find((g) => g.routing_note || g.hold_note);
  const note = routeNote?.hold_note || routeNote?.routing_note;

  return (
    <div
      role="status"
      aria-label={meter.ariaLabel}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 2,
        padding: '6px 18px',
        borderTop: `1px solid ${colors.border}`,
        fontFamily: font.mono,
        fontSize: 11,
        color: colors.textMuted,
        flexShrink: 0,
        overflow: 'hidden',
        whiteSpace: 'nowrap',
      }}
    >
      {/* THE authoritative number: running session spend. */}
      <span
        style={{
          fontWeight: 700,
          color: colors.text,
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {meter.cost}
      </span>

      {meter.segments.map((seg, i) => {
        const isSaving = seg.startsWith('cache saved');
        return (
          <span key={i} style={{ display: 'inline-flex', alignItems: 'center' }}>
            <span style={{ color: colors.textDim, margin: '0 7px' }} aria-hidden="true">
              ·
            </span>
            <span
              style={{
                color: isSaving ? colors.success : colors.textMuted,
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {seg}
            </span>
          </span>
        );
      })}
      {note && (
        <span style={{ display: 'inline-flex', alignItems: 'center' }}>
          <span style={{ color: colors.textDim, margin: '0 7px' }} aria-hidden="true">
            ·
          </span>
          <span style={{ color: colors.textMuted }}>{note}</span>
        </span>
      )}
    </div>
  );
}
