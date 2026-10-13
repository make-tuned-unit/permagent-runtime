import { scheduleForumStartup } from './forumStartup';
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
 * WASM blob. Browser loads use its worker API; CPU asset tests use the same
 * decoder without requiring Worker support.
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
  loader.setMeshoptDecoder(typeof Worker === 'function' ? browserDecoder : MeshoptDecoder);
};

// One same-origin module worker shared by the GLTF resource cache. Unlike
// meshopt's blob workers this respects the desktop's script-src self policy.
let decoderWorker: Worker | undefined;
let nextId = 0;
const pending = new Map<number, { resolve: (value: Uint8Array) => void; reject: (error: Error) => void }>();
function getDecoderWorker() {
  if (!decoderWorker) {
    decoderWorker = new Worker(new URL('./forumMeshopt.worker.ts', import.meta.url), { type: 'module' });
    decoderWorker.onmessage = ({ data }) => {
      const request = pending.get(data.id); pending.delete(data.id);
      if (data.error) request?.reject(new Error(data.error)); else request?.resolve(data.result);
    };
    decoderWorker.onerror = () => {
      for (const request of pending.values()) request.reject(new Error('World geometry decoder could not start'));
      pending.clear(); decoderWorker?.terminate(); decoderWorker = undefined;
    };
  }
  return decoderWorker;
}
const browserDecoder: typeof MeshoptDecoder = {
  ...MeshoptDecoder,
  decodeGltfBufferAsync(...args) {
    return new Promise((resolve, reject) => {
      scheduleForumStartup('GLB worker decode submission', () => {
        try {
          const worker = getDecoderWorker(), id = ++nextId;
          const source = args[2].slice();
          pending.set(id, { resolve, reject });
          try { worker.postMessage({ id, args: [args[0], args[1], source, args[3], args[4]] }, [source.buffer]); }
          catch (error) { pending.delete(id); throw error; }
        } catch (error) { reject(error); }
        return true;
      });
    });
  },
};
