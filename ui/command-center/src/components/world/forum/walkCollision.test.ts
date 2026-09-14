import { describe, expect, it } from 'vitest';
import { BoxGeometry, Mesh, MeshStandardMaterial, Vector3 } from 'three';
import { attemptWalk } from './walkCollision';
function box(x:number,y:number,z:number,w:number,h:number,d:number,name='Stone') {
  const material=new MeshStandardMaterial();material.name=name;
  const mesh=new Mesh(new BoxGeometry(w,h,d),material);mesh.position.set(x,y,z);mesh.updateMatrixWorld(true);return mesh;
}
describe('grounded forum exploration',()=>{
  it('walks on a surface and blocks leaving its edge',()=>{
    const floor=box(0,-.1,0,4,.2,4),feet=new Vector3(0,0,0);
    expect(attemptWalk(feet,.2,0,[floor])).toBe(true);expect(feet.y).toBeCloseTo(0);
    feet.x=1.95;expect(attemptWalk(feet,.2,0,[floor])).toBe(false);
  });
  it('stops against walls and does not tunnel through them',()=>{
    const floor=box(0,-.1,0,10,.2,10),wall=box(0,1,-.5,3,2,.1),feet=new Vector3(0,0,0);
    expect(attemptWalk(feet,0,-.25,[floor,wall])).toBe(false);
    expect(feet.z).toBe(0);
  });
  it('climbs stair-height changes but rejects a high ledge',()=>{
    const floor=box(0,-.1,0,10,.2,10),step=box(0,.0675,-.5,2,.135,1);
    const feet=new Vector3(0,0,.1);
    expect(attemptWalk(feet,0,-.2,[floor,step])).toBe(true);expect(feet.y).toBeCloseTo(.135);
    const high=box(0,.6,-1.5,2,1.2,1);feet.set(0,0,-.9);
    expect(attemptWalk(feet,0,-.2,[floor,high])).toBe(false);
  });
  it('does not treat the water material as a walkable floor',()=>{
    const water=box(0,-.05,0,4,.1,4,'Water');
    expect(attemptWalk(new Vector3(),.2,0,[water])).toBe(false);
  });
});
