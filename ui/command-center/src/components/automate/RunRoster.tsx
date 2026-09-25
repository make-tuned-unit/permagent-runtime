/**
 * RunRoster — the "agents at work" surface.
 *
 * A live roster of everything running or recently active: background workers,
 * scheduled jobs (with their run status), and active agent sessions — one row
 * each with a status dot, what it's doing, how long ago it last acted, and an
 * interrupt control where the daemon allows it. Polls `/api/runs` (a best-effort
 * aggregate) every few seconds. Interrupting a running schedule reuses the
 * existing `/schedule/{id}/kill` route.
 *
 * The point: long-running and parallel agent work used to be invisible. This
 * makes it legible at a glance and stoppable in one click.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import { FiSquare } from 'react-icons/fi';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { apiFetch } from '../../lib/api';
import { relativeTimeAgo } from '../../lib/time-decay';
import { useCommandCenter } from '../../lib/store';

import { Tooltip } from '../common/Tooltip';
import { reconcileRosterRuns, rosterDetail, rosterProblem, type RosterRunItem, type RosterScheduleFacts } from './runRosterModel';

type RunItem = RosterRunItem;

const KIND_LABEL: Record<RunItem['kind'], string> = {
  worker: 'Worker',
  schedule: 'Schedule',
  session: 'Session',
};

/**
 * @param schedules The Automate view's `/schedule/list` rows. `/api/runs` has
 *   no last error or failure count for a schedule, so a paused or failing row
 *   takes them from here (joined by id). Optional: without it the row still
 *   shows its status and schedule, just not why it stopped.
 */
export function RunRoster({ schedules }: { schedules?: RosterScheduleFacts[] } = {}) {
  const { colors } = useTheme();
  const factsById = useMemo(
    () => new Map((schedules ?? []).map(j => [j.id, j] as const)),
    [schedules],
  );
  const openAgentSettings = useCommandCenter(s => s.openAgentSettings);
  const [runs, setRuns] = useState<RunItem[]>([]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');

  const load = useCallback(async () => {
    try {
      const r = await apiFetch<{ runs: RunItem[] }>('/api/runs');
      setRuns(Array.isArray(r.runs) ? r.runs : []);
      setStatus('ready');
      return true;
    } catch {
      setStatus('error');
      // Swallowed on purpose (the roster shows its own error row), so it reports
      // the failure to the Retry button as `false` — no tick on a failed reload.
      return false;
    }
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(load, 4000);
    return () => clearInterval(t);
  }, [load]);

  const [stopError, setStopError] = useState<string | null>(null);
  const interrupt = useCallback(async (r: RunItem) => {
    if (r.kind !== 'schedule') return false;
    try {
      await apiFetch(`/schedule/${encodeURIComponent(r.id)}/kill`, { method: 'POST' });
      setStopError(null);
      load();
      return true;
    } catch (err) {
      // Surface the failure — a silent no-op Stop reads as a dead button
      // (2026-07 wiring audit). The next poll still reflects reality.
      const msg = err instanceof Error ? err.message : String(err);
      setStopError(`Couldn't stop "${r.name}": ${msg}`);
      setTimeout(() => setStopError(null), 8000);
      // This helper states its own failure rather than rejecting, so it says so
      // to the button primitive too: `false` means "no success tick".
      return false;
    }
  }, [load]);

  // Resume a paused schedule — the same `/schedule/{id}/unpause` route the
  // Automate list's own Resume button calls. A paused row used to say so only
  // with a muted dot and a hover tooltip, and offered no way back.
  const resume = useCallback(async (r: RunItem) => {
    if (r.kind !== 'schedule') return false;
    try {
      await apiFetch(`/schedule/${encodeURIComponent(r.id)}/unpause`, { method: 'POST' });
      setStopError(null);
      load();
      return true;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setStopError(`Couldn't resume "${r.name}": ${msg}`);
      setTimeout(() => setStopError(null), 8000);
      return false;
    }
  }, [load]);

  const dotColor = (s: RunItem['status']): string => ({
    working: colors.cyan,
    idle: colors.textDim,
    error: colors.danger,
    missed: colors.warning,
    paused: colors.textMuted,
  }[s]);

  // R5: reconcile against `/schedule/list` BEFORE sorting/counting, so this
  // panel and "Running Now" never show two different answers to "is it
  // running" for the same schedule (see runRosterModel.reconcileRosterRuns).
  const reconciledRuns = useMemo(
    () => reconcileRosterRuns(runs, schedules ?? []),
    [runs, schedules],
  );

  // Sort the eye to what matters: active first, then problems, then the rest.
  const rank = (s: RunItem['status']) =>
    ({ working: 0, error: 1, missed: 2, paused: 3, idle: 4 }[s] ?? 5);
  const sorted = [...reconciledRuns].sort((a, b) => rank(a.status) - rank(b.status));
  const activeCount = reconciledRuns.filter(r => r.status === 'working').length;

  return (
    <div
      style={{
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        borderRadius: radius.lg,
        padding: `14px ${space.xxl}px`,
        marginBottom: space.xxl,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: space.lg }}>
        <div style={{ fontFamily: font.display, fontSize: textSize.small, fontWeight: 600, color: colors.text, letterSpacing: 0.2 }}>
          Agents at work
        </div>
        <span style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.body }}>
          {activeCount > 0 ? `${activeCount} active` : 'all resting'}
        </span>
      </div>

      {status === 'loading' && (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, fontFamily: font.body }}>Loading…</div>
      )}

      {stopError && (
        <div style={{ fontSize: textSize.micro, color: colors.danger, fontFamily: font.body, marginBottom: space.md }}>
          {stopError}
        </div>
      )}

      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: space.lg }}>
          <span style={{ fontSize: textSize.caption, color: colors.danger, fontFamily: font.body }}>Couldn't load activity.</span>
          <Button
            colors={colors}
            variant="bare"
            onClick={load}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 600,
              fontFamily: font.body,
              fontSize: textSize.micro,
            } as CSSProperties}
          >
            Retry
          </Button>
        </div>
      )}

      {status === 'ready' && sorted.length === 0 && (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, fontFamily: font.body, lineHeight: 1.5 }}>
          Nothing running — your workers rest until their next tick.
        </div>
      )}

      {status === 'ready' && sorted.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.xs }}>
          {sorted.map(r => (
            <div
              key={`${r.kind}:${r.id}`}
              style={{
                display: 'flex', alignItems: 'center', gap: space.lg, padding: `${space.sm}px ${space.lg}px`,
                borderRadius: radius.md, background: colors.bgDeeper,
                border: `1px solid ${colors.border}`,
              }}
            >
              <Tooltip content={r.status}>
                <span tabIndex={0} style={{ outline: 'none' }}>
                  <span
                    style={{
                      width: 8, height: 8, borderRadius: '50%', flexShrink: 0,
                      background: dotColor(r.status),
                      boxShadow: r.status === 'working' ? `0 0 6px ${colors.cyanGlow}` : 'none',
                    }}
     />
                </span>
              </Tooltip>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: space.md, minWidth: 0 }}>
                  <span style={{ fontSize: 12.5, fontWeight: 600, color: colors.text, fontFamily: font.body, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {r.name}
                  </span>
                  {r.status === 'paused' && (
                    <span data-testid="roster-paused" style={{
                      fontSize: 10, fontWeight: 600, color: colors.warning, fontFamily: font.body, flexShrink: 0,
                      padding: `0 ${space.sm}px`, borderRadius: radius.pill, border: `1px solid ${colors.warning}`,
                    }}>
                      Paused
                    </span>
                  )}
                  <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.body, flexShrink: 0 }}>
                    {KIND_LABEL[r.kind]}
                  </span>
                </div>
                {(() => {
                  const facts = r.kind === 'schedule' ? factsById.get(r.id) : undefined;
                  const d = rosterDetail(r.kind, r.detail, facts);
                  const problem = r.kind === 'schedule' ? rosterProblem(r.status, facts) : null;
                  return (
                    <>
                      {d.words && (
                        <div style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.body, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                          {d.words}
                          {/* The expression itself stays one glance away, smaller
                              and dimmer than the words — truncated first. */}
                          {d.raw && (
                            <span data-testid="roster-cron" style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, marginLeft: space.md }}>
                              {d.raw}
                            </span>
                          )}
                        </div>
                      )}
                      {problem && (
                        <Tooltip content={problem}>
                          <div
                            data-testid="roster-problem"
                            tabIndex={0}
                            style={{
                              fontSize: textSize.micro, fontFamily: font.body, outline: 'none',
                              color: r.status === 'error' ? colors.danger : colors.warning,
                              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                            }}
                          >
                            {problem}
                          </div>
                        </Tooltip>
                      )}
                    </>
                  );
                })()}
              </div>
              {r.last_activity && (
                <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.body, flexShrink: 0 }}>
                  {relativeTimeAgo(r.last_activity)}
                </span>
              )}
              {r.kind === 'worker' && (
                <Tooltip content="Open in Settings → Agents">
                  <Button
                    colors={colors}
                    type="button"
                    onClick={() => openAgentSettings(r.id)}
                    style={{
                      '--pa-btn-fg': colors.cyan,
                      '--pa-btn-border': colors.border,
                      '--pa-btn-bg-hover': colors.surfaceHi,
                      '--pa-btn-border-hover': colors.borderHi,
                      '--pa-btn-pad': '3px 7px',
                      '--pa-btn-radius': `${radius.sm}px`,
                      fontFamily: font.body,
                      fontSize: 10,
                      gap: space.xs,
                      flexShrink: 0,
                    } as CSSProperties}
                  >
                    Settings
                  </Button>
                </Tooltip>
              )}
              {r.kind === 'schedule' && r.status === 'paused' && (
                <Tooltip content="Resume this schedule">
                  <Button
                    colors={colors}
                    onClick={() => resume(r)}
                    style={{
                      '--pa-btn-fg': colors.cyan,
                      '--pa-btn-border': colors.border,
                      '--pa-btn-bg-hover': colors.surfaceHi,
                      '--pa-btn-border-hover': colors.borderHi,
                      '--pa-btn-pad': '3px 7px',
                      '--pa-btn-radius': `${radius.sm}px`,
                      fontFamily: font.body,
                      fontSize: 10,
                      gap: space.xs,
                      flexShrink: 0,
                    } as CSSProperties}
                  >
                    Resume
                  </Button>
                </Tooltip>
              )}
              {/* Only schedules have a kill verb today — showing Stop on
                  worker/session rows made a button that did nothing
                  (interrupt() early-returns for those kinds). */}
              {r.interruptible && r.kind === 'schedule' && (
                // No tick: the row dropping out of "working" on the next poll is
                // the confirmation that the run stopped.
                <Tooltip content="Stop this run">
                  <Button
                    colors={colors}
                    onClick={() => interrupt(r)}
                    flashSuccess={false}
                    style={{
                      '--pa-btn-fg': colors.danger,
                      '--pa-btn-border': colors.border,
                      '--pa-btn-bg-hover': colors.surfaceHi,
                      '--pa-btn-border-hover': colors.danger,
                      '--pa-btn-pad': '3px 7px',
                      '--pa-btn-radius': `${radius.sm}px`,
                      fontFamily: font.body,
                      fontSize: 10,
                      gap: space.xs,
                      flexShrink: 0,
                    } as CSSProperties}
                  >
                    <FiSquare size={9} /> Stop
                  </Button>
                </Tooltip>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
