import { Component, useEffect, useRef, type ReactNode } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { Button } from '../../common/Button';
import { radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';

export type ForumStageState = 'loading' | 'ready' | 'slow' | 'failed';
export const WORLD_READY_BUDGET_MS = 5000;

/** Outside Canvas/Suspense, so errors and suspended assets cannot hide recovery. */
export class ForumStageBoundary extends Component<{
  children: ReactNode; onFailure: () => void;
}, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch() { this.props.onFailure(); }
  render() { return this.state.failed ? null : this.props.children; }
}

/** A committed Canvas is not readiness: wait for its first active frame. */
export function ForumStageReady({ active, onReady, onFailure }: {
  active: boolean; onReady: () => void; onFailure: () => void;
}) {
  const reported = useRef(false);
  const canvas = useThree(state => state.gl.domElement);
  useFrame(() => {
    if (!active || reported.current) return;
    reported.current = true;
    onReady();
  });
  useEffect(() => {
    const lost = (event: Event) => { event.preventDefault(); reported.current = true; onFailure(); };
    canvas.addEventListener('webglcontextlost', lost);
    return () => canvas.removeEventListener('webglcontextlost', lost);
  }, [canvas, onFailure]);
  return null;
}

export function ForumStageStatus({ state, onRetry }: { state: ForumStageState; onRetry: () => void }) {
  const { colors } = useTheme();
  if (state === 'ready') return null;
  const recovery = state === 'slow' || state === 'failed';
  return <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', pointerEvents: 'none' }}>
    <div role={state === 'failed' ? 'alert' : 'status'} aria-live="polite" style={{
      padding: space.lg, margin: space.lg, borderRadius: radius.md,
      background: colors.surface, color: colors.text, fontSize: textSize.body,
    }}>
      <p>{state === 'failed' ? 'World could not start.' : state === 'slow' ? 'World is taking longer to load.' : 'Loading World…'}</p>
      <p style={{ color: colors.textMuted }}>{state === 'failed' ? 'The controls and the rest of the app still work.' : 'You can use the controls or navigate elsewhere while it loads.'}</p>
      {recovery && <Button colors={colors} flashSuccess={false} style={{ pointerEvents: 'auto' }} onClick={onRetry}>Retry World</Button>}
    </div>
  </div>;
}
