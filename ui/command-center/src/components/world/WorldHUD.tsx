import { useRef, useState, useEffect } from 'react';
import type { CameraMode } from './types';
import { COLORS } from './constants';
import { radius, textSize } from '../../styles/tokens';
import { CanvasLegend } from '../common/CanvasLegend';
import { worldGestures, worldVocabulary } from './worldLegend';

interface WorldHUDProps {
  mode: CameraMode;
  showFps: boolean;
  hoveredStation: string | null;
  stationTooltip: string | null;
}

function FpsCounter() {
  const [fps, setFps] = useState(0);
  const frames = useRef(0);
  const lastTime = useRef(performance.now());

  useEffect(() => {
    let rafId: number;
    const tick = () => {
      frames.current++;
      const now = performance.now();
      if (now - lastTime.current >= 1000) {
        setFps(frames.current);
        frames.current = 0;
        lastTime.current = now;
      }
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, []);

  return <span>{fps} FPS</span>;
}

export function WorldHUD({
  mode,
  showFps,
  hoveredStation,
  stationTooltip,
}: WorldHUDProps) {
  const hudStyle: React.CSSProperties = {
    position: 'absolute',
    bottom: 16,
    right: 16,
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'flex-end',
    gap: 8,
    fontFamily: 'monospace',
    fontSize: textSize.caption,
    color: COLORS.neonCyan,
    pointerEvents: 'none',
    zIndex: 10,
  };

  const badgeStyle: React.CSSProperties = {
    background: 'rgba(10, 14, 26, 0.8)',
    padding: '4px 10px',
    borderRadius: radius.xs,
    border: `1px solid ${COLORS.neonCyan}30`,
    backdropFilter: 'blur(4px)',
  };

  return (
    <>
      {/* The hall's key. It is here, in orbit — the mode every user starts in —
          because the badge below only ever said "WASD to walk" once the camera
          had already switched, which teaches the gesture to someone who has
          already been surprised by it. The badge keeps its reminder for while
          you are walking; the key is where you learn it first. */}
      <CanvasLegend
        canvasId="world"
        gestures={worldGestures(mode)}
        vocabulary={worldVocabulary()}
        palette={{
          bg: 'rgba(10, 14, 26, 0.86)',
          border: `${COLORS.neonCyan}30`,
          text: COLORS.primaryMarble,
          dim: `${COLORS.primaryMarble}99`,
          accent: COLORS.neonCyan,
        }}
      />

      <div style={hudStyle}>
        <div style={badgeStyle}>
          {mode === 'orbit' ? 'ORBIT' : 'WALKING'}
          {mode === 'third-person' && (
            <span style={{ opacity: 0.6, marginLeft: 8 }}>WASD to walk · ESC to exit</span>
          )}
        </div>

        {showFps && (
          <div style={badgeStyle}>
            <FpsCounter />
          </div>
        )}
      </div>

      {/* Station tooltip */}
      {hoveredStation && stationTooltip && (
        <div
          style={{
            position: 'absolute',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%, -50%)',
            background: 'rgba(10, 14, 26, 0.9)',
            color: stationTooltip.includes('coming soon') ? COLORS.neonAmber : COLORS.neonCyan,
            padding: '8px 16px',
            borderRadius: radius.md,
            fontFamily: 'monospace',
            fontSize: textSize.body,
            border: `1px solid ${COLORS.neonCyan}40`,
            pointerEvents: 'none',
            zIndex: 10,
            backdropFilter: 'blur(4px)',
          }}
        >
          {stationTooltip}
        </div>
      )}
    </>
  );
}
