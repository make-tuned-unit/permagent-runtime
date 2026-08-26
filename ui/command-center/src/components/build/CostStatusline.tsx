import { useEffect, useState } from 'react';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { font } from '../../styles/tokens';
import { formatCostMeter, type SubagentCostIncl } from '../../lib/costMeter';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { api } from '../../lib/api';
import { RoleRoutingPrompt } from '../chat/RoleRoutingPrompt';

/**
 * Always-on Build statusline: `$0.42 · 47k↑ 12k↓ · cache saved $0.28 · 31% ctx · <model>`
 * while chatting, or `~$0.03 · 13k tokens · +$0.0032 this turn · today $0.53 ·
 * glm-5.3 · estimated — no published price` while the CLI coding harness runs,
 * plus `· incl. N subagents $X` when the session has spawned children.
 *
 * The Build tab's terminal runs `permagent run --recipe permagent-coding
 * --interactive` as its own PTY subprocess with its OWN session id — a
 * completely different account from the browser chat session `liveTokens`
 * tracks. Reading `liveTokens` alone (the old behavior) showed $0.00 the whole
 * time the user coded, because that stream was idle: a real number off the
 * wrong account. `codingSpend`, sourced from the daemon's
 * `session_spend_changed` bus frame (see livenessSync.ts), is the harness's
 * own ledger and — per {@link formatCostMeter} — is authoritative over
 * `liveTokens` whenever it is present. Child spend comes from
 * `GET /api/sessions/{id}/cost` for the coding session id when that is the
 * authority, otherwise the active chat session. The rendered strings come
 * verbatim from {@link formatCostMeter}, which is what the wiring test asserts.
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
        // The fail-closed estimate disclosure needs to read as a caution, not
        // as ordinary supporting context — colors.warning exists on the theme,
        // so use it; a theme without one would fall back to textMuted rather
        // than inventing a color that isn't part of the design system.
        const isEstimated = seg === 'estimated — no published price';
        return (
          <span key={i} style={{ display: 'inline-flex', alignItems: 'center' }}>
            <span style={{ color: colors.textDim, margin: '0 7px' }} aria-hidden="true">
              ·
            </span>
            <span
              style={{
                color: isSaving
                  ? colors.success
                  : isEstimated
                    ? (colors.warning ?? colors.textMuted)
                    : colors.textMuted,
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
      <RoleRoutingPrompt variant="compact" />
    </div>
  );
}
