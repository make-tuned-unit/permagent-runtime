import { useEffect, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { Euler, PerspectiveCamera, Vector3 } from 'three';
import { attemptWalk } from './walkCollision';
import { ROSTER, getAgentPosition } from '../agents';
import { crossedGatePlane, inExedra } from './meshPortal';
import { FORUM_WALK_FOV } from './vistaCamera';

export function ForumWalk({ onInspect, onExit, start, onEnterMesh, onExitMesh, onMeshPrompt, inMesh = false }: {
  start:[number,number,number];
  onInspect:(id:string)=>void;
  onExit:()=>void;
  /** Walked out through the MESH gate: the Agora takes over the scene. */
  onEnterMesh?:()=>void;
  /** Walked back in through the ring while the Agora is open. */
  onExitMesh?:()=>void;
  /** True while the walker is standing in the exedra beyond the ring. */
  onMeshPrompt?:(near:boolean)=>void;
  inMesh?:boolean;
}) {
  const { camera, gl } = useThree();
  const feet = useRef(new Vector3(...start));
  const previous = useRef(new Vector3(...start));
  const nearGate = useRef(false);
  // Set while this component releases pointer lock on purpose, so the
  // pointerlockchange handler does not read it as "the user pressed Esc" and
  // drop out of walk mode — the Sovereign stays on foot inside the Agora.
  const releasing = useRef(false);
  const keys = useRef(new Set<string>());
  const angles = useRef(new Euler(0,0,0,'YXZ'));
  const drag = useRef(false);
  // Placement is its own effect, keyed only on `start`: the listener effect
  // below re-runs whenever a callback or `inMesh` changes, and teleporting the
  // walker back to the district start every time the Agora opens or closes
  // would undo the walk they just took.
  useEffect(()=>{
    feet.current.set(...start);
    previous.current.set(...start);
    nearGate.current = false;
    // Walking has its own field. The overview camera now carries the vista's
    // 72 degrees (`vistaCamera.ts`), which is a landscape lens, not a pair of
    // eyes; restore the field walk mode has always been framed for.
    if (camera instanceof PerspectiveCamera && camera.fov !== FORUM_WALK_FOV) { camera.fov = FORUM_WALK_FOV; camera.updateProjectionMatrix(); }
    camera.position.set(start[0],start[1]+1.7,start[2]);camera.lookAt(start[0],start[1]+1.7,start[2]-10);
    angles.current.setFromQuaternion(camera.quaternion,'YXZ');
  },[camera,start]);
  useEffect(()=>{
    const enterMesh = () => { releasing.current = true; document.exitPointerLock?.(); onEnterMesh?.(); };
    const editable = (target: EventTarget|null) => target instanceof HTMLElement && (target.isContentEditable || ['INPUT','TEXTAREA','SELECT','BUTTON'].includes(target.tagName));
    // The forum is a panel in a shared window, and walk keys have to be
    // window-level (under pointer lock there is no per-element key target). So
    // ignore them once focus has moved to another surface — a sibling panel,
    // an overlay — instead of steering the camera from behind someone's back.
    // A null/body/documentElement activeElement is the normal walking state.
    const shell = gl.domElement.closest('.forum-shell');
    const focusedElsewhere = () => {
      const active = document.activeElement;
      if (!active || active === document.body || active === document.documentElement) return false;
      return !!shell && !shell.contains(active);
    };
    const keyDown = (e:KeyboardEvent) => {
      if (focusedElsewhere()) { keys.current.clear(); return; }
      if (editable(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (['KeyW','KeyA','KeyS','KeyD','ArrowLeft','ArrowRight','ShiftLeft','ShiftRight'].includes(e.code)) { e.preventDefault();keys.current.add(e.code); }
      if (e.code==='Escape') { e.preventDefault(); if (inMesh) onExitMesh?.(); else onExit(); }
      if (e.code==='KeyE' && !e.repeat) {
        // Standing in the exedra (round the side of the ring rather than
        // through it): E is the way in, same key as meeting an agent.
        if (nearGate.current && !inMesh && onEnterMesh) { enterMesh(); return; }
        let nearest: string|null=null, distance=3;
        for (const agent of ROSTER) {
          const p=getAgentPosition(agent.id); if(!p)continue;
          const d=Math.hypot(p.x-feet.current.x,p.y-feet.current.y,p.z-feet.current.z);
          if(d<distance){distance=d;nearest=agent.id;}
        }
        if(nearest) { document.exitPointerLock?.();onInspect(nearest);onExit(); }
      }
    };
    const keyUp=(e:KeyboardEvent)=>keys.current.delete(e.code);
    const clear=()=>{keys.current.clear();drag.current=false;};
    // Focus leaving the forum must also release whatever is already held down,
    // or the camera keeps drifting on a stuck key.
    const focusMoved=()=>{ if(focusedElsewhere())clear(); };
    const move=(e:MouseEvent)=>{
      if (document.pointerLockElement!==gl.domElement && !drag.current) return;
      angles.current.y-=e.movementX*.002;
      angles.current.x=Math.max(-1.35,Math.min(1.35,angles.current.x-e.movementY*.002));
      camera.quaternion.setFromEuler(angles.current);
    };
    const startDrag=()=>{drag.current=true;}; const end=()=>{drag.current=false;};
    let wasLocked=document.pointerLockElement===gl.domElement;
    const locked=()=>{
      const now=document.pointerLockElement===gl.domElement;
      if(wasLocked && !now){ if(releasing.current) releasing.current=false; else {clear();onExit();} }
      wasLocked=now;
    };
    window.addEventListener('keydown',keyDown);window.addEventListener('keyup',keyUp);
    window.addEventListener('blur',clear);document.addEventListener('visibilitychange',clear);
    document.addEventListener('focusin',focusMoved);
    document.addEventListener('mousemove',move);document.addEventListener('mouseup',end);
    document.addEventListener('pointerlockchange',locked);gl.domElement.addEventListener('mousedown',startDrag);
    return ()=>{
      window.removeEventListener('keydown',keyDown);window.removeEventListener('keyup',keyUp);
      window.removeEventListener('blur',clear);document.removeEventListener('visibilitychange',clear);
      document.removeEventListener('focusin',focusMoved);
      document.removeEventListener('mousemove',move);document.removeEventListener('mouseup',end);
      document.removeEventListener('pointerlockchange',locked);gl.domElement.removeEventListener('mousedown',startDrag);
      if(document.pointerLockElement===gl.domElement)document.exitPointerLock?.();
    };
  },[camera,gl,onExit,onInspect,onEnterMesh,onExitMesh,inMesh]);
  useFrame((_,rawDt)=>{
    const k=keys.current, dt=Math.min(rawDt,.05);
    if(k.has('ArrowLeft'))angles.current.y+=dt*1.6;
    if(k.has('ArrowRight'))angles.current.y-=dt*1.6;
    camera.quaternion.setFromEuler(angles.current);
    const forward=Number(k.has('KeyW'))-Number(k.has('KeyS'));
    const right=Number(k.has('KeyD'))-Number(k.has('KeyA'));
    const n=Math.hypot(forward,right);
    if(n){
      const speed=k.has('ShiftLeft')||k.has('ShiftRight')?5.5:3.2;
      const yaw=angles.current.y;
      const dx=(-Math.sin(yaw)*forward+Math.cos(yaw)*right)*speed*dt/n;
      const dz=(-Math.cos(yaw)*forward-Math.sin(yaw)*right)*speed*dt/n;
      // Axis fallback permits sliding along a wall without clipping through it.
      if(!attemptWalk(feet.current,dx,dz)) { attemptWalk(feet.current,dx,0);attemptWalk(feet.current,0,dz); }
      camera.position.set(feet.current.x,feet.current.y+1.7,feet.current.z);
    }
    // The MESH gate is checked every frame, from the step actually taken, so a
    // fast step cannot tunnel through the ring plane.
    const crossing = crossedGatePlane(previous.current, feet.current);
    previous.current.copy(feet.current);
    if (crossing==='outward' && !inMesh && onEnterMesh) { releasing.current=true; document.exitPointerLock?.(); onEnterMesh(); }
    else if (crossing==='inward' && inMesh && onExitMesh) { onExitMesh(); }
    const near = !inMesh && inExedra(feet.current);
    if (near !== nearGate.current) { nearGate.current = near; onMeshPrompt?.(near); }
  });
  return null;
}
