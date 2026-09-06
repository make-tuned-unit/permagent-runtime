import { useEffect, useState } from 'react';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { font, textSize } from '../../styles/tokens';
import { formatCostMeter, type SubagentCostIncl } from '../../lib/costMeter';
import { GLOSSARY } from '../../lib/vocabulary';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { api } from '../../lib/api';
import { RoleRoutingPrompt } from '../chat/RoleRoutingPrompt';

function budgetTitle(codingSpend: ReturnType<typeof useCommandCenter.getState>['codingSpend']): string {
  const budget = codingSpend?.budget;
  if (!budget) return GLOSSARY.costMeter;
  const format = (value: number | null) => value === null ? 'unavailable' : `$${value.toFixed(2)}`;
  const cap = budget.session.cap;
  const evidence = [
    budget.sessionBilling.billingClass,
    budget.sessionBilling.provider,
    budget.sessionBilling.model,
  ].filter(Boolean).join(' / ') || 'unavailable';
  return [
    `Budget projection ${budget.provenance.version}`,
    `session used ${format(budget.session.effectiveUsedUsd)}`,
    `session settled ${format(budget.session.settledUsd)}`,
    `remaining ${format(budget.session.remainingUsd)}`,
    `cap soft/gate/hard ${format(cap.softUsd)}/${format(cap.gateUsd)}/${format(cap.hardUsd)}`,
    `billing ${evidence}`,
    `task used ${format(budget.task.effectiveUsedUsd)}`,
    `task settled ${format(budget.task.settledUsd)}`,
    `sources ${budget.provenance.sources.join(', ') || 'unavailable'}`,
    `as of ${budget.provenance.asOf}`,
  ].join(' · ');
}

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
  const codingHarnessHydration = useCommandCenter((s) => s.codingHarnessHydration);
  const chatSessionId = useCommandCenter((s) => s.chatSessionId);
  const { goals } = useLiveGoals();
  const [subagentRollup, setSubagents] = useState<{
    sessionId: string;
    value: SubagentCostIncl;
  } | null>(null);

  const rollupSessionId = codingSpend?.sessionId ?? chatSessionId;
  // Scope async evidence to its source before rendering, including the render
  // before the new session's effect runs. Old child costs are never new costs.
  const subagents = subagentRollup?.sessionId === rollupSessionId
    ? subagentRollup.value : null;

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
        setSubagents({
          sessionId: rollupSessionId,
          value: { count: cost.perChild.length, totalUsd: cost.childrenTotal },
        });
      })
      .catch(() => {
        if (!cancelled) setSubagents(null);
      });
    return () => {
      cancelled = true;
    };
  }, [rollupSessionId, liveTokens?.accumulatedCostUsd, codingSpend?.sessionUsd]);

  const meter = formatCostMeter(liveTokens, codingSpend, subagents, codingHarnessHydration);
  const routeNote = goals.find((g) => g.routing_note || g.hold_note);
  const note = routeNote?.hold_note || routeNote?.routing_note;

  return (
    <div
      role="status"
      aria-label={meter.ariaLabel}
      // The line compresses four ideas into about forty characters — "37% ctx",
      // "cache saved $0.12", "incl. 2 subagents". The audience is technical and
      // the severity is low, but a term the interface invents still gets
      // defined somewhere, and hover is the cheapest somewhere on a status bar
      // with no room for prose.
      title={budgetTitle(codingSpend)}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 2,
        padding: '6px 18px',
        borderTop: `1px solid ${colors.border}`,
        fontFamily: font.mono,
        fontSize: textSize.micro,
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
