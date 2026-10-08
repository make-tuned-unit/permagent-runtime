import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { useGLTF } from '@react-three/drei';
import { useEffect, useMemo, useState } from 'react';
import { textSize } from '../../../styles/tokens';
import { useForumReducedMotion } from './useForumReducedMotion';
import { GardenAssetBoundary } from './GardenAssetBoundary';
import { extendForumLoader } from './forumGltf';
import { disposeHeroRobot, buildHeroRobot, HERO_KIT_URL, heroProportions } from './heroRobot';

type PortraitSize = 'desk' | 'dock' | 'profile';

function DemandIdle({ reducedMotion }: { reducedMotion: boolean }) {
  const invalidate = useThree(state => state.invalidate);
  useEffect(() => {
    if (reducedMotion) return;
    let stopped = false;
    let timer: number | undefined;
    const requestFrame = () => {
      if (stopped || document.hidden) return;
      invalidate();
      timer = window.setTimeout(requestFrame, 100);
    };
    timer = window.setTimeout(requestFrame, 100);
    return () => {
      stopped = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [invalidate, reducedMotion]);
  return null;
}

function PortraitRobot({ id, reducedMotion }: { id: string; reducedMotion: boolean }) {
  const { scene } = useGLTF(HERO_KIT_URL, false, false, extendForumLoader);
  const robot = useMemo(() => buildHeroRobot(scene, id), [scene, id]);
  useEffect(() => () => disposeHeroRobot(robot), [robot]);
  useFrame(({ clock }, delta) => {
    if (!robot.head) return;
    const target = reducedMotion ? 0 : Math.sin(clock.elapsedTime * .3) * .12;
    robot.head.rotation.y += (target - robot.head.rotation.y) * (1 - Math.exp(-delta * 4));
  });
  return <group scale={heroProportions(id)} position={[0, -.92, 0]}>
    <primitive object={robot.object} dispose={null} />
  </group>;
}

function PortraitScene({ id, reducedMotion }: { id: string; reducedMotion: boolean }) {
  return <>
    <DemandIdle reducedMotion={reducedMotion} />
    <ambientLight intensity={1.35} />
    <directionalLight position={[-2.2, 3.2, 3.4]} intensity={2.4} color="#ffe2b8" />
    <directionalLight position={[2.4, 1.8, 2]} intensity={1.25} color="#b9d9ed" />
    <PortraitRobot id={id} reducedMotion={reducedMotion} />
  </>;
}

function useDocumentVisible() {
  const [visible, setVisible] = useState(() => typeof document === 'undefined' || !document.hidden);
  useEffect(() => {
    const update = () => setVisible(!document.hidden);
    document.addEventListener('visibilitychange', update);
    return () => document.removeEventListener('visibilitychange', update);
  }, []);
  return visible;
}

export function HeroAgentPortrait({ id, size, label }: { id: string; size: PortraitSize; label?: string }) {
  const visible = useDocumentVisible();
  const reducedMotion = useForumReducedMotion();
  const [failedId, setFailedId] = useState<string | null>(null);
  const failed = failedId === id;
  const dimension = size === 'dock' ? 28 : size === 'profile' ? 112 : 220;
  return <div
    className={`hero-agent-portrait hero-agent-portrait-${size}`}
    data-testid="live-agent-portrait"
    aria-label={failed ? 'Portrait unavailable' : label}
    role={label ? 'img' : undefined}
    aria-hidden={label ? undefined : true}
  >
    {visible && <GardenAssetBoundary key={id} onFailure={() => setFailedId(id)} fallback={<div className="agent-portrait-fallback" style={{ width: dimension, height: size === 'dock' ? 28 : size === 'profile' ? 136 : 260, display: 'grid', placeContent: 'center', textAlign: 'center', fontSize: textSize.micro, color: 'var(--forum-muted)' }}><span aria-hidden="true">◇</span>{size !== 'dock' && <span>Portrait unavailable</span>}</div>}><Canvas
      frameloop="demand"
      dpr={[1, 1.5]}
      camera={{ position: [0, .88, 3.2], fov: 31, near: .1, far: 20 }}
      gl={{ alpha: true, antialias: true }}
      style={{ width: dimension, height: size === 'dock' ? 28 : size === 'profile' ? 136 : 260, background: 'transparent' }}
    >
      <PortraitScene id={id} reducedMotion={reducedMotion} />
    </Canvas></GardenAssetBoundary>}
  </div>;
}
