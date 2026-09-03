import { useState, useEffect, useRef, useCallback, type CSSProperties } from 'react';
import { COLORS } from './constants';
import { api, eventsWsUrl } from '../../lib/api';
import { wireEventType } from '../../lib/wireEvent';
import { HudShell, Section, StatRow } from './HudShell';
import { duration, ease, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

// ── Types ────────────────────────────────────────────────────────────

type LibrarianPhase = 'idle' | 'warming' | 'describing' | 'batch_complete' | 'error';

interface LibrarianStatus {
  state: LibrarianPhase;
  current_task: string;
  current_memory: { key: string; content_preview: string } | null;
  schedule: { next_window_start: string | null; window_duration_min: number };
  session_stats: {
    batch_started_at: string | null;
    memories_described_this_session: number;
    avg_seconds_per_memory: number | null;
  };
  lifetime_stats: { total_memories: number; described: number; pending: number };
  model: string;
  provider: string;
  error_message: string | null;
}

interface LibrarianRunStatus {
  running: boolean;
  started_at: string | null;
  finished_at: string | null;
  described: number | null;
  last_error: string | null;
}

// ── Phase badge colors ───────────────────────────────────────────────

function phaseColors(c: {
  fillSubtle: string; textMuted: string; border: string;
  staleSoft: string; warning: string;
  cyanSoft: string; cyan: string; borderHi: string;
  danger: string; dangerStrong: string;
}) {
  return {
    idle: { bg: c.fillSubtle, text: c.textMuted, border: c.border },
    warming: { bg: c.staleSoft, text: c.warning, border: c.warning },
    describing: { bg: c.cyanSoft, text: c.cyan, border: c.borderHi },
    batch_complete: { bg: c.cyanSoft, text: c.cyan, border: c.borderHi },
    error: { bg: `${c.dangerStrong}26`, text: c.dangerStrong, border: c.dangerStrong },
  } as const;
}

// ── WebSocket hook for token streaming ───────────────────────────────

interface StreamState {
  tokens: string;
  retrying: boolean;
  lastQuality: 'structured' | 'fallback' | null;
}

function useLibrarianTokenStream(active: boolean): StreamState {
  const [state, setState] = useState<StreamState>({ tokens: '', retrying: false, lastQuality: null });
  const wsRef = useRef<WebSocket | null>(null);
  const currentKeyRef = useRef<string | null>(null);

  useEffect(() => {
    if (!active) {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
      return;
    }

    // Daemon token rides the WS query (C1/C2 auth). The await opens an async
    // gap, so the cleanup closes via wsRef and `cancelled` guards against a
    // token load racing unmount opening an orphan socket.
    let cancelled = false;
    void (async () => {
      const url = await eventsWsUrl();
      if (cancelled) return;
      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onmessage = (ev) => {
        try {
          const event = JSON.parse(ev.data);
          const eventType = wireEventType(event);

          if (eventType === 'librarian_describe_started') {
            currentKeyRef.current = event.payload?.memory_key ?? null;
            setState({ tokens: '', retrying: false, lastQuality: null });
          } else if (eventType === 'librarian_describe_retry') {
            setState((prev) => ({ ...prev, tokens: '', retrying: true }));
          } else if (eventType === 'librarian_describe_token') {
            const key = event.payload?.memory_key;
            if (key === currentKeyRef.current) {
              setState((prev) => ({ ...prev, tokens: prev.tokens + (event.payload?.token ?? '') }));
            }
          } else if (eventType === 'librarian_describe_completed') {
            const quality = event.payload?.quality === 'fallback' ? 'fallback' as const : 'structured' as const;
            setState((prev) => ({ ...prev, retrying: false, lastQuality: quality }));
            currentKeyRef.current = null;
          }
        } catch {
          // ignore malformed
        }
      };

      ws.onerror = () => ws.close();
    })();

    return () => {
      cancelled = true;
      wsRef.current?.close();
      wsRef.current = null;
    };
  }, [active]);

  return state;
}

// ── Helpers ──────────────────────────────────────────────────────────

function formatRelativeTime(iso: string | null): string {
  if (!iso) return '—';
  const diff = new Date(iso).getTime() - Date.now();
  if (diff < 0) return 'now';
  const mins = Math.ceil(diff / 60000);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ${mins % 60}m`;
}

function phaseName(phase: LibrarianPhase): string {
  switch (phase) {
    case 'idle': return 'IDLE';
    case 'warming': return 'WARMING';
    case 'describing': return 'DESCRIBING';
    case 'batch_complete': return 'COMPLETE';
    case 'error': return 'ERROR';
  }
}

// ── Component ────────────────────────────────────────────────────────

interface LibrarianHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function LibrarianHUD({ visible, onClose }: LibrarianHUDProps) {
  const [status, setStatus] = useState<LibrarianStatus | null>(null);
  const [startingNow, setStartingNow] = useState(false);
  const [runStatus, setRunStatus] = useState<LibrarianRunStatus | null>(null);
  const [runMessage, setRunMessage] = useState<string | null>(null);
  // Named apart from the local phase palette below; the theme is here only to
  // feed the button primitive's variant defaults.
  const { colors: theme } = useTheme();

  const isDescribing = status?.state === 'describing' || status?.state === 'warming';
  const stream = useLibrarianTokenStream(visible && isDescribing);

  // Poll the detailed operational status at 1s.
  useEffect(() => {
    if (!visible) return;

    let cancelled = false;
    const poll = async () => {
      try {
        const s = await api.getLibrarianStatus();
        if (!cancelled) setStatus({ ...s, state: s.state as LibrarianPhase });
      } catch {
        // silently retry next tick
      }
    };
    poll();
    const id = setInterval(poll, 1000);
    return () => { cancelled = true; clearInterval(id); };
  }, [visible]);

  useEffect(() => {
    if (!visible) return;
    let cancelled = false;
    void api.getLibrarianRunStatus().then((next) => {
      if (!cancelled) setRunStatus(next);
    }).catch(() => {
      // The detailed HUD remains usable if run status is temporarily unavailable.
    });
    return () => { cancelled = true; };
  }, [visible]);

  // The manual-run endpoint only dispatches work. Poll its dedicated status
  // while the background batch is active, then surface its terminal result.
  useEffect(() => {
    if (!visible || !runStatus?.running) return;

    let cancelled = false;
    const poll = async () => {
      try {
        const next = await api.getLibrarianRunStatus();
        if (cancelled) return;
        setRunStatus(next);
        if (!next.running) {
          setRunMessage(next.last_error
            ? `Librarian run failed: ${next.last_error}`
            : `Librarian run complete — ${next.described ?? 0} described`);
        }
      } catch {
        // Keep the current state and retry on the next interval.
      }
    };
    const id = setInterval(poll, 3000);
    return () => { cancelled = true; clearInterval(id); };
  }, [visible, runStatus?.running]);

  // Returns the outcome so the Button primitive cannot tick success over the
  // "Unable to start…" line this same call puts on screen.
  const handleRunNow = useCallback(async () => {
    if (startingNow || runStatus?.running) return false;
    setStartingNow(true);
    try {
      const result = await api.runLibrarianNow();
      const next = await api.getLibrarianRunStatus();
      setRunStatus(next);
      setRunMessage(result.status === 'already_running'
        ? 'Librarian is already running'
        : 'Librarian run started');
      return true;
    } catch (error) {
      setRunMessage(error instanceof Error ? error.message : 'Unable to start Librarian run');
      return false;
    } finally {
      setStartingNow(false);
    }
  }, [startingNow, runStatus?.running]);

  if (!visible || !status) return null;

  const phase = (status.state as LibrarianPhase) || 'idle';
  const colors = phaseColors(theme)[phase] ?? phaseColors(theme).idle;
  const { lifetime_stats: lt, session_stats: ss, schedule } = status;
  const descPct = lt.total_memories > 0 ? Math.round((lt.described / lt.total_memories) * 100) : 0;

  const statusPill = (
    <div style={{
      display: 'inline-block',
      padding: `${space.xxs}px ${space.md}px`,
      borderRadius: radius.xs,
      fontSize: textSize.micro,
      fontWeight: 700,
      letterSpacing: '0.08em',
      background: colors.bg,
      color: colors.text,
      border: `1px solid ${colors.border}`,
    }}>
      {phaseName(phase)}
    </div>
  );

  return (
    <HudShell
      visible={visible}
      onClose={onClose}
      title="THE LIBRARIAN"
      statusPill={statusPill}
    >
      {/* Phase task description */}
      <div style={{ padding: `${space.xs}px 14px ${space.md}px` }}>
        <span style={{ fontSize: textSize.micro, color: theme.textMuted }}>
          {status.current_task}
        </span>
      </div>

      {/* Current memory + streaming tokens */}
      {(status.current_memory || stream.tokens) && (
        <Section title="CURRENT MEMORY" trimColor={COLORS.neonAmber}>
          {status.current_memory && (
            <div style={{ fontSize: textSize.micro, color: theme.text, marginBottom: space.xs }}>
              <span style={{ color: COLORS.neonAmber, fontWeight: 600 }}>
                {status.current_memory.key}
              </span>
              {stream.retrying && (
                <span style={{ color: COLORS.neonAmber, fontSize: textSize.micro, marginLeft: space.md, opacity: 0.8 }}>
                  retrying…
                </span>
              )}
              {stream.lastQuality === 'fallback' && (
                <span style={{ color: theme.dangerStrong, fontSize: textSize.micro, marginLeft: space.md }}>
                  low quality
                </span>
              )}
              <div style={{ color: theme.textMuted, marginTop: space.xxs }}>
                {status.current_memory.content_preview}
              </div>
            </div>
          )}
          {stream.tokens && (
            <div style={{
              fontSize: textSize.micro,
              color: COLORS.neonCyan,
              whiteSpace: 'pre-wrap',
              maxHeight: 80,
              overflowY: 'auto',
              fontStyle: 'italic',
              marginTop: space.xs,
              lineHeight: 1.4,
            }}>
              {stream.tokens}
              <span style={cursorStyle}>▌</span>
            </div>
          )}
        </Section>
      )}

      {/* Stats */}
      <Section title="MEMORY STATS" trimColor={COLORS.neonCyan}>
        <StatRow label="Total memories" value={lt.total_memories} />
        <StatRow label="Described" value={`${lt.described} (${descPct}%)`} />
        <StatRow label="Pending" value={lt.pending} />
        {/* Progress bar */}
        <div style={{ marginTop: space.sm, height: 4, background: theme.fillSubtle, borderRadius: radius.xs }}>
          <div style={{
            height: '100%',
            width: `${descPct}%`,
            background: `linear-gradient(90deg, ${COLORS.neonCyan}80, ${COLORS.neonCyan})`,
            borderRadius: radius.xs,
            transition: `width ${duration.smooth}ms ${ease.smooth}`,
          }} />
        </div>
      </Section>

      {/* Session */}
      {ss.batch_started_at && (
        <Section title="SESSION" trimColor={COLORS.neonAmber}>
          <StatRow label="Described this session" value={ss.memories_described_this_session} />
          <StatRow
            label="Avg per memory"
            value={ss.avg_seconds_per_memory != null ? `${ss.avg_seconds_per_memory.toFixed(1)}s` : '—'}
          />
        </Section>
      )}

      {/* Schedule */}
      <Section title="SCHEDULE" trimColor={COLORS.marbleVeining}>
        <StatRow label="Model" value={status.model} />
        <StatRow label="Provider" value={status.provider} />
        <StatRow label="Next window" value={formatRelativeTime(schedule.next_window_start)} />
        <StatRow label="Window duration" value={`${schedule.window_duration_min}m`} />
      </Section>

      {/* Error */}
      {status.error_message && (
        <Section title="ERROR" trimColor={theme.dangerStrong}>
          <div style={{ fontSize: textSize.micro, color: theme.dangerStrong, wordBreak: 'break-word' }}>
            {status.error_message}
          </div>
        </Section>
      )}

      {/* Actions */}
      <div style={{ padding: `${space.md}px 14px ${space.xl}px` }}>
        {runMessage && (
          <div style={{ fontSize: textSize.micro, color: runStatus?.last_error ? theme.dangerStrong : COLORS.neonCyan, marginBottom: space.sm }}>
            {runMessage}
          </div>
        )}
        <Button
          colors={theme}
          type="button"
          onClick={handleRunNow}
          disabled={startingNow || runStatus?.running || phase === 'describing' || phase === 'warming'}
          style={actionBtnVars}
        >
          {startingNow ? 'Starting…' : runStatus?.running ? 'Running…' : 'Run Now'}
        </Button>
      </div>
    </HudShell>
  );
}

// ── Styles ───────────────────────────────────────────────────────────

const actionBtnVars = {
  '--pa-btn-bg': `${COLORS.neonCyan}15`,
  '--pa-btn-fg': COLORS.neonCyan,
  '--pa-btn-border': `${COLORS.neonCyan}40`,
  '--pa-btn-bg-hover': `${COLORS.neonCyan}26`,
  '--pa-btn-border-hover': `${COLORS.neonCyan}70`,
  '--pa-btn-bg-active': `${COLORS.neonCyan}15`,
  '--pa-btn-pad': '6px 0',
  '--pa-btn-radius': `${radius.xs}px`,
  '--pa-btn-weight': 600,
  width: '100%',
  fontSize: textSize.micro,
  fontFamily: 'monospace',
  letterSpacing: '0.05em',
} as CSSProperties;

const cursorStyle: React.CSSProperties = {
  animation: 'blink 1s step-end infinite',
};

// Inject blink keyframes once
if (typeof document !== 'undefined' && !document.getElementById('librarian-hud-styles')) {
  const style = document.createElement('style');
  style.id = 'librarian-hud-styles';
  style.textContent = `@keyframes blink { 50% { opacity: 0; } }`;
  document.head.appendChild(style);
}
