import { BoxGeometry, BufferGeometry, CatmullRomCurve3, CylinderGeometry, Euler, Group, InstancedMesh, Matrix4, Mesh, MeshStandardMaterial, Quaternion, SphereGeometry, TorusGeometry, TubeGeometry, Vector3, type Material } from 'three';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import type { Point3 } from './habitatLayout';
/** Merge authored static details by material, keeping the scene inexpensive to draw. */
export class DetailBuilder {
  private parts=new Map<Material,BufferGeometry[]>();
  add(g:BufferGeometry,at:Point3,scale:Point3,m:Material,rotate:Point3=[0,0,0]) {
    g.applyMatrix4(new Matrix4().compose(new Vector3(...at),new Quaternion().setFromEuler(new Euler(...rotate)),new Vector3(...scale)));
    const list=this.parts.get(m)??[];list.push(g);this.parts.set(m,list);
  }
  box(at:Point3,size:Point3,m:Material,rotate:Point3=[0,0,0]){this.add(new BoxGeometry(1,1,1),at,size,m,rotate);}
  sphere(at:Point3,size:Point3,m:Material){this.add(new SphereGeometry(1,16,10),at,size,m);}
  cylinder(at:Point3,r:number,h:number,m:Material,top=r,rotate:Point3=[0,0,0]){this.add(new CylinderGeometry(top,r,h,24),at,[1,1,1],m,rotate);}
  ring(at:Point3,r:number,t:number,m:Material,rotate:Point3=[Math.PI/2,0,0]){this.add(new TorusGeometry(r,t,6,40),at,[1,1,1],m,rotate);}
  tube(points:Point3[],r:number,m:Material){this.add(new TubeGeometry(new CatmullRomCurve3(points.map(p=>new Vector3(...p))),Math.max(12,points.length*3),r,6,false),[0,0,0],[1,1,1],m);}
  dispose(){for(const parts of this.parts.values())parts.forEach(g=>g.dispose());this.parts.clear();}
  finish(name:string){
    const steps=this.finishSteps(name);let next=steps.next();
    while(!next.done)next=steps.next();return next.value;
  }
  *finishSteps(name:string):Generator<void,Group>{
    const group=new Group();group.name=name;let completed=false;
    try {
      for(const [material,parts] of this.parts){
        // Bound merge/upload buffers as well as construction. A material with
        // thousands of stems used to make a single enormous blocking merge.
        let batch:BufferGeometry[]=[],vertices=0;
        const merge=()=>{
          if(!batch.length)return;
          const geometry=mergeGeometries(batch,false);batch.forEach(g=>g.dispose());batch=[];vertices=0;
          if(geometry){const mesh=new Mesh(geometry,material);mesh.castShadow=true;mesh.receiveShadow=true;group.add(mesh);}
        };
        for(const p of parts){
          for(const key of Object.keys(p.attributes))if(!['position','normal','uv'].includes(key))p.deleteAttribute(key);
          batch.push(p);vertices+=p.getAttribute('position').count;
          if(vertices>=32768){merge();yield;}
        }
        merge();yield;
      }
      this.parts.clear();completed=true;return group;
    } finally {if(!completed){this.dispose();finishDetails(group);}}
  }
}
export const finishDetails=(group:Group)=>{group.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();if(o instanceof InstancedMesh)o.dispose();}});};
export const material=(color:string,roughness=.75,metalness=0)=>new MeshStandardMaterial({color,roughness,metalness});
