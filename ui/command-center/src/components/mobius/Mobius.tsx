import { useEffect, useRef, useState } from 'react';
import { useTheme } from '../../styles/useTheme';

export type MobiusState = 'idle' | 'thinking' | 'speaking' | 'calibrating' | 'sleeping';

interface MobiusProps {
  size?: number;
  state?: MobiusState;
  logoMode?: boolean;
  glow?: number;
  className?: string;
}

const FRAME_COUNT = 151;
const ASPECT = 1024 / 485; // source logo.webp dimensions

// Idle pulse: frames 109-123, each tripled for slow breathing (45 frames at 30fps = 1.5s)
const IDLE_FRAMES: number[] = [];
for (let f = 109; f <= 123; f++) { IDLE_FRAMES.push(f, f, f); }

const FPS: Record<MobiusState, number> = {
  idle: 30,
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
  const { mobiusGlow, idleAnim, reduceMotion } = useTheme();
  const [frame, setFrame] = useState(0);
  const rafRef = useRef<number>();
  const lastTime = useRef(0);

  // Apply appearance prefs
  const effectiveGlow = glow * (mobiusGlow / 100);
  const isIdle = state === 'idle';
  const idleDisabled = isIdle && (idleAnim === 'still' || reduceMotion);
  const isAnimated = state !== 'sleeping' && !idleDisabled;
  const fps = idleDisabled ? 0 : (FPS[state] || 0);

  // Preload frames once
  useEffect(() => {
    for (let i = 0; i < FRAME_COUNT; i++) {
      const img = new Image();
      img.src = frameSrc(i);
    }
  }, []);

  // rAF-driven frame cycling
  // idle: steps through IDLE_FRAMES sequence (triplicated 109-123)
  // other active states: loops all 0-150
  const idleIdxRef = useRef(0);
  useEffect(() => {
    if (!isAnimated || fps === 0) {
      setFrame(0);
      return;
    }
    idleIdxRef.current = 0;
    setFrame(isIdle ? IDLE_FRAMES[0] : 0);
    const interval = 1000 / fps;
    const tick = (now: number) => {
      if (now - lastTime.current >= interval) {
        if (isIdle) {
          idleIdxRef.current = (idleIdxRef.current + 1) % IDLE_FRAMES.length;
          setFrame(IDLE_FRAMES[idleIdxRef.current]);
        } else {
          setFrame(f => (f + 1) % FRAME_COUNT);
        }
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

  const glowOpacity = state === 'sleeping' ? 0 : effectiveGlow * 0.45;
  const glowSize = Math.round(size * 0.4);

  return (
    <div className={className} style={{
      position: 'relative', width, height,
      flexShrink: 0, display: 'inline-block',
    }}>
      {/* Glow: radial gradient behind the image, no rectangular edges */}
      {glowOpacity > 0 && (
        <div style={{
          position: 'absolute',
          top: '50%', left: '50%',
          width: size, height: size,
          transform: 'translate(-50%, -50%)',
          borderRadius: '50%',
          background: `radial-gradient(circle, rgba(0,213,255,${glowOpacity}) 0%, transparent 70%)`,
          filter: `blur(${glowSize}px)`,
          pointerEvents: 'none',
        }} />
      )}
      <img
        src={src}
        width={width}
        height={height}
        style={{
          position: 'relative',
          display: 'block',
          objectFit: 'contain',
          opacity: state === 'sleeping' ? 0.35 : 1,
          willChange: isAnimated ? 'contents' : undefined,
        }}
        alt="Permagent"
        draggable={false}
      />
    </div>
  );
}
