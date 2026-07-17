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
import { useCallback, useEffect, useState } from 'react';
import { FiSquare } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { relativeTimeAgo } from '../../lib/time-decay';

interface RunItem {
  kind: 'worker' | 'schedule' | 'session';
  id: string;
  name: string;
  status: 'working' | 'idle' | 'error' | 'missed' | 'paused';
  detail: string | null;
  last_activity: string | null;
  started_at: string | null;
  interruptible: boolean;
}

const KIND_LABEL: Record<RunItem['kind'], string> = {
  worker: 'Worker',
  schedule: 'Schedule',
  session: 'Session',
};

export function RunRoster() {
  const { colors } = useTheme();
  const [runs, setRuns] = useState<RunItem[]>([]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');

  const load = useCallback(async () => {
    try {
      const r = await apiFetch<{ runs: RunItem[] }>('/api/runs');
      setRuns(Array.isArray(r.runs) ? r.runs : []);
      setStatus('ready');
    } catch {
      setStatus('error');
    }
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(load, 4000);
    return () => clearInterval(t);
  }, [load]);

  const [stopError, setStopError] = useState<string | null>(null);
  const interrupt = useCallback(async (r: RunItem) => {
    if (r.kind !== 'schedule') return;
    try {
      await apiFetch(`/schedule/${encodeURIComponent(r.id)}/kill`, { method: 'POST' });
      setStopError(null);
      load();
    } catch (err) {
      // Surface the failure — a silent no-op Stop reads as a dead button
      // (2026-07 wiring audit). The next poll still reflects reality.
      const msg = err instanceof Error ? err.message : String(err);
      setStopError(`Couldn't stop "${r.name}": ${msg}`);
      setTimeout(() => setStopError(null), 8000);
    }
  }, [load]);

  const dotColor = (s: RunItem['status']): string => ({
    working: colors.cyan,
    idle: colors.textDim,
    error: colors.danger,
    missed: colors.warning,
    paused: colors.textMuted,
  }[s]);

  // Sort the eye to what matters: active first, then problems, then the rest.
  const rank = (s: RunItem['status']) =>
    ({ working: 0, error: 1, missed: 2, paused: 3, idle: 4 }[s] ?? 5);
  const sorted = [...runs].sort((a, b) => rank(a.status) - rank(b.status));
  const activeCount = runs.filter(r => r.status === 'working').length;

  return (
    <div
      style={{
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        borderRadius: radius.lg,
        padding: '14px 16px',
        marginBottom: 16,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
        <div style={{ fontFamily: font.display, fontSize: 13, fontWeight: 600, color: colors.text, letterSpacing: 0.2 }}>
          Agents at work
        </div>
        <span style={{ fontSize: 11, color: colors.textDim, fontFamily: font.body }}>
          {activeCount > 0 ? `${activeCount} active` : 'all resting'}
        </span>
      </div>

      {status === 'loading' && (
        <div style={{ fontSize: 12, color: colors.textDim, fontFamily: font.body }}>Loading…</div>
      )}

      {stopError && (
        <div style={{ fontSize: 11, color: colors.danger, fontFamily: font.body, marginBottom: 8 }}>
          {stopError}
        </div>
      )}

      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: 12, color: colors.danger, fontFamily: font.body }}>Couldn't load activity.</span>
          <button
            onClick={load}
            style={{
              fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
              cursor: 'pointer', fontFamily: font.body, padding: 0, fontWeight: 600,
            }}
          >
            Retry
          </button>
        </div>
      )}

      {status === 'ready' && sorted.length === 0 && (
        <div style={{ fontSize: 12, color: colors.textDim, fontFamily: font.body, lineHeight: 1.5 }}>
          Nothing running — your workers rest until their next tick.
        </div>
      )}

      {status === 'ready' && sorted.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {sorted.map(r => (
            <div
              key={`${r.kind}:${r.id}`}
              style={{
                display: 'flex', alignItems: 'center', gap: 10, padding: '7px 10px',
                borderRadius: radius.md, background: colors.bgDeeper,
                border: `1px solid ${colors.border}`,
              }}
            >
              <span
                title={r.status}
                style={{
                  width: 8, height: 8, borderRadius: '50%', flexShrink: 0,
                  background: dotColor(r.status),
                  boxShadow: r.status === 'working' ? `0 0 6px ${colors.cyanGlow}` : 'none',
                }}
              />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
                  <span style={{ fontSize: 12.5, fontWeight: 600, color: colors.text, fontFamily: font.body, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {r.name}
                  </span>
                  <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.body, flexShrink: 0 }}>
                    {KIND_LABEL[r.kind]}
                  </span>
                </div>
                {r.detail && (
                  <div style={{ fontSize: 11, color: colors.textMuted, fontFamily: font.body, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {r.detail}
                  </div>
                )}
              </div>
              {r.last_activity && (
                <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.body, flexShrink: 0 }}>
                  {relativeTimeAgo(r.last_activity)}
                </span>
              )}
              {/* Only schedules have a kill verb today — showing Stop on
                  worker/session rows made a button that did nothing
                  (interrupt() early-returns for those kinds). */}
              {r.interruptible && r.kind === 'schedule' && (
                <button
                  onClick={() => interrupt(r)}
                  title="Stop this run"
                  style={{
                    display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0,
                    fontSize: 10, fontFamily: font.body, color: colors.danger,
                    background: 'none', border: `1px solid ${colors.border}`,
                    borderRadius: radius.sm, padding: '3px 7px', cursor: 'pointer',
                  }}
                >
                  <FiSquare size={9} /> Stop
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
