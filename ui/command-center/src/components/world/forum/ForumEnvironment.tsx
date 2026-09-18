import { useEffect, useState } from 'react';
import { useThree } from '@react-three/fiber';
import * as THREE from 'three';
import {
  environmentIntensity,
  faceUrls,
  loadEnvironmentManifest,
  type Appearance,
} from './bakedEnvironment';

/**
 * Image-based lighting, derived from the forum's own sky.
 *
 * Until now every surface was lit by a hemisphere light and two directionals,
 * which gives shape but no ENVIRONMENT: water, glass and bronze had nothing to
 * reflect, so they read as coloured plastic. This renders the existing sky dome
 * into a cube map once, runs it through three's PMREM filter and hands the
 * result to `scene.environment`, which every `MeshStandardMaterial` in the GLB
 * picks up for free.
 *
 * Deriving it from the sky rather than shipping an HDRI is the point: the
 * reflections are always the sky actually overhead, so day and night agree with
 * each other automatically, nothing is downloaded, and no third-party asset or
 * licence enters the bundle.
 *
 * Cost is one cube render, not one per frame. The sky is static apart from
 * drifting clouds and the odd meteor, neither of which changes the light, so
 * re-filtering on every frame would buy nothing at real expense.
 */
export function ForumEnvironment({ appearance }: { appearance: Appearance }) {
  const scene = useThree((state) => state.scene);
  const renderer = useThree((state) => state.gl);
  const [baked, setBaked] = useState<THREE.CubeTexture | null>(null);

  // Prefer the sky Unreal baked for this appearance. It is an ordinary set of
  // six images, so a user without an engine gets the same lighting as this
  // machine; when a build ships without one, `manifest` is null and the
  // derived-from-our-own-sky path below runs exactly as before.
  useEffect(() => {
    let cancelled = false;
    let loaded: THREE.CubeTexture | null = null;
    const base = import.meta.env.BASE_URL;
    void loadEnvironmentManifest(base, appearance).then((manifest) => {
      if (cancelled || !manifest) return;
      new THREE.CubeTextureLoader().load(
        faceUrls(base, appearance),
        (texture) => {
          if (cancelled) {
            texture.dispose();
            return;
          }
          texture.colorSpace = THREE.SRGBColorSpace;
          loaded = texture;
          setBaked(texture);
        },
        undefined,
        // A face that fails to decode leaves the scene on its derived sky
        // rather than on a half-built cube map.
        () => undefined,
      );
    });
    return () => {
      cancelled = true;
      loaded?.dispose();
      setBaked(null);
    };
  }, [appearance]);

  useEffect(() => {
    if (baked) {
      // The baked sky is already the light arriving at the island; PMREM
      // filters it so rough surfaces read as rough rather than mirror-bright.
      const pmremBaked = new THREE.PMREMGenerator(renderer);
      const filtered = pmremBaked.fromCubemap(baked);
      const previousBaked = scene.environment;
      scene.environment = filtered.texture;
      scene.environmentIntensity = environmentIntensity(appearance);
      return () => {
        scene.environment = previousBaked;
        filtered.dispose();
        pmremBaked.dispose();
      };
    }
    // The sky is drawn by meshes tagged with a large negative render order
    // (`SKY_RENDER_ORDER`), and those are exactly what should light the island.
    // Everything else is hidden for the one cube render, so the island does not
    // light itself — a feedback loop that reads as a washed-out haze.
    const hidden: THREE.Object3D[] = [];
    scene.traverse((object) => {
      const isSky = object.renderOrder <= -100;
      const mesh = object as THREE.Mesh;
      if (!isSky && mesh.isMesh && object.visible) {
        object.visible = false;
        hidden.push(object);
      }
    });

    const pmrem = new THREE.PMREMGenerator(renderer);
    let target: THREE.WebGLRenderTarget | null = null;
    try {
      pmrem.compileCubemapShader();
      target = pmrem.fromScene(scene, 0.04, 0.1, 1200);
      const previous = scene.environment;
      scene.environment = target.texture;
      // A light touch: the sky is bright, and at full strength it flattens the
      // key light's modelling. This lifts shadowed faces without erasing them.
      scene.environmentIntensity = appearance === 'night' ? 0.35 : 0.6;
      for (const object of hidden) object.visible = true;
      return () => {
        scene.environment = previous;
        target?.dispose();
        pmrem.dispose();
      };
    } catch {
      // A context loss or an unsupported float target must not take the world
      // with it: the scene simply keeps the lighting it had before.
      for (const object of hidden) object.visible = true;
      pmrem.dispose();
      return undefined;
    }
  }, [scene, renderer, appearance, baked]);

  return null;
}
