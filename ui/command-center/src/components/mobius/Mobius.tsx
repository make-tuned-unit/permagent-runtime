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
const ASPECT = 1024 / 485; // source logo.webp dimensions (loading/active frames)
const IDLE_FRAME_COUNT = 63;
const IDLE_ASPECT = 1024 / 576; // idle pulse frames

const FPS: Record<MobiusState, number> = {
  idle: 13,       // 63 frames at 80ms each ≈ 12.5fps
  thinking: 30,
  speaking: 24,
  calibrating: 20,
  sleeping: 0,
};

function frameSrc(n: number): string {
  return `/mobius/frame_${String(n).padStart(3, '0')}.webp`;
}

function idleFrameSrc(n: number): string {
  return `/mobius-idle/frame_${String(n).padStart(2, '0')}.webp`;
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

  // Preload frames once per set
  useEffect(() => {
    if (isIdle) {
      for (let i = 0; i < IDLE_FRAME_COUNT; i++) {
        const img = new Image();
        img.src = idleFrameSrc(i);
      }
    } else {
      for (let i = 0; i < FRAME_COUNT; i++) {
        const img = new Image();
        img.src = frameSrc(i);
      }
    }
  }, [isIdle]);

  // rAF-driven frame cycling
  // idle: loops 63 dedicated idle frames
  // other active states: loops all 0-150 loading frames
  useEffect(() => {
    if (!isAnimated || fps === 0) {
      setFrame(0);
      return;
    }
    setFrame(0);
    const interval = 1000 / fps;
    const count = isIdle ? IDLE_FRAME_COUNT : FRAME_COUNT;
    const tick = (now: number) => {
      if (now - lastTime.current >= interval) {
        setFrame(f => (f + 1) % count);
        lastTime.current = now;
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    lastTime.current = performance.now();
    rafRef.current = requestAnimationFrame(tick);
    return () => { if (rafRef.current) cancelAnimationFrame(rafRef.current); };
  }, [isAnimated, isIdle, fps]);

  const src = isAnimated
    ? (isIdle ? idleFrameSrc(frame) : frameSrc(frame))
    : '/mobius/logo.webp';

  // size = height; width derives from natural aspect ratio of the active frame set
  const aspect = isIdle ? IDLE_ASPECT : ASPECT;
  const height = size;
  const width = Math.round(size * aspect);

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
