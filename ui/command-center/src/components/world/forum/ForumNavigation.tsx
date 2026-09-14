import { useEffect, useRef } from 'react';
import { useThree } from '@react-three/fiber';
import { OrbitControls, Html } from '@react-three/drei';
import { getForumMotion } from './ForumInhabitants';
import { DISTRICTS, ForumDistrict } from './districts';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';
import { MESH_GATE, gateApproachPoint } from './meshPortal';
export function ForumNavigation({ district, onDistrict, focusAgent }: { district: ForumDistrict; focusAgent: { id: string; revision: number } | null; onDistrict: (id: string) => void }) {
  const camera = useThree(s => s.camera);
  const { colors } = useTheme();
  const controls = useRef<React.ComponentRef<typeof OrbitControls>>(null);
  useEffect(() => {
    // The Mesh gate is a threshold, not a place to look down on: stand back on
    // the spur at the approach point and look outward through the ring, which
    // is also where a walker is put down when they return from the Agora.
    if (district.id === 'mesh') {
      const [ax,,az] = gateApproachPoint();
      camera.position.set(ax + 9.5, 9, az + 9.5);
      controls.current?.target.set(MESH_GATE.center[0], 2.4, MESH_GATE.center[2]);
      controls.current?.update();
      return;
    }
    const [x,y,z] = district.position;
    // Deliberate static camera transition: respects reduced motion and never
    // switches the user's input mode as a side effect of selecting a person.
    camera.position.set(x + (district.id==='commons' ? 29 : 13), y + (district.id==='commons' ? 29 : 12), z + (district.id==='commons' ? 36 : 16));
    controls.current?.target.set(x,y+1,z);
    controls.current?.update();
  }, [camera,district]);
  useEffect(() => {
    if (!focusAgent) return;
    const person = getForumMotion(focusAgent.id);
    if (!person) return;
    camera.position.set(person.x + 5, person.y + 3.5, person.z + 7);
    controls.current?.target.set(person.x, person.y + 1.2, person.z);
    controls.current?.update();
  }, [camera, focusAgent]);
  return <><OrbitControls ref={controls} makeDefault target={[0,1,0]} minDistance={5} maxDistance={420} maxPolarAngle={Math.PI/2.05} />
    {DISTRICTS.filter(d=>d.id!=='commons').map(d=><Html key={d.id} position={[d.position[0],d.position[1]+3.2,d.position[2]]} center distanceFactor={28}><Button colors={colors} variant="ghost" flashSuccess={false} className="forum-waypoint" style={{borderColor:d.accent}} onClick={()=>onDistrict(d.id)}>{d.name}</Button></Html>)}
  </>;
}
