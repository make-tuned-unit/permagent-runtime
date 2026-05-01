import { useEffect, useMemo, useRef, useState } from 'react';

export type MobiusState = 'idle' | 'thinking' | 'speaking' | 'calibrating' | 'sleeping';

interface MobiusProps {
  size?: number;
  state?: MobiusState;
  logoMode?: boolean;
  glow?: number;
  className?: string;
}

function lemniscatePoint(t: number, a: number): [number, number] {
  const denom = 1 + Math.sin(t) ** 2;
  return [(a * Math.cos(t)) / denom, (a * Math.sin(t) * Math.cos(t)) / denom];
}

function lemniscatePath(a: number, N = 240): string {
  const pts: [number, number][] = [];
  for (let i = 0; i <= N; i++) {
    const t = (i / N) * Math.PI * 2;
    pts.push(lemniscatePoint(t, a));
  }
  let d = `M ${pts[0][0].toFixed(2)} ${pts[0][1].toFixed(2)}`;
  for (let i = 1; i < pts.length; i++) {
    d += ` L ${pts[i][0].toFixed(2)} ${pts[i][1].toFixed(2)}`;
  }
  return d + ' Z';
}

interface ArcTable { lengths: number[]; total: number; N: number }

function buildArcTable(a: number, N = 720): ArcTable {
  const lengths = [0];
  let prev = lemniscatePoint(0, a);
  let total = 0;
  for (let i = 1; i <= N; i++) {
    const t = (i / N) * Math.PI * 2;
    const cur = lemniscatePoint(t, a);
    total += Math.hypot(cur[0] - prev[0], cur[1] - prev[1]);
    lengths.push(total);
    prev = cur;
  }
  return { lengths, total, N };
}

function progressToT(p: number, table: ArcTable): number {
  const target = p * table.total;
  let lo = 0, hi = table.lengths.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (table.lengths[mid] < target) lo = mid + 1;
    else hi = mid;
  }
  return (lo / table.N) * Math.PI * 2;
}

export function Mobius({ size = 280, state = 'idle', logoMode = false, glow = 1, className }: MobiusProps) {
  const a = 100;
  const path = useMemo(() => lemniscatePath(a), []);
  const arcTable = useMemo(() => buildArcTable(a), []);

  const [t, setT] = useState(0);
  const [progress, setProgress] = useState(0);
  const reduced = useRef(
    typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
  );

  useEffect(() => {
    let raf: number;
    let last = performance.now();
    let p = progress;
    const loop = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      setT(x => x + dt);
      let speed = 0.18;
      if (state === 'thinking') speed = 0.55;
      if (state === 'speaking') speed = 0.42;
      if (state === 'calibrating') speed = 0.35;
      if (state === 'sleeping') speed = 0;
      if (logoMode) speed = 0.25;
      if (reduced.current) speed = 0;
      p = (p + speed * dt) % 1;
      setProgress(p);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, logoMode]);

  let displayP = progress;
  if (state === 'speaking') {
    displayP = (progress + 0.06 * Math.sin(t * 8)) % 1;
    if (displayP < 0) displayP += 1;
  } else if (state === 'thinking') {
    displayP = (progress + 0.04 * Math.sin(t * 3)) % 1;
    if (displayP < 0) displayP += 1;
  }

  const tt = progressToT(displayP, arcTable);
  const [dx, dy] = lemniscatePoint(tt, a);

  let dotR = 6;
  if (state === 'thinking') dotR = 5 + 1.2 * Math.sin(t * 6);
  if (state === 'speaking') dotR = 6 + 1.5 * Math.sin(t * 9);
  if (state === 'calibrating') dotR = 7;
  if (state === 'sleeping') dotR = 4.5;
  if (logoMode) dotR = 4;

  const trailCount = 16;
  const trail: { x: number; y: number; r: number; alpha: number }[] = [];
  if (state !== 'sleeping') {
    for (let i = 1; i <= trailCount; i++) {
      let p2 = displayP - i * 0.012;
      if (p2 < 0) p2 += 1;
      const tT = progressToT(p2, arcTable);
      const [tx, ty] = lemniscatePoint(tT, a);
      trail.push({ x: tx, y: ty, r: dotR * (1 - i / trailCount) * 0.85, alpha: (1 - i / trailCount) * 0.55 });
    }
  }

  const stripStrokeW = logoMode ? 1.6 : 2.2;
  const stateGlow = state === 'sleeping' ? 0 : (state === 'thinking' ? 1.3 : state === 'speaking' ? 1.15 : 1) * glow;

  return (
    <div className={className} style={{ width: size, height: size, display: 'inline-block', position: 'relative' }}>
      <svg viewBox="-130 -75 260 150" width={size} height={size * (150 / 260)} style={{ display: 'block', overflow: 'visible' }}>
        <defs>
          <linearGradient id="mob-grad" x1="-100" y1="0" x2="100" y2="0" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#00D5FF" />
            <stop offset="55%" stopColor="#7BB7FF" />
            <stop offset="100%" stopColor="#A855F7" />
          </linearGradient>
          <radialGradient id="mob-dot-grad" cx="50%" cy="50%">
            <stop offset="0%" stopColor="#FFFFFF" />
            <stop offset="40%" stopColor="#9DEEFF" />
            <stop offset="100%" stopColor="#00D5FF" />
          </radialGradient>
          <radialGradient id="mob-amb" cx="50%" cy="50%">
            <stop offset="0%" stopColor="rgba(0,213,255,0.35)" />
            <stop offset="100%" stopColor="rgba(0,213,255,0)" />
          </radialGradient>
          <filter id="mob-blur"><feGaussianBlur stdDeviation="4" /></filter>
        </defs>

        {!logoMode && stateGlow > 0 && (
          <circle cx="0" cy="0" r="120" fill="url(#mob-amb)" opacity={0.5 * stateGlow} />
        )}

        <path
          d={path}
          stroke="url(#mob-grad)"
          strokeWidth={stripStrokeW}
          fill="none"
          strokeLinecap="round"
          opacity={state === 'sleeping' ? 0.35 : 1}
          style={{ filter: logoMode ? 'none' : state === 'sleeping' ? 'none' : 'drop-shadow(0 0 12px rgba(0,213,255,0.25))' }}
        />

        {trail.map((p, i) => (
          <circle key={i} cx={p.x} cy={p.y} r={p.r} fill="#00D5FF" opacity={p.alpha * stateGlow} />
        ))}

        {!logoMode && state !== 'sleeping' && glow > 0 && (
          <circle cx={dx} cy={dy} r={dotR * 3.5} fill="rgba(0,213,255,0.35)" opacity={0.55 * stateGlow} filter="url(#mob-blur)" />
        )}

        <circle
          cx={dx} cy={dy} r={dotR}
          fill="url(#mob-dot-grad)"
          style={{ filter: state === 'sleeping' ? 'none' : 'drop-shadow(0 0 6px rgba(0,213,255,0.95)) drop-shadow(0 0 18px rgba(0,213,255,0.6))' }}
        />
      </svg>
    </div>
  );
}
