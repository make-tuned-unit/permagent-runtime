/**
 * The Unreal-baked sky, if this build shipped one.
 *
 * Unreal is not the renderer — the world lives inside the app and users will
 * not have an engine installed — so it runs offline here and ships six cube
 * faces per appearance (`scripts/unreal/bake_environment.py`). Those faces are
 * the light arriving at the island, and handing them to three as an environment
 * map is what lets water, glass and bronze reflect a real sky.
 *
 * Everything here is optional by design. A build with no bake, a bake that is
 * still running, a face that failed to copy: each falls back to the environment
 * the scene derives from its own sky dome, which is dimmer but never wrong. The
 * world must never fail to draw because an art asset is missing.
 */

export type Appearance = 'day' | 'night';

/** The six faces, in the order `THREE.CubeTextureLoader` expects them. */
export const CUBE_FACE_ORDER = ['px', 'nx', 'py', 'ny', 'pz', 'nz'] as const;

export type EnvironmentManifest = {
  schema: string;
  appearance: Appearance;
  faceSize: number;
  faces: string[];
};

/** Where a bake lives, relative to the app's base URL. */
export function environmentPath(base: string, appearance: Appearance): string {
  return `${base}world/env/${appearance}/`;
}

/**
 * Read the manifest for an appearance, or `null` when this build has no bake.
 *
 * A missing manifest is the ordinary case for a source checkout, so it is not
 * an error and is not logged as one. A manifest that exists but does not list
 * all six faces IS a broken bake, and returns null rather than loading a cube
 * map with a hole in it — three would throw, and a half-lit world is harder to
 * diagnose than an obviously absent one.
 */
export async function loadEnvironmentManifest(
  base: string,
  appearance: Appearance,
  fetcher: typeof fetch = fetch,
): Promise<EnvironmentManifest | null> {
  try {
    const response = await fetcher(`${environmentPath(base, appearance)}manifest.json`, {
      cache: 'force-cache',
    });
    if (!response.ok) return null;
    const manifest = (await response.json()) as EnvironmentManifest;
    if (manifest?.schema !== 'permagent.forum.environment.v1') return null;
    if (manifest.appearance !== appearance) return null;
    const complete = CUBE_FACE_ORDER.every((face) => manifest.faces?.includes(face));
    return complete ? manifest : null;
  } catch {
    return null;
  }
}

/** The six face URLs, in loader order. */
export function faceUrls(base: string, appearance: Appearance): string[] {
  const path = environmentPath(base, appearance);
  return CUBE_FACE_ORDER.map((face) => `${path}${face}.png`);
}

/**
 * How strongly a baked sky lights the island.
 *
 * Unreal exposes its sky at a photographic exposure (85,000 lux of sun at
 * EV 13 for day); the capture is already tone-mapped to low dynamic range, so
 * this is a taste value rather than a physical one. Night is lower because a
 * moonlit environment map at day strength turns midnight into dusk.
 */
export function environmentIntensity(appearance: Appearance): number {
  return appearance === 'night' ? 0.45 : 0.85;
}
