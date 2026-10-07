import { Html } from '@react-three/drei';
import { useFrame } from '@react-three/fiber';
import { useRef, type ReactNode } from 'react';
import { Group, Vector3 } from 'three';

export const HABITAT_NAMEPLATE_DISTANCE_FACTOR = 4;
export const HABITAT_NAMEPLATE_MIN_DISTANCE = 1.5;
export const HABITAT_NAMEPLATE_FADE_BAND = .35;
export const HABITAT_NAMEPLATE_MAX_WIDTH_PX = 140;

export function HabitatNameplate({
  position,
  children,
}: {
  position: [number, number, number];
  children: ReactNode;
}) {
  const anchor = useRef<Group>(null);
  const label = useRef<HTMLDivElement>(null);
  const worldPosition = useRef(new Vector3());
  useFrame(({ camera }) => {
    if (!anchor.current || !label.current) return;
    anchor.current.localToWorld(worldPosition.current.set(...position));
    const distance = worldPosition.current.distanceTo(camera.position);
    const opacity = distance <= HABITAT_NAMEPLATE_MIN_DISTANCE
      ? 0
      : Math.min(1, (distance - HABITAT_NAMEPLATE_MIN_DISTANCE) / HABITAT_NAMEPLATE_FADE_BAND);
    label.current.style.opacity = String(opacity);
    label.current.style.visibility = opacity === 0 ? 'hidden' : 'visible';
  });
  return <group ref={anchor}>
    <Html position={position} center distanceFactor={HABITAT_NAMEPLATE_DISTANCE_FACTOR} zIndexRange={[20, 0]}>
      <div ref={label} className="habitat-nameplate" style={{ maxWidth: HABITAT_NAMEPLATE_MAX_WIDTH_PX }}>{children}</div>
    </Html>
  </group>;
}
