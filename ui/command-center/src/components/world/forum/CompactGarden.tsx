import { Suspense, useEffect, useMemo, useRef } from 'react';
import { OrbitControls, RoundedBox, useTexture } from '@react-three/drei';
import { useFrame, useThree } from '@react-three/fiber';
import { CylinderGeometry, Group, InstancedMesh, Matrix4, Mesh, MeshStandardMaterial, PerspectiveCamera, RepeatWrapping, SRGBColorSpace, Vector2 } from 'three';
import { GARDEN_ARRIVAL, GARDEN_SOIL_PATCHES, GARDEN_SOIL_Y, gardenCollisionSource } from './gardenLayout';
import { registerForumCollision } from './walkCollision';
import { getReduceMotion } from '../../../styles/tokens';
import { advanceForumSurfaceClock, releaseForumSurfaceClock, applyForumSurfaceMaterials } from './ForumSurfaceMaterials';
import type { ForumDistrict } from './districts';
import { getForumMotion } from './ForumInhabitants';
import { VaultBoundary } from '../areas/hall/BlenderVault';
import { ForumSky } from './ForumSky';

const STONE = '#c3baa4', BRONZE = '#8b6843', WOOD = '#73543b';
function Slab({ at, size, color = STONE }: { at: [number,number,number]; size: [number,number,number]; color?: string }) {
  return <RoundedBox position={at} args={size} radius={.045} smoothness={2} castShadow receiveShadow><meshStandardMaterial color={color} roughness={.76} metalness={color === BRONZE ? .65 : 0} name={color === WOOD ? 'Wood' : 'Stone'} /></RoundedBox>;
}
function GardenPaving() {
  const maps = useTexture(['diff','nor_dx','rough'].map(kind=>`${import.meta.env.BASE_URL}world/garden-materials/white_sandstone_blocks_02_${kind}_2k.jpg`));
  useMemo(()=>{maps.forEach(map=>{map.wrapS=map.wrapT=RepeatWrapping;map.repeat.set(2,2);});maps[0].colorSpace=SRGBColorSpace;},[maps]);
  const tiles = useMemo(() => Array.from({ length: 24 }, (_, i) => [i % 4 * 4.45 - 6.675, Math.floor(i / 4) * 3.96 - 9.9] as const), []);
  return <group>{tiles.map(([x,z], i) => <mesh key={i} position={[x,-.02,z]} receiveShadow><boxGeometry args={[4.4,.04,3.91]} /><meshStandardMaterial color={i%3===0?'#c1bba9':'#ddd7c5'} map={maps[0]} normalMap={maps[1]} normalScale={new Vector2(.45,-.45)} roughnessMap={maps[2]} roughness={.9} name="Stone" /></mesh>)}</group>;
}
function GardenSoil() {
  const patches=useMemo(()=>{
    const geometry=new CylinderGeometry(1,1,GARDEN_SOIL_Y,24);
    const material=new MeshStandardMaterial({color:'#514333',roughness:1,name:'Soil'});
    const mesh=new InstancedMesh(geometry,material,GARDEN_SOIL_PATCHES.length);
    mesh.name='ground planting soil';mesh.receiveShadow=true;
    GARDEN_SOIL_PATCHES.forEach((patch,i)=>{const matrix=new Matrix4().makeScale(patch.radius,1,patch.radius);matrix.setPosition(...patch.position);mesh.setMatrixAt(i,matrix);});
    mesh.instanceMatrix.needsUpdate=true;mesh.computeBoundingBox();mesh.computeBoundingSphere();return mesh;
  },[]);
  useEffect(()=>()=>{patches.dispose();patches.geometry.dispose();(patches.material as MeshStandardMaterial).dispose();},[patches]);
  return <primitive object={patches} dispose={null} />;
}
export function CompactGarden({ onEvidence, kitReady = false }: { onEvidence: () => void; kitReady?: boolean }) {
  const root = useRef<Group>(null);
  useFrame((_,delta)=>{if(!getReduceMotion())advanceForumSurfaceClock(Math.min(delta,.05));});
  useEffect(()=>releaseForumSurfaceClock,[]);
  useEffect(() => {
    const source = gardenCollisionSource();
    const cleanup = registerForumCollision(source);
    source.traverse(o => { if (o instanceof Mesh) o.geometry.dispose(); });
    const sourceMaterials=new Set<MeshStandardMaterial>();source.traverse(o=>{if(o instanceof Mesh){for(const material of Array.isArray(o.material)?o.material:[o.material])sourceMaterials.add(material as MeshStandardMaterial);}});sourceMaterials.forEach(m=>m.dispose());
    return cleanup;
  }, []);
  useEffect(() => root.current ? applyForumSurfaceMaterials(root.current) : undefined, []);
  return <group ref={root}>
    <Slab at={[0,-.22,0]} size={[18,.44,24]} />
    <VaultBoundary fallback={null}><Suspense fallback={null}><GardenPaving /></Suspense></VaultBoundary>
    <GardenSoil />
    {/* Real edge walls, not a destination beyond an invisible boundary. */}
    {([-8.8,8.8] as const).map(x => <Slab key={x} at={[x,.45,0]} size={[.4,.9,24]} />)}
    {([-11.8,11.8] as const).map(z => <Slab key={z} at={[0,.45,z]} size={[18,.9,.4]} />)}
    {/* Table placeholder is removed as soon as the authored kit is staged. */}
    {!kitReady && <group name="garden-table-fallback" position={[-3,0,3]}><mesh position={[0,.95,0]} castShadow receiveShadow><cylinderGeometry args={[1.35,1.35,.12,64]} /><meshStandardMaterial color={WOOD} roughness={.45} name="Wood" /></mesh><Slab at={[0,.45,0]} size={[.5,.9,.5]} color={BRONZE} /></group>}
    {/* An open pergola frames the arrival without creating unreachable floors. */}
    {([-7,-1] as const).flatMap(x => [1,7].map(z => <Slab key={`${x}-${z}`} at={[x,1.7,z]} size={[.18,3.4,.18]} color={BRONZE} />))}
    {Array.from({length:11},(_,i)=><Slab key={i} at={[-4,3.45,1+i*.6]} size={[6.5,.1,.12]} color={WOOD} />)}
    {[1,7].map(z=><Slab key={z} at={[-4,3.35,z]} size={[6.4,.16,.16]} color={BRONZE} />)}
    <Slab at={[4,-.01,-2]} size={[4,.18,6]} color="#435e5b" />
    <mesh position={[4,.09,-2]} rotation-x={-Math.PI/2}><planeGeometry args={[3.8,5.8,24,24]} /><meshPhysicalMaterial name="Water" color="#416d69" roughness={.16} metalness={0} transparent opacity={.85} /></mesh>
    {[2,6].map(x=><Slab key={x} at={[x,.18,-2]} size={[.18,.36,6.3]} />)}
    {[-5,1].map(z=><Slab key={z} at={[4,.18,z]} size={[4.3,.36,.18]} />)}
    <group position={[-7,0,-7]} onClick={e=>{e.stopPropagation();onEvidence();}}>
      <Slab at={[0,.5,0]} size={[1.1,1,1.1]} color={WOOD} />
      <Slab at={[0,1.08,0]} size={[1.2,.12,1.2]} color={BRONZE} />

    </group>
    {[-7,7].flatMap(x=>[-9,8].map(z=><group key={`${x}:${z}`}><Slab at={[x,.32,z]} size={[.4,.64,.4]} color={BRONZE} /><mesh position={[x,.74,z]}><sphereGeometry args={[.11,12,8]} /><meshStandardMaterial color="#ffe0a7" emissive="#ffce84" emissiveIntensity={1.1} /></mesh></group>))}
  </group>;
}
export function GardenLighting({ appearance }: { appearance: 'day'|'night' }) {
  const day = appearance === 'day';
  return <>
    <ForumSky appearance={appearance} />
    <color attach="background" args={['#172b3a']} />
    <hemisphereLight args={[day ? '#cfdeea' : '#b2c5d8', '#87725a', day ? 1.4 : .95]} />
    <directionalLight position={[-8,12,6]} color={day ? '#ffdfaf' : '#e5d6bb'} intensity={day ? 3 : 1.3} castShadow shadow-mapSize={[2048,2048]} shadow-camera-left={-14} shadow-camera-right={14} shadow-camera-top={16} shadow-camera-bottom={-16} shadow-normalBias={.03} />
    <directionalLight position={[8,5,-8]} color="#b4d3e1" intensity={.5} />
    <pointLight position={[-3,2.7,3]} color="#ffce87" intensity={day ? 5 : 24} distance={10} decay={2} />
    <pointLight position={[-6,2,-6]} color="#ffd4a1" intensity={day ? 2 : 12} distance={7} decay={2} />
  </>;
}
export function GardenNavigation({ district, focusAgent, overview }: { district: ForumDistrict; focusAgent: { id: string; revision: number } | null; overview: boolean }) {
  const camera = useThree(s=>s.camera);
  const controls = useRef<React.ComponentRef<typeof OrbitControls>>(null);
  useEffect(()=>{
    const p = focusAgent && getForumMotion(focusAgent.id);
    const at = p ? [p.x,p.y,p.z] : district.position;
    const arrival = district.id === 'commons' && !p;
    camera.position.set(...(overview ? [16,18,21] : arrival ? [...GARDEN_ARRIVAL.position] : [at[0]+2.8,1.7,at[2]+3.8]) as [number,number,number]);
    controls.current?.target.set(...(overview ? [0,0,0] : arrival ? [...GARDEN_ARRIVAL.target] : [at[0],1.2,at[2]]) as [number,number,number]);
    if(camera instanceof PerspectiveCamera){camera.fov=overview?48:58;camera.updateProjectionMatrix();}
    controls.current?.update();
  },[camera,district,focusAgent,overview]);
  return <OrbitControls ref={controls} makeDefault minDistance={.8} maxDistance={40} maxPolarAngle={Math.PI*.52} />;
}
