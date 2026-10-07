import {
  CylinderGeometry,
  ExtrudeGeometry,
  Shape,
  Group,
  LatheGeometry,
  Mesh,
  MeshStandardMaterial,
  Vector2,
} from 'three';
import { DetailBuilder } from './habitatGeometry';

/** Barrel length along local +Y (metres). */
export const TELESCOPE_BARREL_LENGTH = 2.4;
/** Nominal assembly height for layout checks (metres). */
export const TELESCOPE_ASSEMBLY_HEIGHT = 2.6;
/** Elevation above horizontal (radians). */
export const TELESCOPE_ELEVATION = Math.PI / 6;
/** Barrel-group rotation around Z that yields TELESCOPE_ELEVATION when the axis is local Y. */
export const TELESCOPE_BARREL_ROTATION_Z = -Math.PI / 3;

const DEW_CAP_LENGTH = 0.14;
const PIVOT_LOCAL_Y = TELESCOPE_BARREL_LENGTH / 2;
const PEDESTAL_TOP_Y = 1.02;
const FORK_PIVOT_Y = 1.78;

function lathe(profile: Vector2[], segments: number): LatheGeometry {
  return new LatheGeometry(profile, segments);
}

function shadowed(mesh: Mesh): Mesh {
  mesh.castShadow = true;
  mesh.receiveShadow = false;
  return mesh;
}

/** Procedural survey telescope from the r27 reference (ivory barrel, brass fork, stone pedestal). */
export function buildObservatoryTelescope(): Group {
  const ivory = new MeshStandardMaterial({ color: '#f1e6cf', roughness: 0.36, metalness: 0.04 });
  const brass = new MeshStandardMaterial({ color: '#b49660', roughness: 0.33, metalness: 0.68 });
  const stone = new MeshStandardMaterial({ color: '#c4b59a', roughness: 0.88, metalness: 0.02 });
  const lens = new MeshStandardMaterial({ color: '#174c50', roughness: 0.12, metalness: 0.5 });

  stone.name='telescope sandstone';
  const group = new Group();
  group.name = 'observatory telescope';

  // Closed stepped stone pedestal — top supports the fork base.
  const pedestal = shadowed(new Mesh(lathe([
    new Vector2(0, 0),
    new Vector2(.86, 0), new Vector2(.86, .10),
    new Vector2(.78, .16), new Vector2(.78, .23),
    new Vector2(.57, .23), new Vector2(.57, .32),
    new Vector2(.48, .36), new Vector2(.48, .81),
    new Vector2(.57, .85), new Vector2(.57, .91),
    new Vector2(.66, .91), new Vector2(.66, 1.02),
    new Vector2(0, 1.02),
  ], 8), stone));
  pedestal.position.y = 0.02;
  group.add(pedestal);

  // Brass azimuth ring on pedestal crown.
  const azimuthRing = shadowed(new Mesh(
    lathe([
      new Vector2(0.34, 0),
      new Vector2(0.42, 0),
      new Vector2(0.44, 0.04),
      new Vector2(0.36, 0.04),
    ], 24),
    brass,
  ));
  azimuthRing.position.y = PEDESTAL_TOP_Y;
  group.add(azimuthRing);

  // U-shaped brass fork — arms rise from the ring, pivots at barrel centre height.
  const forkOutline=new Shape();
  forkOutline.moveTo(-.49,0);
  forkOutline.lineTo(-.49,.76);
  forkOutline.quadraticCurveTo(-.49,.88,-.36,.88);
  forkOutline.quadraticCurveTo(-.26,.88,-.26,.76);
  forkOutline.lineTo(-.26,.34);
  forkOutline.quadraticCurveTo(-.26,.19,0,.19);
  forkOutline.quadraticCurveTo(.26,.19,.26,.34);
  forkOutline.lineTo(.26,.76);
  forkOutline.quadraticCurveTo(.26,.88,.36,.88);
  forkOutline.quadraticCurveTo(.49,.88,.49,.76);
  forkOutline.lineTo(.49,0); forkOutline.closePath();
  const forkGeometry=new ExtrudeGeometry(forkOutline,{depth:.14,bevelEnabled:true,bevelSegments:2,steps:1,bevelSize:.025,bevelThickness:.025,curveSegments:8});
  forkGeometry.translate(0,0,-.07);forkGeometry.rotateY(Math.PI/2);
  const fork=shadowed(new Mesh(forkGeometry,brass));fork.position.y=PEDESTAL_TOP_Y;
  group.add(fork);

  // Side trunnions at barrel pivot (centre of gravity).
  for (const z of [-0.3, 0.3]) {
    const trunnion = shadowed(new Mesh(new CylinderGeometry(0.1, 0.1, 0.3, 24), brass));
    trunnion.name = `trunnion ${z}`;
    trunnion.rotation.x = Math.PI / 2;
    trunnion.position.set(0, FORK_PIVOT_Y, z);
    group.add(trunnion);
  }

  // All barrel parts share one local Y axis; the group rotates once for elevation.
  const barrelGroup = new Group();
  barrelGroup.name = 'barrel assembly';
  barrelGroup.position.set(
    Math.sin(TELESCOPE_BARREL_ROTATION_Z) * PIVOT_LOCAL_Y,
    FORK_PIVOT_Y - Math.cos(TELESCOPE_BARREL_ROTATION_Z) * PIVOT_LOCAL_Y,
    0,
  );
  barrelGroup.rotation.z = TELESCOPE_BARREL_ROTATION_Z;

  const barrelProfile = [
    new Vector2(0, 0),
    new Vector2(0.14, 0),
    new Vector2(0.18, 0.12),
    new Vector2(0.23, 0.38),
    new Vector2(0.235, 0.72),
    new Vector2(0.235, 1.05),
    new Vector2(0.235, 1.28),
    new Vector2(0.27, TELESCOPE_BARREL_LENGTH - DEW_CAP_LENGTH),
  ];

  barrelGroup.add(shadowed(new Mesh(lathe(barrelProfile, 32), ivory)));

  const dewCapY = TELESCOPE_BARREL_LENGTH - DEW_CAP_LENGTH;
  const dewCap = shadowed(new Mesh(
    lathe([
      new Vector2(0.27, 0),
      new Vector2(0.30, 0.06),
      new Vector2(0.29, DEW_CAP_LENGTH),
    ], 24),
    ivory,
  ));
  dewCap.position.y = dewCapY;
  barrelGroup.add(dewCap);

  const objectiveBand = shadowed(new Mesh(
    lathe([
      new Vector2(0.289, 0),
      new Vector2(0.308, 0.025),
      new Vector2(0.289, 0.05),
    ], 20),
    brass,
  ));
  objectiveBand.position.y = TELESCOPE_BARREL_LENGTH - 0.06;
  barrelGroup.add(objectiveBand);

  const lensDisc = shadowed(new Mesh(
    lathe([
      new Vector2(0, 0),
      new Vector2(0.272, 0),
      new Vector2(0.272, 0.012),
      new Vector2(0, 0.012),
    ], 24),
    lens,
  ));
  lensDisc.position.y = TELESCOPE_BARREL_LENGTH;
  barrelGroup.add(lensDisc);

  const centreBand = shadowed(new Mesh(
    lathe([
      new Vector2(0.237, 0),
      new Vector2(0.263, 0.03),
      new Vector2(0.237, 0.06),
    ], 20),
    brass,
  ));
  centreBand.name = 'pivot collar';
  centreBand.position.y = PIVOT_LOCAL_Y - 0.03;
  barrelGroup.add(centreBand);

  const eyepiece = shadowed(new Mesh(
    lathe([
      new Vector2(0.05, 0),
      new Vector2(0.07, 0.08),
      new Vector2(0.06, 0.14),
    ], 16),
    brass,
  ));
  eyepiece.position.set(0, -0.12, 0);
  barrelGroup.add(eyepiece);

  const rearBand = shadowed(new Mesh(
    lathe([
      new Vector2(0.13, 0),
      new Vector2(0.15, 0.025),
      new Vector2(0.13, 0.05),
    ], 16),
    brass,
  ));
  rearBand.position.y = 0.04;
  barrelGroup.add(rearBand);

  // Separate narrow collars, fasteners and a finder scope provide depth at arm's length.
  const details=new DetailBuilder();
  for(const [y,r] of [[.24,.215],[.85,.239],[1.5,.248],[2.20,.275]]){
    details.ring([0,y,0],r,.014,brass);
    for(let i=0;i<8;i++){const a=i*Math.PI/4;details.sphere([Math.cos(a)*(r+.007),y,Math.sin(a)*(r+.007)],[.015,.015,.015],brass);}
  }
  for(const y of [.45,.91])details.box([0,y,.31],[.075,.08,.19],brass);
  details.cylinder([0,.69,.39],.057,.83,ivory,.072);
  details.ring([0,1.09,.39],.072,.013,brass);
  details.cylinder([0,1.107,.39],.059,.01,lens);
  const fittings=details.finish('Optical fittings');barrelGroup.add(fittings);
  const mountDetails=new DetailBuilder();
  for(const z of [-.465,.465]){
    mountDetails.cylinder([0,FORK_PIVOT_Y,z],.139,.038,brass,.139,[Math.PI/2,0,0]);
    for(let i=0;i<32;i++){const a=i*Math.PI/16;mountDetails.box([Math.cos(a)*.114,FORK_PIVOT_Y+Math.sin(a)*.114,z+Math.sign(z)*.023],[.018,.008,.01],stone,[0,0,a]);}
  }
  for(let i=0;i<8;i++){
    const a=i*Math.PI/4+Math.PI/8,x=Math.cos(a)*.45,z=Math.sin(a)*.45;
    mountDetails.box([x,.59,z],[.015,.36,.15],brass,[0,-a,0]);
  }
  group.add(mountDetails.finish('Engraved mount fittings'));
  group.add(barrelGroup);

  group.scale.setScalar(0.92);
  return group;
}
