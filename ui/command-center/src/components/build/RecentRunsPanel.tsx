/**
 * RecentRunsPanel — B1 (dagK).
 *
 * `api.getHarnessRunHistory` (lib/api.ts → GET
 * /api/coding-sessions/harness-runs/history) has existed with no caller: Build
 * only ever showed live/active harness runs (CouncilEscalation reads
 * `getActiveHarnessRuns`), so the moment a run finished, every trace of it —
 * status, cost, evidence, transcript — fell out of the UI. R5 (agent
 * transparency): a run finishing is not the run ceasing to have happened, and
 * "what did the agent just do" is exactly the question this panel answers.
 *
 * States: loading (skeleton rows, final shape), empty (StateBlock), error
 * (StateBlock + retry), populated. A poll keeps it live without a manual
 * refresh; `AsOf` says how fresh the list is.
 */

import { useEffect, useState, type CSSProperties } from 'react';
import { api, type HarnessRunView } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { font, radius, space, textSize } from '../../styles/tokens';
import { AsOf } from '../common/AsOf';
import { Button } from '../common/Button';
import { Chip, type ChipTone } from '../common/Chip';
import { StateBlock } from '../common/StateBlock';

const POLL_MS = 15_000;

function statusTone(status: HarnessRunView['status']): ChipTone {
  switch (status) {
    case 'succeeded': return 'success';
    case 'failed': case 'cancelled': return 'danger';
    case 'waiting_gate': return 'warning';
    case 'running': case 'verifying': return 'accent';
    default: return 'neutral';
  }
}

function statusLabel(status: HarnessRunView['status']): string {
  switch (status) {
    case 'waiting_gate': return 'Waiting on gate';
    case 'queued': return 'Queued';
    case 'running': return 'Running';
    case 'verifying': return 'Verifying';
    case 'succeeded': return 'Succeeded';
    case 'failed': return 'Failed';
    case 'cancelled': return 'Cancelled';
    default: return status;
  }
}

/** R3: durations are human ("3 min"), never a raw millisecond count. */
function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—';
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const remM = m % 60;
  return remM ? `${h}h ${remM}m` : `${h}h`;
}

function formatCost(usd: number | null): string {
  return usd == null ? '—' : `$${usd.toFixed(2)}`;
}

export function RecentRunsPanel() {
  const { colors } = useTheme();
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const openChatDock = useCommandCenter(s => s.openChatDock);
  const [state, setState] = useState<
    | { kind: 'loading' }
    | { kind: 'error' }
    | { kind: 'ready'; runs: HarnessRunView[]; fetchedAt: number }
  >({ kind: 'loading' });
  const [expanded, setExpanded] = useState<string | null>(null);

  const load = () => {
    api.getHarnessRunHistory()
      .then(runs => setState({ kind: 'ready', runs, fetchedAt: Date.now() }))
      .catch(() => setState(prev => (prev.kind === 'ready' ? prev : { kind: 'error' })));
  };

  useEffect(() => {
    load();
    const timer = window.setInterval(load, POLL_MS);
    return () => window.clearInterval(timer);
  }, []);

  const openTranscript = (sessionId: string) => {
    switchToSession(sessionId).catch(err => console.error('[build] open session failed:', err));
    openChatDock();
  };

  return (
    <div style={{
      margin: '10px 18px 0', borderRadius: radius.md, border: `1px solid ${colors.border}`,
      background: colors.surface, overflow: 'hidden',
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', gap: space.md,
        padding: `${space.md}px ${space.xl}px`, borderBottom: `1px solid ${colors.border}`,
      }}>
        <span style={{
          fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono,
          textTransform: 'uppercase', letterSpacing: '0.04em', flex: 1,
        }}>
          Recent runs
        </span>
        {state.kind === 'ready' && (
          <span style={{ fontSize: textSize.micro }}>
            <AsOf asOf={state.fetchedAt} prefix="Updated" />
          </span>
        )}
      </div>

      {state.kind === 'loading' && (
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          {[0, 1, 2].map(i => (
            <div key={i} style={{
              padding: `${space.md}px ${space.xl}px`, borderBottom: `1px solid ${colors.border}`,
              display: 'flex', gap: space.md, alignItems: 'center',
            }}>
              <div style={{ width: 70, height: 16, borderRadius: radius.pill, background: colors.border }} />
              <div style={{ flex: 1, height: 14, borderRadius: radius.xs, background: colors.border, maxWidth: 240 }} />
            </div>
          ))}
        </div>
      )}

      {state.kind === 'error' && (
        <StateBlock tone="error" title="Couldn't load recent runs" onRetry={load} compact />
      )}

      {state.kind === 'ready' && state.runs.length === 0 && (
        <StateBlock tone="empty" title="No runs yet" detail="A finished harness run — coding-agent or Council — appears here." compact />
      )}

      {state.kind === 'ready' && state.runs.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          {state.runs.map(run => {
            const isOpen = expanded === run.runId;
            return (
              <div key={run.runId} style={{ borderBottom: `1px solid ${colors.border}` }}>
                <div style={{
                  display: 'flex', alignItems: 'center', gap: space.md,
                  padding: `${space.md}px ${space.xl}px`,
                }}>
                  <Chip kind="state" tone={statusTone(run.status)}>{statusLabel(run.status)}</Chip>
                  <span style={{
                    flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                    fontSize: textSize.caption, color: colors.text,
                  }}>
                    {run.promptTitle || run.project}
                  </span>
                  <span style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.mono, flexShrink: 0 }}>
                    {formatDuration(run.elapsedMs)}
                  </span>
                  <span style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.mono, flexShrink: 0 }}>
                    {formatCost(run.spendUsd)}
                  </span>
                  {run.evidence && (
                    <Button
                      colors={colors} variant="bare" type="button" flashSuccess={false}
                      aria-expanded={isOpen}
                      onClick={() => setExpanded(isOpen ? null : run.runId)}
                      style={{
                        '--pa-btn-fg': colors.textDim, '--pa-btn-fg-hover': colors.text,
                        '--pa-btn-pad': '0 4px', '--pa-btn-radius': `${radius.xs}px`,
                        fontSize: textSize.micro, fontFamily: font.body,
                      } as CSSProperties}
                    >
                      {isOpen ? 'Hide evidence' : 'Evidence'}
                    </Button>
                  )}
                  <Button
                    colors={colors} variant="bare" type="button" flashSuccess={false}
                    onClick={() => openTranscript(run.sessionId)}
                    style={{
                      '--pa-btn-fg': colors.cyan, '--pa-btn-fg-hover': colors.cyan,
                      '--pa-btn-pad': '0 4px', '--pa-btn-radius': `${radius.xs}px`,
                      fontSize: textSize.micro, fontFamily: font.body,
                    } as CSSProperties}
                  >
                    Open transcript
                  </Button>
                </div>
                {isOpen && run.evidence && (
                  <pre style={{
                    margin: 0, padding: `${space.md}px ${space.xl}px ${space.xl}px ${space.huge + space.xxl}px`,
                    fontFamily: font.mono, fontSize: textSize.micro, lineHeight: 1.6,
                    color: colors.textMuted, background: colors.codeBg,
                    whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
                  }}>
                    {run.evidence}
                  </pre>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
