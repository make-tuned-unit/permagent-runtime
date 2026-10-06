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
  finish(name:string){
    const group=new Group();group.name=name;
    for(const [material,parts] of this.parts){
      // Normalize attributes before merging primitive families.
      for(const p of parts){if(!p.getAttribute('uv'))continue;for(const key of Object.keys(p.attributes))if(!['position','normal','uv'].includes(key))p.deleteAttribute(key);}
      const geometry=mergeGeometries(parts,false);parts.forEach(g=>g.dispose());if(!geometry)continue;
      const mesh=new Mesh(geometry,material);mesh.castShadow=true;mesh.receiveShadow=true;group.add(mesh);
    }
    this.parts.clear();return group;
  }
}
export const finishDetails=(group:Group)=>{group.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();if(o instanceof InstancedMesh)o.dispose();}});};
export const material=(color:string,roughness=.75,metalness=0)=>new MeshStandardMaterial({color,roughness,metalness});
