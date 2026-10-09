/** One offscreen WebGL renderer for still Settings portraits. The World owns
 * character design in heroRobot/createAgentWardrobe; this module only captures
 * it. Calls serialize so neither rows nor StrictMode can create parallel GL
 * contexts. Images are cached by World id for the page lifetime. */
import { ACESFilmicToneMapping, AmbientLight, DirectionalLight, PerspectiveCamera, Scene, SRGBColorSpace, WebGLRenderer } from 'three';
import type { Group } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { withMeshopt } from '../../world/forum/forumGltf';
import { buildHeroRobot, disposeHeroRobot, HERO_KIT_URL, heroProportions } from '../../world/forum/heroRobot';

const THUMBNAIL_PIXELS = 192;
const IDLE_RELEASE_MS = 3000;
const snapshots = new Map<string, Promise<string | null>>();
let loadedScene: Promise<Group> | undefined;
let renderer: WebGLRenderer | undefined;
let queue: Promise<unknown> = Promise.resolve();
let releaseTimer: ReturnType<typeof setTimeout> | undefined;

function heroScene(): Promise<Group> {
  loadedScene ??= withMeshopt(new GLTFLoader()).loadAsync(HERO_KIT_URL)
    .then(gltf => gltf.scene)
    .catch(error => {
      // A transient load failure must not poison every later portrait request.
      // Only a later call retries; this path never starts a retry loop.
      loadedScene = undefined;
      throw error;
    });
  return loadedScene;
}

function releaseRendererSoon() {
  if (releaseTimer) clearTimeout(releaseTimer);
  releaseTimer = setTimeout(() => {
    renderer?.dispose();
    renderer?.forceContextLoss();
    renderer = undefined;
    releaseTimer = undefined;
  }, IDLE_RELEASE_MS);
}

async function capture(id: string): Promise<string | null> {
  const modelScene = await heroScene();
  const robot = buildHeroRobot(modelScene, id);
  if (releaseTimer) clearTimeout(releaseTimer);
  releaseTimer = undefined;
  try {
    renderer ??= new WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true });
    // React Three Fiber's non-flat Canvas uses ACES + sRGB. Match the live
    // World portrait's color pipeline before serializing the still.
    renderer.toneMapping = ACESFilmicToneMapping;
    renderer.outputColorSpace = SRGBColorSpace;
    renderer.setPixelRatio(1);
    renderer.setSize(THUMBNAIL_PIXELS, THUMBNAIL_PIXELS, false);
    renderer.setClearColor(0x000000, 0);

    const scene = new Scene();
    scene.add(new AmbientLight(0xffffff, 1.35));
    const warm = new DirectionalLight(0xffe2b8, 2.4);
    warm.position.set(-2.2, 3.2, 3.4);
    scene.add(warm);
    const cool = new DirectionalLight(0xb9d9ed, 1.25);
    cool.position.set(2.4, 1.8, 2);
    scene.add(cool);
    const camera = new PerspectiveCamera(31, 1, 0.1, 20);
    // A square bust crop suited to 28px rows. Position/lighting track the
    // World's live desk portrait; no animation or status color enters the still.
    camera.position.set(0, 0.58, 2.4);
    camera.lookAt(0, 0.58, 0);
    robot.object.scale.set(...heroProportions(id));
    robot.object.position.set(0, -0.92, 0);
    scene.add(robot.object);
    renderer.render(scene, camera);
    if (renderer.getContext().isContextLost()) return null;
    return renderer.domElement.toDataURL('image/png');
  } finally {
    disposeHeroRobot(robot);
    releaseRendererSoon();
  }
}

export function getAgentThumbnail(worldId: string): Promise<string | null> {
  const hit = snapshots.get(worldId);
  if (hit) return hit;
  const result = queue.then(() => capture(worldId)).catch(() => null);
  queue = result.then(() => undefined);
  snapshots.set(worldId, result);
  void result.then(url => {
    // Keep successful stills (and in-flight requests) shared across rows, but
    // let a later explicit request retry a failed load or WebGL capture.
    if (url === null && snapshots.get(worldId) === result) snapshots.delete(worldId);
  });
  return result;
}
