import { useCallback, useEffect, useState } from 'react';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { font, space, textSize } from '../../styles/tokens';
import {
  formatBudgetDetail,
  formatCostMeter,
  isCautionSegment,
  type SubagentCostIncl,
} from '../../lib/costMeter';
import { GLOSSARY } from '../../lib/vocabulary';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { api } from '../../lib/api';
import { RoleRoutingPrompt } from '../chat/RoleRoutingPrompt';
import { ProviderModelPicker } from '../settings/ProviderModelPicker';
import { CHAT_LAUNCHER_MARGIN } from '../chat/ChatLauncher';

import { Tooltip } from '../common/Tooltip';
/**
 * The tooltip carries everything the condensed bar leaves out. The bar used to
 * print the whole ledger inline ("session settled … session used … session
 * remaining … session unknown … session budget unknown · task settled …") and
 * The user's verdict was "I am unsure what all of this is"; the detail moved here,
 * one fact per line, in words, rather than being dropped.
 */
function budgetTitle(codingSpend: ReturnType<typeof useCommandCenter.getState>['codingSpend']) {
  const budget = codingSpend?.budget;
  if (!budget) return GLOSSARY.costMeter;
  return (
    <div data-testid="cost-budget-detail" style={{ display: 'flex', flexDirection: 'column', whiteSpace: 'pre' }}>
      {formatBudgetDetail(budget).split('\n').map((line, i) => <div key={i}>{line}</div>)}
    </div>
  );
}

/**
 * The model you are spending on, at the end of the line that says what you are
 * spending. The user: "The model picker dropdown disappeared. It would make sense
 * for it to be on the bottom bar with the costs and I could switch between
 * models."
 *
 * It had not disappeared — it was in the terminal's tab strip, which renders it
 * only for a `permagent run --interactive` tab, and he had two plain shells
 * open. That gate is right about WHEN a switch is possible and wrong about
 * WHERE the control belongs: the model and the money are one glance.
 *
 * Degradation is explicit rather than hidden. With no harness session this
 * renders a non-interactive note saying so, because hiding the control is the
 * exact behaviour that was reported as a bug — a control that vanishes teaches
 * nothing, while "no harness session" answers the question the absence raised.
 * It is not a disabled *button*: there is no dead trigger to click.
 *
 * Selection state lives in the store (`harnessModelSessionId` /
 * `harnessModelSelections`), published by TerminalManager, so this picker and
 * the detached terminal's picker are two views of one value.
 */
function HarnessModelControl() {
  const { colors } = useTheme();
  const sessionId = useCommandCenter((s) => s.harnessModelSessionId);
  const selections = useCommandCenter((s) => s.harnessModelSelections);
  const request = useCommandCenter((s) => s.harnessModelRequest);
  const requestHarnessModel = useCommandCenter((s) => s.requestHarnessModel);

  // The harness role's saved default, for the no-session case below. Read once
  // per mount; written straight back through the same call Settings makes, so
  // the two surfaces cannot drift.
  const [defaultRoute, setDefaultRoute] = useState<{ provider: string | null; model: string | null }>(
    { provider: null, model: null },
  );
  useEffect(() => {
    let live = true;
    Promise.all([api.readConfig('harness_provider'), api.readConfig('harness_model')])
      .then(([provider, model]) => {
        if (!live) return;
        setDefaultRoute({
          provider: typeof provider === 'string' && provider ? provider : null,
          model: typeof model === 'string' && model ? model : null,
        });
      })
      .catch(() => { /* an unreadable config just leaves the picker unset */ });
    return () => { live = false; };
  }, []);
  const saveDefaultRoute = useCallback(async (value: { provider: string; model: string }) => {
    setDefaultRoute(value);
    try {
      await api.setModelRoute('harness', value.provider, value.model);
    } catch {
      // Put the picker back to what the daemon still holds rather than showing
      // a choice that was never saved.
      const [provider, model] = await Promise.all([
        api.readConfig('harness_provider').catch(() => null),
        api.readConfig('harness_model').catch(() => null),
      ]);
      setDefaultRoute({
        provider: typeof provider === 'string' && provider ? provider : null,
        model: typeof model === 'string' && model ? model : null,
      });
    }
  }, []);

  // `marginLeft: auto` pins this to the right end and `flexShrink: 0` keeps it
  // whole: the bar clips from the right, so anything that shrinks must be the
  // supporting segments, never the authoritative spend figure on the left.
  const anchored = {
    marginLeft: 'auto',
    flexShrink: 0,
    paddingLeft: space.xl,
    display: 'inline-flex',
    alignItems: 'center',
    gap: space.sm,
  } as const;

  // With no harness running the control is NOT dead: it edits the model the
  // next run will start on — the same `harness` role Settings → Models writes.
  // It used to read "no harness — model fixed", which answered the question
  // ("why can't I switch?") but left the user's actual ask — "I could switch
  // between models" from the cost bar — unserved whenever no session was live.
  if (!sessionId) {
    return (
      <div style={anchored} data-testid="statusline-model-default">
        <Tooltip content="No harness session is running. This sets the model the NEXT harness run starts on — the same setting as Settings → Models → Harness.">
          <span tabIndex={0} style={{ color: colors.textDim, whiteSpace: 'nowrap', outline: 'none' }}>
            next run
          </span>
        </Tooltip>
        <div style={{ width: 190, maxWidth: '26vw' }}>
          <ProviderModelPicker
            value={defaultRoute}
            onChange={(value) => { void saveDefaultRoute(value); }}
            compact
            aria-label="Choose the model the next harness run uses"
          />
        </div>
      </div>
    );
  }

  return (
    <div style={anchored} data-testid="statusline-model-control">
      {request?.sessionId === sessionId && (
        <span aria-live="polite" style={{ color: colors.textDim, whiteSpace: 'nowrap' }}>
          requested {request.provider}/{request.model}
        </span>
      )}
      {/* Fixed, narrow and capped: the trigger is one line of text plus a
          chevron, and it must not grow with the model name it is showing. The
          menu itself is portalled to the body and flips upward on its own when
          there is no room below, which is what a control pinned to the bottom
          of the window needs — do not re-anchor it here. */}
      <div style={{ width: 190, maxWidth: '26vw' }}>
        <ProviderModelPicker
          value={selections[sessionId] ?? { provider: null, model: null }}
          onChange={(value) => { void requestHarnessModel(value); }}
          compact
          aria-label="Choose harness provider and model"
        />
      </div>
    </div>
  );
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
/** Right inset of the Build status bar: clear of the floating chat pill when it
 *  is showing, otherwise the bar's own 18px gutter. */
export function statuslineRightPad(launcher: { width: number; height: number } | null): number {
  const GUTTER = 18;
  if (!launcher) return GUTTER;
  return Math.ceil(launcher.width) + GUTTER + CHAT_LAUNCHER_MARGIN;
}

export function CostStatusline() {
  const { colors } = useTheme();
  const chatLauncherSize = useCommandCenter((s) => s.chatLauncherSize);
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
    // The bar owns the chrome (border, padding, mono micro type, one clipped
    // line); the cost group and the model picker are its two children. The
    // budget tooltip covers the FIGURES only — hovering the picker to change
    // models should not raise a wall of ledger text over the menu.
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        // The floating chat pill sits over this bar's right end — it covered
        // the model control (QA 2026-09-20). The right padding clears whatever
        // the pill currently measures.
        padding: `${space.sm}px ${statuslineRightPad(chatLauncherSize)}px ${space.sm}px 18px`,
        borderTop: `1px solid ${colors.border}`,
        fontFamily: font.mono,
        fontSize: textSize.micro,
        color: colors.textMuted,
        flexShrink: 0,
        overflow: 'hidden',
        whiteSpace: 'nowrap',
      }}
    >
      {/* The line compresses four ideas into about forty characters — "37% ctx",
          "cache saved $0.12", "incl. 2 subagents". The audience is technical and
          the severity is low, but a term the interface invents still gets defined
          somewhere, and the tip is the cheapest somewhere on a status bar with no
          room for prose. `budgetTitle` spends that room on the live projection
          when there is one and falls back to the glossary entry when there isn't. */}
      <Tooltip content={budgetTitle(codingSpend)}>
        <span tabIndex={0} style={{ outline: 'none', minWidth: 0, overflow: 'hidden' }}>
          <div
            role="status"
            aria-label={meter.ariaLabel}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: space.xxs,
              minWidth: 0,
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
              // A figure that is not a confirmed bill — the fail-closed estimate,
              // budget held for calls whose charge could not be confirmed, a
              // remaining budget computed from either —
              // needs to read as a caution, not as ordinary supporting context.
              // colors.warning exists on the theme, so use it; a theme without
              // one falls back to textMuted rather than inventing a color that
              // isn't part of the design system.
              const isEstimated = isCautionSegment(seg);
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
        </span>
      </Tooltip>
      <HarnessModelControl />
    </div>
  );
}
