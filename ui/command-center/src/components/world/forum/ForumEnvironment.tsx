import { useEffect, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import { environmentIntensity, faceUrls, loadEnvironmentManifest, type Appearance } from './bakedEnvironment';

type Environment = { target: THREE.WebGLRenderTarget; intensity: number };
const noop = () => {};

/** Two owned environment targets per Canvas at most. Keep the previous usable
 * map while loading; never regenerate/dispose it merely to leave a workspace. */
export function ForumEnvironment({ appearance, active = true, attempt = 0, request = appearance, onApplied = noop, onFailure = noop }: {
  appearance: Appearance; active?: boolean; attempt?: number; request?: Appearance;
  onApplied?: (appearance: Appearance) => void; onFailure?: (appearance: Appearance) => void;
}) {
  const scene = useThree(state => state.scene);
  const renderer = useThree(state => state.gl);
  const cache = useRef(new Map<Appearance, Environment>());
  const ready = useRef<Appearance | null>(null);
  const reported = useRef<Appearance | null>(null);

  useEffect(() => {
    const previous = scene.environment;
    const previousIntensity = scene.environmentIntensity;
    const owned = cache.current;
    return () => {
      scene.environment = previous;
      scene.environmentIntensity = previousIntensity;
      for (const { target } of owned.values()) target.dispose();
      owned.clear();
    };
  }, [scene, renderer]);

  useEffect(() => {
    ready.current = null;
    reported.current = null;
    if (!active) return;
    const apply = (environment: Environment) => {
      scene.environment = environment.target.texture;
      scene.environmentIntensity = environment.intensity;
      ready.current = appearance;
    };
    const hit = cache.current.get(appearance);
    if (hit) { apply(hit); return; }
    let cancelled = false;
    let expired = false;
    const timer = window.setTimeout(() => { expired = true; onFailure(appearance); }, 5000);
    const current = () => !cancelled && !expired;
    const prepare = (baked: THREE.CubeTexture | null) => {
      if (!current()) { baked?.dispose(); return; }
      const hidden: THREE.Object3D[] = [];
      let pmrem: THREE.PMREMGenerator | null = null;
      try {
        pmrem = new THREE.PMREMGenerator(renderer);
        if (!baked) scene.traverse(object => {
          if (object.renderOrder > -100 && (object as THREE.Mesh).isMesh && object.visible) {
            object.visible = false; hidden.push(object);
          }
        });
        const target = baked ? pmrem.fromCubemap(baked) : pmrem.fromScene(scene, 0.04, 0.1, 1200);
        const environment = { target, intensity: baked ? environmentIntensity(appearance) : appearance === 'night' ? 0.35 : 0.6 };
        cache.current.set(appearance, environment);
        apply(environment);
        window.clearTimeout(timer);
      } catch { onFailure(appearance); window.clearTimeout(timer); }
      finally { for (const object of hidden) object.visible = true; baked?.dispose(); pmrem?.dispose(); }
    };
    void loadEnvironmentManifest(import.meta.env.BASE_URL, appearance).then(manifest => {
      if (!current()) return;
      if (!manifest) { prepare(null); return; }
      new THREE.CubeTextureLoader().load(faceUrls(import.meta.env.BASE_URL, appearance), texture => {
        texture.colorSpace = THREE.SRGBColorSpace;
        prepare(texture);
      }, undefined, () => prepare(null));
    }).catch(() => { if (current()) prepare(null); });
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [scene, renderer, appearance, active, attempt, request, onFailure]);

  useFrame(() => {
    if (!active || ready.current !== appearance || reported.current === appearance) return;
    reported.current = appearance;
    onApplied(appearance);
  });
  return null;
}
