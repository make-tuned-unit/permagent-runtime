import { useLoader, useThree } from '@react-three/fiber';
import { ImageBitmapLoader, Loader, LoadingManager, Texture, TextureLoader, LinearFilter, NoColorSpace, RepeatWrapping, SRGBColorSpace, type WebGLRenderer } from 'three';
import { measureForumStartup, scheduleForumStartup } from './forumStartup';
import { forumAssetManager, guardForumAssetRequest, noteForumAssetRequest } from './forumAssetRetry';

/** Explicit asynchronous image decode. Keep the loader identity shared by sky
 * preparation and rendering so choosing an appearance never fetches it twice. */
export class ForumTextureLoader extends Loader<Texture> {
  constructor(manager: LoadingManager = forumAssetManager) { super(manager); }
  override load(url: string, rawLoad?: (texture: Texture) => void, onProgress?: (event: ProgressEvent) => void, rawError?: (error: unknown) => void): Texture {
    // A request that never settles fails at the World asset deadline instead of suspending forever.
    const { onLoad, onError } = guardForumAssetRequest<Texture>(url, rawLoad, rawError);
    if (typeof createImageBitmap !== 'function') {
      return new TextureLoader(this.manager).setPath(this.path).load(url, texture => {
        const image = texture.image as HTMLImageElement;
        image.decoding = 'async';
        void (image.decode?.() ?? Promise.resolve()).then(() => onLoad(texture), onError);
      }, onProgress, onError);
    }
    const texture = new Texture();
    new ImageBitmapLoader(this.manager).setPath(this.path).setOptions({ imageOrientation: 'flipY', premultiplyAlpha: 'none' }).load(url, bitmap => {
      measureForumStartup('texture decoded / assign', () => {
        texture.image = bitmap; texture.flipY = false; texture.needsUpdate = true;
        texture.addEventListener('dispose', () => bitmap.close());
        onLoad(texture);
      });
    }, onProgress, onError);
    return texture;
  }
}
type Upload = { ready: boolean; error?: unknown; promise: Promise<void> };
const uploads = new WeakMap<WebGLRenderer, WeakMap<Texture, Upload>>();
function prepare(renderer: WebGLRenderer, texture: Texture): Upload {
  let cache = uploads.get(renderer);
  if (!cache) { cache = new WeakMap(); uploads.set(renderer, cache); }
  let entry = cache.get(texture);
  if (!entry) {
    entry = { ready: false, promise: Promise.resolve() };
    const current = entry;
    current.promise = new Promise(resolve => {
      scheduleForumStartup('texture GPU upload', () => {
        try { renderer.initTexture(texture); current.ready = true; }
        catch (error) { current.error = error; }
        resolve(); return true;
      }, renderer);
    });
    cache.set(texture, current);
  }
  return entry;
}
export type ForumTexturePreset = 'color' | 'panorama' | 'surface-color' | 'surface-data' | 'leaf';
const configured = new WeakMap<Texture, ForumTexturePreset>();
function configure(texture: Texture, preset: ForumTexturePreset) {
  if (configured.get(texture) === preset) return;
  // These URLs have one authored sampler configuration. Prepare that exact
  // configuration before upload; cloning or needsUpdate after upload would
  // force another synchronous transfer during the first scene render.
  texture.colorSpace = preset === 'surface-data' ? NoColorSpace : SRGBColorSpace;
  if (preset.startsWith('surface')) { texture.wrapS = texture.wrapT = RepeatWrapping; texture.anisotropy = 4; }
  if (preset === 'leaf') texture.anisotropy = 4;
  if (preset === 'panorama') { texture.generateMipmaps = false; texture.minFilter = texture.magFilter = LinearFilter; }
  texture.needsUpdate = true; configured.set(texture, preset);
}
export function useForumTexture<T extends string | string[]>(input: T, preset: ForumTexturePreset | ((index: number) => ForumTexturePreset) = 'color'): T extends string[] ? Texture[] : Texture {
  const renderer = useThree(state => state.gl);
  const urls = Array.isArray(input) ? input as string[] : [input as string];
  noteForumAssetRequest(`texture:${urls.join('\n')}`, urls, () => useLoader.clear(ForumTextureLoader, input));
  const textures = useLoader(ForumTextureLoader, input);
  const entries = (Array.isArray(textures) ? textures : [textures]).map((texture, index) => { configure(texture, typeof preset === 'function' ? preset(index) : preset); return prepare(renderer, texture); });
  const error = entries.find(entry => entry.error)?.error;
  if (error) throw error;
  const pending = entries.find(entry => !entry.ready);
  if (pending) throw pending.promise;
  return textures as T extends string[] ? Texture[] : Texture;
}
useForumTexture.clear = (url: string) => useLoader.clear(ForumTextureLoader, url);
