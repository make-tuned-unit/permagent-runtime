import { useRef, useState, useEffect } from 'react';
import type { CameraMode } from './types';
import { COLORS } from './constants';

interface WorldHUDProps {
  mode: CameraMode;
  showFps: boolean;
  hoveredStation: string | null;
  stationTooltip: string | null;
  /** Enter the Carved Cave descent (C-nav). */
  onDescend: () => void;
  /** Step down/up a stratum stop while descending. */
  onDescentStep: (dir: 1 | -1) => void;
  /** Ascend out of descent back to orbit. */
  onAscend: () => void;
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
  onDescend,
  onDescentStep,
  onAscend,
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
    fontSize: 12,
    color: COLORS.neonCyan,
    pointerEvents: 'none',
    zIndex: 10,
  };

  const badgeStyle: React.CSSProperties = {
    background: 'rgba(10, 14, 26, 0.8)',
    padding: '4px 10px',
    borderRadius: 4,
    border: `1px solid ${COLORS.neonCyan}30`,
    backdropFilter: 'blur(4px)',
  };

  // Interactive controls must re-enable pointer events (the HUD container disables them).
  const ctrlStyle: React.CSSProperties = {
    ...badgeStyle,
    cursor: 'pointer',
    pointerEvents: 'auto',
    color: COLORS.neonCyan,
    userSelect: 'none',
  };

  return (
    <>
      <div style={hudStyle}>
        <div style={badgeStyle}>
          {mode === 'orbit' ? 'ORBIT' : mode === 'descent' ? 'DESCENT' : 'FOLLOWING'}
          {mode === 'third-person' && (
            <span style={{ opacity: 0.6, marginLeft: 8 }}>ESC to exit</span>
          )}
          {mode === 'descent' && (
            <span style={{ opacity: 0.6, marginLeft: 8 }}>W/S · ESC to exit</span>
          )}
        </div>

        {/* Carved Cave descent affordance (C-nav, entry point A). */}
        {mode === 'orbit' && (
          <div style={ctrlStyle} onClick={onDescend} role="button" tabIndex={0}>
            ↓ Descend
          </div>
        )}
        {mode === 'descent' && (
          <div style={{ display: 'flex', gap: 6 }}>
            <div style={ctrlStyle} onClick={() => onDescentStep(-1)} role="button" tabIndex={0}>
              ▲
            </div>
            <div style={ctrlStyle} onClick={() => onDescentStep(1)} role="button" tabIndex={0}>
              ▼
            </div>
            <div style={ctrlStyle} onClick={onAscend} role="button" tabIndex={0}>
              ⤴ Ascend
            </div>
          </div>
        )}

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
            borderRadius: 8,
            fontFamily: 'monospace',
            fontSize: 14,
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
