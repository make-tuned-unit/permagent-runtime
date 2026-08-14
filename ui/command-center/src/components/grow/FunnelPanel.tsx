// Conversion funnel BUILDER — the consumer for
// GET /analytics/first_party/funnel and /step_options (backend:
// routes/first_party_analytics.rs → analytics_funnel.rs).
//
// This was a text box: you typed `/pricing,event:purchase` from memory and
// hoped. Steps you mistyped, or that the project never sent, produced an empty
// funnel indistinguishable from a real zero. The dropdowns are populated from
// the event names and paths this project has ACTUALLY recorded, so an empty
// funnel now means what it says.
//
// The panel presents, it never re-derives: ordering (a step counts only after
// every earlier one, in sequence), identity counting, and the medians all
// happen server-side where they are unit-tested. What this file owns is saying
// out loud what the numbers are made of — which identity is the denominator,
// that bots are excluded, and how many rows could not be sequenced.

import { useCallback, useEffect, useRef, useState } from 'react';
import { font, radius } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';

interface FunnelStep {
  label: string;
  /** Identities (see `identity`) that reached this step in order. */
  sessions: number;
  /** Identities lost between the previous step and this one. */
  dropped: number;
  stepRate: number | null;
  overallRate: number | null;
  /** MEDIAN seconds from the previous step. Median, not mean: one visitor who
   *  leaves a tab open overnight would otherwise define the number. */
  medianSecondsFromPrev: number | null;
}

export type FunnelIdentity = 'session' | 'visitor';

export interface FunnelData {
  steps: FunnelStep[];
  identity: FunnelIdentity;
  conversionRate: number;
  value: number;
  /** 1-based index of the step losing the most identities — "fix this first". */
  biggestDropStep: number | null;
  excludedNoIdentity: number;
  excludedBots: number;
}

interface NamedCount { name: string; count: number }
export interface StepOptions {
  events: NamedCount[];
  paths: NamedCount[];
  periodDays: number;
  includingBots: boolean;
}

export interface BuilderStep {
  type: 'path' | 'event';
  value: string;
}

const DAYS = 30;
/** The backend rejects more than eight steps (each one costs a scan). */
export const MAX_STEPS = 8;

/** Last-run steps per project, so the panel reopens with data instead of an
 *  empty form. localStorage, tolerant of private mode. */
const stepsStorageKey = (projectId: string) => `permagent-funnel-steps-${projectId}`;
const identityStorageKey = (projectId: string) => `permagent-funnel-identity-${projectId}`;

/** Wire form: `path:/pricing,event:purchase`. Prefixes are always explicit —
 *  the backend treats a bare value as a path, which silently turns a mistyped
 *  event name into a path that matches nothing. */
export function serializeSteps(steps: BuilderStep[]): string {
  return steps
    .filter((s) => s.value.trim() !== '')
    .map((s) => `${s.type}:${s.value.trim()}`)
    .join(',');
}

/** Parse the wire form back, including the legacy bare-path form written by
 *  the text box this replaced. */
export function parseSteps(raw: string): BuilderStep[] {
  return raw
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => {
      if (part.startsWith('event:')) return { type: 'event' as const, value: part.slice(6).trim() };
      if (part.startsWith('path:')) return { type: 'path' as const, value: part.slice(5).trim() };
      return { type: 'path' as const, value: part };
    })
    .filter((s) => s.value !== '');
}

/** Seconds → something a human reads at a glance. `null` stays absent rather
 *  than becoming a reassuring "0s". */
export function formatDuration(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds)) return null;
  if (seconds < 1) return '<1s';
  if (seconds < 60) return `${Math.round(seconds)}s`;
  if (seconds < 3600) {
    const m = Math.floor(seconds / 60);
    const s = Math.round(seconds - m * 60);
    return s > 0 ? `${m}m ${s}s` : `${m}m`;
  }
  if (seconds < 86_400) {
    const h = Math.floor(seconds / 3600);
    const m = Math.round((seconds - h * 3600) / 60);
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }
  const d = Math.floor(seconds / 86_400);
  const h = Math.round((seconds - d * 86_400) / 3600);
  return h > 0 ? `${d}d ${h}h` : `${d}d`;
}

/** What the number on screen counts. Stated, never implied. */
export function identityLabel(identity: FunnelIdentity): string {
  return identity === 'visitor' ? 'visitors' : 'sessions';
}

export function identityNote(identity: FunnelIdentity): string {
  return identity === 'visitor'
    ? 'Counting VISITORS: a daily-rotating hash of browser + language. Present on every row, but it merges people who share a device signature and resets at midnight UTC.'
    : 'Counting SESSIONS: one visit, one journey. Rows with no session id cannot be sequenced and are excluded below.';
}

type FunnelState = 'idle' | 'loading' | 'ready' | 'error';

export function FunnelPanel({ projectId, colors }: { projectId: string; colors: ThemeColors }) {
  const [steps, setSteps] = useState<BuilderStep[]>(() => {
    try { return parseSteps(localStorage.getItem(stepsStorageKey(projectId)) ?? ''); } catch { return []; }
  });
  const [identity, setIdentity] = useState<FunnelIdentity>(() => {
    try {
      return localStorage.getItem(identityStorageKey(projectId)) === 'visitor' ? 'visitor' : 'session';
    } catch { return 'session'; }
  });
  const [options, setOptions] = useState<StepOptions | null>(null);
  const [data, setData] = useState<FunnelData | null>(null);
  const [state, setState] = useState<FunnelState>('idle');
  const generation = useRef(0);

  const run = useCallback((next: BuilderStep[], nextIdentity: FunnelIdentity) => {
    const wire = serializeSteps(next);
    if (!wire) return;
    const gen = ++generation.current;
    setState('loading');
    apiFetch<FunnelData>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/funnel`
      + `?steps=${encodeURIComponent(wire)}&days=${DAYS}&identity=${nextIdentity}`,
    )
      .then((d) => {
        if (gen !== generation.current) return;
        setData(d);
        setState('ready');
        try {
          localStorage.setItem(stepsStorageKey(projectId), wire);
          localStorage.setItem(identityStorageKey(projectId), nextIdentity);
        } catch { /* private mode */ }
      })
      .catch(() => {
        if (gen !== generation.current) return;
        setState('error');
      });
  }, [projectId]);

  // The dropdown is only ever as good as the project's real history, so the
  // options come from the same rows the funnel is computed over.
  useEffect(() => {
    let live = true;
    apiFetch<StepOptions>(
      `/api/projects/${encodeURIComponent(projectId)}/analytics/first_party/step_options?days=${DAYS}`,
    )
      .then((o) => { if (live) setOptions(o); })
      .catch(() => { if (live) setOptions({ events: [], paths: [], periodDays: DAYS, includingBots: false }); });
    return () => { live = false; };
  }, [projectId]);

  // A previously built funnel re-runs on mount — revisiting the tab shows the
  // data, not the form. Editing steps deliberately does not refetch until Run.
  useEffect(() => {
    const saved = (() => {
      try { return localStorage.getItem(stepsStorageKey(projectId)) ?? ''; } catch { return ''; }
    })();
    const savedIdentity: FunnelIdentity = (() => {
      try { return localStorage.getItem(identityStorageKey(projectId)) === 'visitor' ? 'visitor' : 'session'; } catch { return 'session'; }
    })();
    const parsed = parseSteps(saved);
    if (parsed.length > 0) run(parsed, savedIdentity);
    return () => { ++generation.current; };
  }, [projectId, run]);

  const entered = data?.steps[0]?.sessions ?? 0;
  const unit = data ? identityLabel(data.identity) : identityLabel(identity);
  const hasOptions = !!options && (options.events.length > 0 || options.paths.length > 0);

  function updateStep(index: number, patch: Partial<BuilderStep>) {
    setSteps((prev) => prev.map((s, i) => (i === index ? { ...s, ...patch } : s)));
  }
  function removeStep(index: number) {
    setSteps((prev) => prev.filter((_, i) => i !== index));
  }
  function moveStep(index: number, delta: number) {
    setSteps((prev) => {
      const to = index + delta;
      if (to < 0 || to >= prev.length) return prev;
      const next = prev.slice();
      const [row] = next.splice(index, 1);
      next.splice(to, 0, row);
      return next;
    });
  }
  function addStep() {
    setSteps((prev) => {
      if (prev.length >= MAX_STEPS) return prev;
      // Default to the project's most common event, falling back to its most
      // common page — never to a made-up name.
      const firstEvent = options?.events[0]?.name;
      const firstPath = options?.paths[0]?.name;
      if (firstEvent) return [...prev, { type: 'event', value: firstEvent }];
      if (firstPath) return [...prev, { type: 'path', value: firstPath }];
      return [...prev, { type: 'event', value: '' }];
    });
  }

  const controlStyle: React.CSSProperties = {
    background: colors.bgDeeper, color: colors.text,
    border: `1px solid ${colors.border}`, borderRadius: radius.md,
    padding: '5px 8px', fontSize: 11, fontFamily: font.mono,
  };
  const buttonStyle: React.CSSProperties = {
    ...controlStyle, cursor: 'pointer', fontFamily: font.body, padding: '5px 10px',
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
            {Math.round(data.conversionRate * 100)}% of {unit} convert
            {data.value > 0 && <> · {data.value.toLocaleString()} value</>}
            {' '}· last {DAYS}d
          </span>
        )}
      </div>

      {/* ── Builder ─────────────────────────────────────────────────────── */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {steps.map((step, i) => (
          <div key={`step-${i}`} style={{ display: 'flex', gap: 6, alignItems: 'center', flexWrap: 'wrap' }}>
            <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, width: 16 }}>{i + 1}</span>
            <select
              aria-label={`Step ${i + 1} type`}
              value={step.type}
              onChange={(e) => updateStep(i, { type: e.target.value as BuilderStep['type'], value: '' })}
              style={controlStyle}
            >
              <option value="event">event</option>
              <option value="path">page</option>
            </select>
            <select
              aria-label={`Step ${i + 1}`}
              value={step.value}
              onChange={(e) => updateStep(i, { value: e.target.value })}
              style={{ ...controlStyle, flex: '1 1 220px', minWidth: 0 }}
            >
              <option value="">— pick {step.type === 'event' ? 'an event' : 'a page'} —</option>
              {(step.type === 'event' ? options?.events : options?.paths)?.map((o) => (
                <option key={o.name} value={o.name}>{o.name} ({o.count.toLocaleString()})</option>
              ))}
              {/* A saved step whose events stopped arriving must stay visible,
                  or Run would silently change the funnel being looked at. */}
              {step.value !== ''
                && !(step.type === 'event' ? options?.events : options?.paths)?.some((o) => o.name === step.value)
                ? <option value={step.value}>{step.value} (no events in {DAYS}d)</option>
                : null}
            </select>
            <button aria-label={`Move step ${i + 1} up`} disabled={i === 0} onClick={() => moveStep(i, -1)} style={buttonStyle}>↑</button>
            <button aria-label={`Move step ${i + 1} down`} disabled={i === steps.length - 1} onClick={() => moveStep(i, 1)} style={buttonStyle}>↓</button>
            <button aria-label={`Remove step ${i + 1}`} onClick={() => removeStep(i)} style={buttonStyle}>✕</button>
          </div>
        ))}
      </div>

      <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
        <button onClick={addStep} disabled={steps.length >= MAX_STEPS} style={buttonStyle}>+ Add step</button>
        <select
          aria-label="Count each step by"
          value={identity}
          onChange={(e) => setIdentity(e.target.value as FunnelIdentity)}
          style={controlStyle}
        >
          <option value="session">count sessions</option>
          <option value="visitor">count visitors</option>
        </select>
        <button
          onClick={() => run(steps, identity)}
          disabled={state === 'loading' || serializeSteps(steps) === ''}
          style={{ ...buttonStyle, opacity: state === 'loading' ? 0.6 : 1 }}
        >{state === 'loading' ? 'Computing…' : 'Run'}</button>
      </div>

      {options && !hasOptions && (
        <div style={{ fontSize: 11, color: colors.textDim }}>
          No events or pageviews recorded for this project in the last {DAYS} days — there is
          nothing to build a funnel from yet.
        </div>
      )}
      {state === 'idle' && hasOptions && (
        <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
          Pick an ordered sequence of steps. A {identityLabel(identity).slice(0, -1)} counts at a step
          only after passing every earlier one <em>in order</em> — a bookmark landing on /thanks
          never “converts”. Bots are excluded.
        </div>
      )}
      {state === 'error' && (
        <div style={{ fontSize: 11, color: colors.danger }}>
          Couldn’t compute the funnel — check the steps (1–{MAX_STEPS}) and try again.
        </div>
      )}

      {state === 'ready' && data && (
        <>
          {entered === 0 ? (
            <div style={{ fontSize: 11, color: colors.textDim }}>
              No {unit} entered this funnel in the last {DAYS} days.
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {data.steps.map((s, i) => {
                const isBiggestDrop = data.biggestDropStep === i + 1;
                const median = formatDuration(s.medianSecondsFromPrev);
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
                      minWidth: 190, flexShrink: 0, fontSize: 10, fontFamily: font.mono,
                      color: isBiggestDrop ? colors.warning : colors.textDim,
                    }}>
                      {i === 0
                        ? 'entered'
                        : <>
                            {Math.round((s.stepRate ?? 0) * 100)}% continued
                            {' · '}−{s.dropped.toLocaleString()} lost
                            {median && <> · median {median}</>}
                            {isBiggestDrop ? ' · biggest drop' : ''}
                          </>}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
          {/* A denominator nobody can name is not a measurement. Say what a bar
              counts, and what was filtered out of it. */}
          <div style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono, lineHeight: 1.5 }}>
            {identityNote(data.identity)}
            {data.excludedBots > 0 && (
              <> {data.excludedBots.toLocaleString()} bot row{data.excludedBots === 1 ? '' : 's'} excluded.</>
            )}
            {data.excludedNoIdentity > 0 && (
              <> {data.excludedNoIdentity.toLocaleString()} matching row{data.excludedNoIdentity === 1 ? '' : 's'} carried
              no {data.identity === 'visitor' ? 'visitor hash' : 'session id'} and couldn’t be sequenced — these
              figures are a floor, not a total.</>
            )}
          </div>
        </>
      )}
    </section>
  );
}
