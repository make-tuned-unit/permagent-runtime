// Conversion funnel — the consumer for GET /analytics/first_party/funnel
// (backend: routes/first_party_analytics.rs → analytics_funnel.rs). The
// endpoint enforces the two rules that make a funnel believable — ORDER
// MATTERS (a session counts at step N only having hit 1..N in sequence) and
// SESSIONS, NOT EVENTS — so this panel only presents, never re-derives.
//
// Wire shape mirrors the Rust `Funnel` (camelCase serde). Rows the backend
// could not sequence (no session id) arrive in `excludedSessionless` and are
// surfaced, not hidden: a silently-filtered denominator is how analytics
// tools lie.

import { useCallback, useEffect, useRef, useState } from 'react';
import { font, radius } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';

interface FunnelStep {
  label: string;
  sessions: number;
  dropped: number;
  stepRate: number | null;
  overallRate: number | null;
}

export interface FunnelData {
  steps: FunnelStep[];
  conversionRate: number;
  value: number;
  /** 1-based index of the step losing the most sessions — "fix this first". */
  biggestDropStep: number | null;
  excludedSessionless: number;
}

/** Last-run steps per project, so the panel reopens with data instead of an
 *  empty form. localStorage, tolerant of private mode. */
const stepsStorageKey = (projectId: string) => `permagent-funnel-steps-${projectId}`;

type FunnelState = 'idle' | 'loading' | 'ready' | 'error';

export function FunnelPanel({ projectId, colors }: { projectId: string; colors: ThemeColors }) {
  const [steps, setSteps] = useState(() => {
    try { return localStorage.getItem(stepsStorageKey(projectId)) ?? ''; } catch { return ''; }
  });
  const [data, setData] = useState<FunnelData | null>(null);
  const [state, setState] = useState<FunnelState>('idle');
  const generation = useRef(0);

  const run = useCallback((raw: string) => {
    const trimmed = raw.trim();
    if (!trimmed) return;
    const gen = ++generation.current;
    setState('loading');
    apiFetch<FunnelData>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/funnel?steps=${encodeURIComponent(trimmed)}&days=30`,
    )
      .then((d) => {
        if (gen !== generation.current) return;
        setData(d);
        setState('ready');
        try { localStorage.setItem(stepsStorageKey(projectId), trimmed); } catch { /* private mode */ }
      })
      .catch(() => {
        if (gen !== generation.current) return;
        setState('error');
      });
  }, [projectId]);

  // A previously run funnel re-runs on mount — revisiting the tab shows the
  // data, not the form. Steps are deliberately not a dep: retyping must not
  // refetch until Run is pressed.
  useEffect(() => {
    const saved = (() => {
      try { return localStorage.getItem(stepsStorageKey(projectId)) ?? ''; } catch { return ''; }
    })();
    if (saved.trim()) run(saved);
    return () => { ++generation.current; };
  }, [projectId, run]);

  const entered = data?.steps[0]?.sessions ?? 0;

  const inputStyle: React.CSSProperties = {
    flex: '1 1 260px', background: colors.bgDeeper, color: colors.text,
    border: `1px solid ${colors.border}`, borderRadius: radius.md,
    padding: '6px 10px', fontSize: 12, fontFamily: font.mono,
  };

  return (
    <section style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: 16, display: 'flex', flexDirection: 'column', gap: 10,
    }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, flexWrap: 'wrap' }}>
        <h3 style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', margin: 0 }}>
          Conversion funnel
        </h3>
        {state === 'ready' && data && entered > 0 && (
          <span style={{ fontSize: 11, color: colors.textMuted, fontFamily: font.mono }}>
            {Math.round(data.conversionRate * 100)}% convert
            {data.value > 0 && <> · {data.value.toLocaleString()} value</>}
            {' '}· last 30d
          </span>
        )}
      </div>

      <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
        <input
          value={steps}
          onChange={(e) => setSteps(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') run(steps); }}
          placeholder="/pricing,event:checkout_started,event:purchase"
          aria-label="Funnel steps"
          style={inputStyle}
        />
        <button
          onClick={() => run(steps)}
          disabled={state === 'loading' || !steps.trim()}
          style={{
            background: colors.bgDeeper, color: colors.text, border: `1px solid ${colors.border}`,
            borderRadius: radius.md, padding: '6px 12px', fontSize: 12, fontFamily: font.body,
            cursor: state === 'loading' || !steps.trim() ? 'default' : 'pointer',
            opacity: state === 'loading' ? 0.6 : 1,
          }}
        >{state === 'loading' ? 'Computing…' : 'Run'}</button>
      </div>

      {state === 'idle' && (
        <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
          Ordered steps, comma-separated: a path (<code>/pricing</code>) or a named event
          (<code>event:purchase</code>). Sessions count at a step only after passing every
          earlier one in order — a bookmark landing on /thanks never “converts”.
        </div>
      )}
      {state === 'error' && (
        <div style={{ fontSize: 11, color: colors.danger }}>
          Couldn’t compute the funnel — check the steps (1–8, comma-separated) and try again.
        </div>
      )}

      {state === 'ready' && data && (
        <>
          {entered === 0 ? (
            <div style={{ fontSize: 11, color: colors.textDim }}>
              No sessions entered this funnel in the last 30 days.
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {data.steps.map((s, i) => {
                const isBiggestDrop = data.biggestDropStep === i + 1;
                return (
                  <div key={`${s.label}-${i}`} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                    <div style={{
                      width: 140, fontSize: 11, color: colors.textMuted, textAlign: 'right',
                      flexShrink: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                      fontFamily: font.mono,
                    }} title={s.label}>{s.label}</div>
                    <div style={{ flex: 1, height: 22, background: colors.bgDeeper, borderRadius: radius.sm, overflow: 'hidden' }}>
                      <div style={{
                        width: `${Math.max(s.sessions > 0 ? 4 : 0, (s.sessions / entered) * 100)}%`,
                        height: '100%',
                        background: isBiggestDrop
                          ? colors.warning
                          : `linear-gradient(90deg, ${colors.cyan}, ${colors.purple})`,
                        borderRadius: radius.sm,
                        opacity: isBiggestDrop ? 0.75 : 0.9,
                      }} />
                    </div>
                    <div style={{
                      minWidth: 44, textAlign: 'right', flexShrink: 0, fontFamily: font.mono,
                      fontSize: 12, color: colors.text, fontVariantNumeric: 'tabular-nums',
                    }}>{s.sessions.toLocaleString()}</div>
                    <div style={{
                      minWidth: 88, flexShrink: 0, fontSize: 10, fontFamily: font.mono,
                      color: isBiggestDrop ? colors.warning : colors.textDim,
                    }}>
                      {i === 0
                        ? 'entered'
                        : `${Math.round((s.stepRate ?? 0) * 100)}%${isBiggestDrop ? ' · biggest drop' : ''}`}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
          {/* A silently-filtered denominator is how analytics tools lie — say
              what could not be sequenced (mirrors the botsExcluded pattern). */}
          {data.excludedSessionless > 0 && (
            <div style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono }}>
              {data.excludedSessionless.toLocaleString()} matching row{data.excludedSessionless === 1 ? '' : 's'} carried
              no session id and couldn’t be sequenced — these figures are a floor, not a total.
            </div>
          )}
        </>
      )}
    </section>
  );
}
