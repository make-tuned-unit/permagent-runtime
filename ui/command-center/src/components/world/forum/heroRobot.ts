import { BufferGeometry, Group, Material, Mesh, MeshBasicMaterial, MeshPhysicalMaterial, MeshStandardMaterial, Object3D, SphereGeometry } from 'three';
import { stationFor } from './habitatLayout';
import { createAgentWardrobe } from './agentWardrobe';
import { heroNode, replaceFixedHeroMouth } from './heroRig';
import { DetailBuilder, finishDetails, material } from './habitatGeometry';

export const HERO_KIT_URL = `${import.meta.env.BASE_URL}world/garden-hero/hero-kit-v3b.glb?v=e114f28bedaa`;

function cloneAsset(scene: Object3D, name: string) {
  const original = scene.getObjectByName(name);
  if (!original) throw new Error(`Hero kit missing ${name}`);
  const copy = original.clone(true);
  copy.position.set(0, 0, 0);
  copy.traverse(object => {
    if (object instanceof Mesh) {
      object.castShadow = true;
      object.receiveShadow = true;
    }
  });
  return copy;
}

export type HeroRobotParts = {
  object: Object3D;
  materials: Material[];
  geometries: BufferGeometry[];
  wardrobe: ReturnType<typeof createAgentWardrobe>;
  head: Object3D | undefined;
  limbs: (Object3D | undefined)[];
  eyes: Mesh[];
  mouth: Mesh;
  seatedBody: Group;
  seatMaterials: MeshStandardMaterial[];
};

/**
 * The one hero body builder used by both the World agents and their desk
 * portraits. Identity wardrobe, trim and accessory variants must stay on this
 * seam so the two surfaces cannot drift into different characters again.
 */
export function buildHeroRobot(scene: Object3D, id: string): HeroRobotParts {
  const object = cloneAsset(scene, 'robot');
  const materials: Material[] = [];
  const geometries: BufferGeometry[] = [];
  const wardrobe = createAgentWardrobe(id);
  object.traverse(object3d => {
    if (!(object3d instanceof Mesh)) return;
    const sourceMaterials = Array.isArray(object3d.material) ? object3d.material : [object3d.material];
    object3d.castShadow = sourceMaterials.some(material => material.name.startsWith('Ceramic'));
    const copy = (source: MeshStandardMaterial) => {
      const cloned = new MeshPhysicalMaterial({
        name: source.name,
        color: source.color,
        map: source.map,
        normalMap: source.normalMap,
        roughnessMap: source.roughnessMap,
        metalnessMap: source.metalnessMap,
        roughness: source.roughness,
        metalness: source.metalness,
        transparent: source.transparent,
        opacity: source.opacity,
        alphaTest: source.alphaTest,
        side: source.side,
        depthWrite: source.depthWrite,
        depthTest: source.depthTest,
      });
      if (cloned.name.startsWith('Ceramic')) {
        cloned.color.set(id === 'strix' ? '#a6b0ac' : id === 'forecaster' ? '#d3dfe0' : id === 'picker' ? '#e2d4b5' : '#eee5d0');
        cloned.roughness = .28;
        cloned.clearcoat = .34;
        cloned.clearcoatRoughness = .22;
        cloned.ior = 1.46;
      } else if (cloned.name.startsWith('Faceplate')) {
        cloned.color.set('#07141a');
        cloned.roughness = .24;
        cloned.clearcoat = .18;
        cloned.clearcoatRoughness = .3;
      }
      cloned.needsUpdate = true;
      materials.push(cloned);
      return cloned;
    };
    object3d.material = Array.isArray(object3d.material)
      ? object3d.material.map(material => copy(material as MeshStandardMaterial))
      : copy(object3d.material as MeshStandardMaterial);
  });
  const head = heroNode(object, 'Head');
  object.add(wardrobe.body);
  head?.add(wardrobe.headwear);
  if (head) {
    const mouthGeometry = replaceFixedHeroMouth(head);
    if (mouthGeometry) geometries.push(mouthGeometry);
  }
  const mouth = new Mesh(new SphereGeometry(1, 24, 12), new MeshBasicMaterial({ color: '#b3e4d5' }));
  mouth.name = 'Voice-synchronized mouth';
  mouth.position.set(0, -.085, .307);
  mouth.scale.set(.055, .004, .013);
  head?.add(mouth);

  const seatBuilder = new DetailBuilder();
  const seatWood = material('#73543b');
  const seatCeramic = material('#e5decc', .4);
  const seatJoint = material('#354345', .4, .3);
  for (const x of [-.125, .125]) {
    seatBuilder.tube([[x, .68, 0], [x, .50, .31]], .075, seatCeramic);
    seatBuilder.sphere([x, .50, .31], [.08, .075, .08], seatJoint);
    seatBuilder.tube([[x, .50, .31], [x, .12, .37]], .068, seatCeramic);
    seatBuilder.box([x, .065, .43], [.18, .10, .28], seatCeramic);
  }
  for (const side of [-1, 1]) {
    seatBuilder.sphere([side * .29, 1.065, 0], [.10, .10, .10], seatJoint);
    seatBuilder.tube([[side * .29, 1.06, 0], [side * .37, .85, .20]], .072, seatCeramic);
    seatBuilder.sphere([side * .37, .85, .20], [.082, .078, .082], seatJoint);
    seatBuilder.tube([[side * .37, .85, .20], [side * .30, 1.055, .71]], .071, seatCeramic);
    seatBuilder.sphere([side * .30, 1.055, .71], [.072, .045, .055], seatJoint);
    seatBuilder.sphere([side * .30, 1.067, .78], [.103, .041, .10], seatCeramic);
    for (let finger = 0; finger < 4; finger++) seatBuilder.sphere([side * .30 + (finger - 1.5) * .045, 1.06, .867], [.019, .027, .064], seatCeramic);
    seatBuilder.sphere([side * .411, 1.058, .78], [.034, .03, .065], seatCeramic);
  }
  seatBuilder.cylinder([0, .61, -.08], .34, .09, seatWood);
  seatBuilder.box([0, .92, -.30], [.61, .53, .065], seatWood);
  for (const x of [-.24, .24]) for (const z of [-.24, .15]) seatBuilder.cylinder([x, .29, z], .028, .58, seatJoint);
  const seatedBody = seatBuilder.finish('Seated pose and chair');
  seatedBody.visible = false;
  object.add(seatedBody);
  const limbs = ['Arm.L', 'Arm.R', 'Leg.L', 'Leg.R'].map(name => heroNode(object, name));
  const eyes: Mesh[] = [];
  head?.traverse(object3d => {
    if (object3d instanceof Mesh && (object3d.material as MeshStandardMaterial).name?.startsWith('Eyes')) eyes.push(object3d);
  });
  return { object, materials, geometries, wardrobe, head, limbs, eyes, mouth, seatedBody, seatMaterials: [seatWood, seatCeramic, seatJoint] };
}

export function disposeHeroRobot(robot: HeroRobotParts) {
  robot.materials.forEach(material => material.dispose());
  robot.geometries.forEach(geometry => geometry.dispose());
  robot.wardrobe.materials.forEach(material => material.dispose());
  finishDetails(robot.wardrobe.body);
  finishDetails(robot.wardrobe.headwear);
  finishDetails(robot.seatedBody);
  robot.seatMaterials.forEach(material => material.dispose());
  robot.mouth.geometry.dispose();
  (robot.mouth.material as MeshBasicMaterial).dispose();
}

export function heroProportions(id: string): [number, number, number] {
  return id === 'strix' ? [1.1, 1.04, 1.05] : id === 'reader' ? [.94, 1.02, .96] : id === 'librarian' ? [1, 1.07, 1] : id === 'picker' ? [1, .95, 1] : [1, 1, 1];
}

export function setHeroEyeMaterial(robot: HeroRobotParts, id: string, live: boolean, state: string) {
  const eyeColor = id === 'henry' ? '#a0dbb9' : stationFor(id).color;
  robot.materials.forEach(material => {
    if (material instanceof MeshStandardMaterial && material.name.startsWith('Eyes')) {
      material.color.set(eyeColor);
      material.emissive.set(eyeColor);
      material.emissiveIntensity = live && state === 'working' ? .8 : .6;
    }
  });
}
