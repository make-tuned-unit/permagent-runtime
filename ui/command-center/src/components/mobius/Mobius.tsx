import { useEffect, useRef, useState } from 'react';

export type MobiusState = 'idle' | 'thinking' | 'speaking' | 'calibrating' | 'sleeping';

interface MobiusProps {
  size?: number;
  state?: MobiusState;
  logoMode?: boolean;
  glow?: number;
  className?: string;
}

const FRAME_COUNT = 151;
const IDLE_START = 109; // frames 109-123 forward then back = ping-pong pulse
const IDLE_END = 123;
const ASPECT = 1024 / 485; // source logo.webp dimensions
const FPS: Record<MobiusState, number> = {
  idle: 30,       // 30fps over frames 110-150 — central node pulse only
  thinking: 30,
  speaking: 24,
  calibrating: 20,
  sleeping: 0,
};

function frameSrc(n: number): string {
  return `/mobius/frame_${String(n).padStart(3, '0')}.webp`;
}

export function Mobius({
  size = 280,
  state = 'idle',
  logoMode: _logoMode = false,
  glow = 1,
  className,
}: MobiusProps) {
  const [frame, setFrame] = useState(0);
  const rafRef = useRef<number>();
  const lastTime = useRef(0);

  const isAnimated = state !== 'sleeping';
  const isIdle = state === 'idle';
  const fps = FPS[state] || 0;

  // Preload frames once
  useEffect(() => {
    for (let i = 0; i < FRAME_COUNT; i++) {
      const img = new Image();
      img.src = frameSrc(i);
    }
  }, []);

  // rAF-driven frame cycling
  // idle: loops 109→123, restarts at 109
  // other active states: loops all 0-150
  useEffect(() => {
    if (!isAnimated || fps === 0) {
      setFrame(0);
      return;
    }
    setFrame(isIdle ? IDLE_START : 0);
    const interval = 1000 / fps;
    const tick = (now: number) => {
      if (now - lastTime.current >= interval) {
        setFrame(f => {
          if (isIdle) {
            const next = f + 1;
            return next > IDLE_END ? IDLE_START : next;
          }
          return (f + 1) % FRAME_COUNT;
        });
        lastTime.current = now;
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    lastTime.current = performance.now();
    rafRef.current = requestAnimationFrame(tick);
    return () => { if (rafRef.current) cancelAnimationFrame(rafRef.current); };
  }, [isAnimated, isIdle, fps]);

  const src = isAnimated ? frameSrc(frame) : '/mobius/logo.webp';

  // size = height; width derives from natural aspect ratio of source asset
  const height = size;
  const width = Math.round(size * ASPECT);

  const glowPx = size * 0.12;
  const glowOpacity = state === 'sleeping' ? 0 : glow * 0.45;
  const filter = glowOpacity > 0
    ? `drop-shadow(0 0 ${glowPx}px rgba(0,213,255,${glowOpacity}))`
    : undefined;

  return (
    <img
      src={src}
      width={width}
      height={height}
      className={className}
      style={{
        display: 'block',
        objectFit: 'contain',
        flexShrink: 0,
        filter,
        opacity: state === 'sleeping' ? 0.35 : 1,
        willChange: isAnimated ? 'contents' : undefined,
      }}
      alt="Permagent"
      draggable={false}
    />
  );
}
