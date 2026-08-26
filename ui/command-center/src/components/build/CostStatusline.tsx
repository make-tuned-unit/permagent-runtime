import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { font } from '../../styles/tokens';
import { formatCostMeter } from '../../lib/costMeter';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { RoleRoutingPrompt } from '../chat/RoleRoutingPrompt';

/**
 * Always-on Build statusline: `$0.42 · 47k↑ 12k↓ · cache saved $0.28 · 31% ctx · <model>`.
 *
 * Reads the live {@link TokenState} the store captures off every SSE frame, so
 * the running-session dollar figure is real and single-sourced from the per-call
 * cost ledger (canonical `cost_of`) — never a stub. It shows exactly ONE
 * spend number (the bold `$`); the cache figure is explicitly a *saving*, so
 * there is no "Usage $ vs Spending %" ambiguity. The rendered strings come
 * verbatim from {@link formatCostMeter}, which is what the wiring test asserts.
 */
export function CostStatusline() {
  const { colors } = useTheme();
  const liveTokens = useCommandCenter((s) => s.liveTokens);
  const { goals } = useLiveGoals();
  const meter = formatCostMeter(liveTokens);
  const routeNote = goals.find(g => g.routing_note || g.hold_note);
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
      <RoleRoutingPrompt variant="compact" />
    </div>
  );
}
