// Echo — Henry resurfaces a *forgotten thread* from your Brain, unprompted.
//
// The magic of a second brain isn't storage, it's rediscovery: capture is easy,
// resurfacing is the hard, valuable part almost nothing does. Echo finds a
// "dormant thread" — an entity you wove through many memories, then went quiet
// on while newer memories piled up elsewhere — and gently offers it back.
//
// It is *honest*: it only appears when a genuinely substantial thread has truly
// gone dormant (≥3 mentions, ≥2-week gap, over a real span), with real numbers
// in the copy. Small or all-active Brains simply never see it — no forced magic.
// And it's *rare*: at most ~once a day, and Dismiss quiets it for three.

import { useEffect, useRef, useState } from 'react';
import { apiFetch } from '../../lib/api';
import type { BrainGraph, GraphEntity } from '../brain/useBrainData';
import { useTheme } from '../../styles/useTheme';
import { font } from '../../styles/tokens';
import { useCommandCenter, navigateToTool } from '../../lib/store';

const LS_KEY = 'permagent-echo-state';
const DAY = 86_400_000;
const SHOW_COOLDOWN = 20 * 3_600_000; // ~once a day
const DISMISS_COOLDOWN = 3 * DAY;

interface EchoState {
  lastShownAt?: number;
  lastShownEntity?: string;
  dismissedUntil?: number;
}

interface EchoPick {
  entity: GraphEntity;
  count: number;
  lastMs: number;
  gapDays: number;
}

function readState(): EchoState {
  try {
    return JSON.parse(localStorage.getItem(LS_KEY) || '{}');
  } catch {
    return {};
  }
}
function writeState(s: EchoState) {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(s));
  } catch {
    /* private mode — best effort */
  }
}

/** Pick the single most substantial dormant thread, or null if none qualifies. */
function pickEcho(graph: BrainGraph, avoidId?: string): EchoPick | null {
  const mems = graph.memories.filter(m => m.timestamp && Number.isFinite(Date.parse(m.timestamp)));
  if (mems.length < 8) return null; // need a real history before "dormant" means anything

  const newest = Math.max(...mems.map(m => Date.parse(m.timestamp)));
  let best: EchoPick | null = null;
  let bestScore = 0;

  for (const e of graph.entities) {
    if (e.id === avoidId) continue; // don't resurface the same thread twice in a row
    const times = mems
      .filter(m => Array.isArray(m.ent) && m.ent.includes(e.id))
      .map(m => Date.parse(m.timestamp));
    if (times.length < 3) continue;

    const last = Math.max(...times);
    const first = Math.min(...times);
    const gapDays = (newest - last) / DAY;
    const spanDays = (last - first) / DAY;
    if (gapDays < 14) continue; // still warm → not dormant
    if (spanDays < 2) continue; // a one-off burst, not a sustained thread

    // Favor substantial threads that have been quiet a while.
    const score = times.length * Math.log1p(gapDays);
    if (score > bestScore) {
      bestScore = score;
      best = { entity: e, count: times.length, lastMs: last, gapDays };
    }
  }
  return best;
}

function relTime(ms: number): string {
  const days = Math.floor((Date.now() - ms) / DAY);
  if (days >= 365) {
    const y = Math.round(days / 365);
    return y > 1 ? `${y} years ago` : 'a year ago';
  }
  if (days >= 45) return `${Math.round(days / 30)} months ago`;
  if (days >= 25) return 'a month ago';
  if (days >= 12) return `${Math.round(days / 7)} weeks ago`;
  return `${days} days ago`;
}

function usePrefersReducedMotion(): boolean {
  const [reduce, setReduce] = useState(false);
  useEffect(() => {
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    setReduce(mq.matches);
    const on = () => setReduce(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return reduce;
}

export function Echo() {
  const { colors } = useTheme();
  const sendMessage = useCommandCenter(s => s.sendMessage);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const reduce = usePrefersReducedMotion();

  const [pick, setPick] = useState<EchoPick | null>(null);
  const [visible, setVisible] = useState(false);
  const [drawn, setDrawn] = useState(false);
  const gradId = useRef(`echo-thread-${Math.round(performance.now())}`).current;

  // Decide-and-fetch once on mount, respecting the cooldowns.
  useEffect(() => {
    const st = readState();
    const now = Date.now();
    if (st.dismissedUntil && now < st.dismissedUntil) return;
    if (st.lastShownAt && now - st.lastShownAt < SHOW_COOLDOWN) return;

    let cancelled = false;
    (async () => {
      try {
        const graph = await apiFetch<BrainGraph>('/api/brain/graph');
        if (cancelled) return;
        const p = pickEcho(graph, st.lastShownEntity);
        if (p) {
          setPick(p);
          setVisible(true);
          writeState({ ...st, lastShownAt: now, lastShownEntity: p.entity.id });
        }
      } catch {
        /* Brain unreachable — no echo, no noise */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Draw the thread in after the card mounts (or instantly under reduced motion).
  useEffect(() => {
    if (!visible) return;
    if (reduce) {
      setDrawn(true);
      return;
    }
    const t = setTimeout(() => setDrawn(true), 80);
    return () => clearTimeout(t);
  }, [visible, reduce]);

  if (!pick || !visible) return null;

  const dismiss = () => {
    writeState({ ...readState(), dismissedUntil: Date.now() + DISMISS_COOLDOWN });
    setVisible(false);
  };
  const explore = () => navigateToTool('memory');
  const ask = () => {
    setActivePanel('chat');
    void sendMessage(
      `I left off with "${pick.entity.name}" a while back — you have ${pick.count} memories on it, last touched ${relTime(pick.lastMs)}. Remind me where I was, and what's worth picking back up.`,
    );
    setVisible(false);
  };

  return (
    <div
      role="note"
      aria-label={`Echo: a dormant thread — ${pick.entity.name}`}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 18,
        marginBottom: 20,
        padding: '14px 16px',
        borderRadius: 14,
        background: colors.surface,
        border: `1px solid ${colors.borderHi}`,
        boxShadow: colors.cardShadow,
        overflow: 'hidden',
      }}
    >
      {/* The signature "red string of memory" — a thread drawing itself from the
          dormant concept (then) toward now. */}
      <svg width="132" height="46" viewBox="0 0 132 46" style={{ flexShrink: 0 }} aria-hidden>
        <defs>
          <linearGradient id={gradId} x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stopColor={colors.cyan} />
            <stop offset="1" stopColor={colors.purple} />
          </linearGradient>
        </defs>
        <path
          d="M12 23 C 46 6, 86 40, 120 23"
          fill="none"
          stroke={`url(#${gradId})`}
          strokeWidth="2"
          strokeLinecap="round"
          pathLength={1}
          strokeDasharray={1}
          strokeDashoffset={drawn ? 0 : 1}
          style={{ transition: reduce ? undefined : 'stroke-dashoffset 900ms cubic-bezier(0.22,1,0.36,1)' }}
        />
        {/* then (dormant) */}
        <circle cx="12" cy="23" r="4.5" fill={colors.cyan}>
          {!reduce && <animate attributeName="opacity" values="0.55;1;0.55" dur="2.6s" repeatCount="indefinite" />}
        </circle>
        {/* now */}
        <circle cx="120" cy="23" r="4.5" fill={colors.purple} opacity={drawn ? 1 : 0} style={{ transition: 'opacity 500ms ease 700ms' }} />
      </svg>

      {/* The words, in Henry's voice */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontFamily: font.mono,
            fontSize: 10,
            letterSpacing: '0.14em',
            color: colors.textDim,
            marginBottom: 4,
          }}
        >
          ✦ ECHO
        </div>
        <div style={{ fontFamily: font.body, fontSize: 14, color: colors.text, lineHeight: 1.4 }}>
          You wove{' '}
          <button
            onClick={explore}
            style={{
              background: 'none',
              border: 'none',
              padding: 0,
              font: 'inherit',
              fontWeight: 700,
              color: colors.cyan,
              cursor: 'pointer',
            }}
          >
            {pick.entity.name}
          </button>{' '}
          through {pick.count} memories, then it went quiet — last touched {relTime(pick.lastMs)}.
          <span style={{ color: colors.textMuted }}> Threads like this are where the good ideas hide.</span>
        </div>
      </div>

      {/* Actions */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
        <button onClick={ask} style={primaryBtn(colors)}>Pick it back up</button>
        <button onClick={explore} style={ghostBtn(colors)}>Explore</button>
        <button onClick={dismiss} aria-label="Dismiss this echo" title="Not now" style={dismissBtn(colors)}>✕</button>
      </div>
    </div>
  );
}

function primaryBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '7px 14px',
    borderRadius: 9,
    border: `1px solid ${colors.cyan}`,
    background: colors.cyanSoft,
    color: colors.cyan,
    fontFamily: font.body,
    fontSize: 12,
    fontWeight: 600,
    cursor: 'pointer',
    whiteSpace: 'nowrap',
  };
}
function ghostBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '7px 12px',
    borderRadius: 9,
    border: `1px solid ${colors.border}`,
    background: 'transparent',
    color: colors.textMuted,
    fontFamily: font.body,
    fontSize: 12,
    fontWeight: 500,
    cursor: 'pointer',
    whiteSpace: 'nowrap',
  };
}
function dismissBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    width: 26,
    height: 26,
    display: 'grid',
    placeItems: 'center',
    borderRadius: 8,
    border: 'none',
    background: 'transparent',
    color: colors.textDim,
    fontSize: 12,
    cursor: 'pointer',
  };
}
