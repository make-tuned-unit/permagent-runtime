import { useRef, useState, useEffect, type CSSProperties } from 'react';
import type { CameraMode } from './types';
import { font, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { CanvasLegend } from '../common/CanvasLegend';
import { worldGestures, worldVocabulary } from './worldLegend';
import {
  HUD_GEOM,
  HUD_PANEL_RADIUS,
  HUD_PILL_RADIUS,
  hudTransition,
} from './hudChrome';

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
  const { colors, reduceMotion } = useTheme();
  const glass = useGlass('glass');

  const hudStyle: CSSProperties = {
    position: 'absolute',
    bottom: HUD_GEOM.panelInset,
    right: HUD_GEOM.panelInset,
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'flex-end',
    gap: space.md,
    fontFamily: font.mono,
    fontSize: textSize.caption,
    color: colors.cyan,
    pointerEvents: 'none',
    zIndex: 10,
  };

  const badgeStyle: CSSProperties = {
    ...glass,
    padding: `${HUD_GEOM.badgePadY}px ${HUD_GEOM.badgePadX}px`,
    borderRadius: HUD_PILL_RADIUS > 0 ? HUD_PILL_RADIUS : HUD_PANEL_RADIUS,
    border: `1px solid ${colors.borderHi}`,
    transition: hudTransition(reduceMotion),
  };

  return (
    <>
      {/* The hall's key. It is here, in orbit — the mode every user starts in —
          because the badge below only ever said "WASD to walk" once the camera
          had already switched, which teaches the gesture to someone who has
          already been surprised by it. The badge keeps its reminder for while
          you are walking; the key is where you learn it first.
          Palette omitted: CanvasLegend takes theme + glass on its own plane. */}
      <CanvasLegend
        canvasId="world"
        gestures={worldGestures(mode)}
        vocabulary={worldVocabulary()}
      />

      <div style={hudStyle}>
        <div style={badgeStyle}>
          {mode === 'orbit' ? 'ORBIT' : 'WALKING'}
          {mode === 'third-person' && (
            <span style={{ opacity: 0.6, marginLeft: space.md }}>WASD to walk · ESC to exit</span>
          )}
        </div>

        {showFps && (
          <div style={badgeStyle}>
            <FpsCounter />
          </div>
        )}
      </div>

      {hoveredStation && stationTooltip && (
        <div
          style={{
            position: 'absolute',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%, -50%)',
            ...glass,
            color: stationTooltip.includes('coming soon') ? colors.warning : colors.cyan,
            padding: `${space.md}px ${space.xxl}px`,
            borderRadius: HUD_PANEL_RADIUS,
            fontFamily: font.mono,
            fontSize: textSize.body,
            border: `1px solid ${colors.borderHi}`,
            pointerEvents: 'none',
            zIndex: 10,
          }}
        >
          {stationTooltip}
        </div>
      )}
    </>
  );
}
