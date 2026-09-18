import * as THREE from 'three';

/**
 * The baked water data: depth, distance to shore, bed height, and a mask.
 *
 * Produced by `scripts/blender/bake_water_info.py` from the island's own
 * geometry, so nothing here invents a water level — the same `Water` material
 * the walker refuses to walk on and the ships sail across is what was traced.
 *
 * Depth is what makes water read as water. A flat teal plane is a plane; water
 * that shows sand through its shallows, darkens as the bed drops away, and
 * gathers foam where it meets land is a lagoon. All three come from this one
 * texture, which is why it is worth a bake.
 */

export type WaterInfoManifest = {
  schema: string;
  resolution: number;
  metresPerTexel: number;
  bounds: { minX: number; maxX: number; minZ: number; maxZ: number };
  depthRange: number;
  shoreRange: number;
  waterCoverage: number;
};

export type WaterInfo = { texture: THREE.Texture; manifest: WaterInfoManifest };

/**
 * Load the water data, or `null` when this build has none.
 *
 * A missing bake is the ordinary case in a fresh checkout and is not an error:
 * the water then renders exactly as it did before, flat but correct. A manifest
 * whose schema this code does not know is refused rather than guessed at, since
 * misreading the channel meanings would tint the whole lagoon wrongly.
 */
export async function loadWaterInfo(
  base: string,
  fetcher: typeof fetch = fetch,
  loader: THREE.TextureLoader = new THREE.TextureLoader(),
): Promise<WaterInfo | null> {
  let manifest: WaterInfoManifest;
  try {
    const response = await fetcher(`${base}world/water-info.json`, { cache: 'force-cache' });
    if (!response.ok) return null;
    manifest = (await response.json()) as WaterInfoManifest;
  } catch {
    return null;
  }
  if (manifest?.schema !== 'permagent.forum.water.v1') return null;
  if (!manifest.bounds || !(manifest.depthRange > 0)) return null;

  const texture = await new Promise<THREE.Texture | null>((resolve) => {
    loader.load(`${base}world/water-info.png`, resolve, undefined, () => resolve(null));
  });
  if (!texture) return null;
  // Data, not colour: these channels are numbers and must not be sRGB-decoded.
  texture.colorSpace = THREE.NoColorSpace;
  texture.flipY = false;
  texture.wrapS = THREE.ClampToEdgeWrapping;
  texture.wrapT = THREE.ClampToEdgeWrapping;
  texture.minFilter = THREE.LinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.generateMipmaps = false;
  return { texture, manifest };
}

/**
 * The world-space rectangle the texture covers, as the shader wants it:
 * (minX, minZ, 1/width, 1/height), so a lookup is two multiplies.
 */
export function boundsVector(manifest: WaterInfoManifest): THREE.Vector4 {
  const { minX, maxX, minZ, maxZ } = manifest.bounds;
  return new THREE.Vector4(minX, minZ, 1 / (maxX - minX), 1 / (maxZ - minZ));
}
