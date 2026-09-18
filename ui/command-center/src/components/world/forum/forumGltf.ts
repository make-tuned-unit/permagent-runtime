import { MeshoptDecoder } from 'three/examples/jsm/libs/meshopt_decoder.module.js';

/**
 * The shipped forum GLB is written with EXT_meshopt_compression plus
 * KHR_mesh_quantization (ui/command-center/scripts/compress-world-glb.mjs),
 * which takes the Blender export from ~62 MB to ~14 MB. Nothing decodes it
 * without a meshopt decoder, so every loader that reads a forum asset has to
 * be handed one — the browser loaders here and the Node loaders in the tests
 * that read `public/world/solar-forum.glb` directly.
 *
 * Draco is deliberately not used: it needs a decoder fetched from a CDN in the
 * browser and Web Workers that Node does not have, so the same file could not
 * be verified by the walk-route tests. Meshopt's decoder is a small inline
 * WASM blob that runs synchronously in both environments.
 *
 * The legacy World's own loaders (areas/hall/BlenderVault) are untouched —
 * those assets are still uncompressed.
 */
type MeshoptCapableLoader = { setMeshoptDecoder(decoder: typeof MeshoptDecoder): unknown };

export function withMeshopt<T extends MeshoptCapableLoader>(loader: T): T {
  loader.setMeshoptDecoder(MeshoptDecoder);
  return loader;
}

/**
 * `extendLoader` argument for drei's `useGLTF`. Pass alongside
 * `useDraco = false` and `useMeshopt = false` so drei installs neither its
 * CDN-hosted Draco decoder nor its own three-stdlib meshopt copy, and this
 * decoder — the one three itself ships — is the only one in play.
 */
export const extendForumLoader = (loader: MeshoptCapableLoader): void => {
  loader.setMeshoptDecoder(MeshoptDecoder);
};
