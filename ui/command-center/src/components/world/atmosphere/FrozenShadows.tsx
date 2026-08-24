// W4 atmosphere — the shadow map does not need rebuilding thirty times a second.
//
// The scene has exactly one shadow-casting light (bible §1) and its 2048² depth
// pass re-renders every caster in the hall on every single frame. That is the
// most expensive thing in the frame that changes the least: a census of the 55
// `castShadow` sites finds all of them on static architecture — columns, domes,
// benches, plinths, zone blockouts. Nothing that moves casts a shadow. The
// agents do not.
//
// So the shadow map is frozen and rebuilt only when something could actually
// have changed it:
//
//   - the key light drifts. `atmosphere/timeOfDay` damps its colour and
//     intensity toward the real local hour, which moves the shadows — but over
//     minutes, not frames.
//   - geometry appears. Zones lazy-load into the scene graph (bible §3), and a
//     newly mounted colonnade with no shadow would be very obvious.
//   - the slow fallback below, in case something changes that neither of those
//     two signals catches.
//
// Everything here reads state that is already being computed; it allocates
// nothing per frame, per bible §8.

import { useEffect, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { getTimeOfDay } from './timeOfDay';

// The key light's intensity is damped toward its target, so it is a continuous
// value: this is how far it must move before the shadows are worth redrawing.
const KEY_INTENSITY_EPSILON = 0.02;
// Belt and braces. Cheap at this cadence, and it means a missed signal costs a
// few seconds of stale shadow rather than a permanently wrong-looking hall.
const FALLBACK_INTERVAL_S = 5;
// The first seconds after mount are when zones, props and agents are still
// arriving, so redraw generously until the scene settles.
const SETTLE_S = 6;

export function FrozenShadows() {
  const gl = useThree((s) => s.gl);
  const lastKey = useRef(-1);
  const lastGeometries = useRef(-1);
  const lastRefresh = useRef(0);
  const start = useRef(-1);

  useEffect(() => {
    const previous = gl.shadowMap.autoUpdate;
    gl.shadowMap.autoUpdate = false;
    gl.shadowMap.needsUpdate = true;
    return () => {
      gl.shadowMap.autoUpdate = previous;
      gl.shadowMap.needsUpdate = true;
    };
  }, [gl]);

  useFrame(({ clock }) => {
    const t = clock.elapsedTime;
    if (start.current < 0) start.current = t;

    if (t - start.current < SETTLE_S) {
      gl.shadowMap.needsUpdate = true;
      lastRefresh.current = t;
      return;
    }

    const key = getTimeOfDay().keyIntensity;
    const geometries = gl.info.memory.geometries;
    const drifted = Math.abs(key - lastKey.current) > KEY_INTENSITY_EPSILON;
    const grew = geometries !== lastGeometries.current;
    const stale = t - lastRefresh.current > FALLBACK_INTERVAL_S;

    if (drifted || grew || stale) {
      gl.shadowMap.needsUpdate = true;
      lastKey.current = key;
      lastGeometries.current = geometries;
      lastRefresh.current = t;
    }
  });

  return null;
}
