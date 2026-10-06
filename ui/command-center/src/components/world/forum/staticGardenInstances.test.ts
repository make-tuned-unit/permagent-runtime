import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { Box3, BoxGeometry, Group, InstancedMesh, Matrix3, Matrix4, Mesh, MeshStandardMaterial, Sphere, Vector3 } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { instanceGardenDecor } from './staticGardenInstances';
import { GARDEN_DECOR, type GardenDecorPlacement } from './gardenLayout';

const closeVector=(a:Vector3,b:Vector3)=>expect(a.distanceTo(b)).toBeLessThan(2e-5);
function count(geometry: Mesh['geometry']) { return Math.min(geometry.drawRange.count,(geometry.index?.count??geometry.getAttribute('position').count)-geometry.drawRange.start)/3; }
describe('static garden instance parity',()=>{
  it('matches independently placed clones through nested nonuniform transforms, material ranges and MASK materials',()=>{
    const library=new Group();library.position.set(91,15,-32); // Library placement is not authored asset-local space.
    const asset=new Group();asset.name='fixture';asset.position.set(.3,.2,-.1);asset.scale.set(1.2,.8,1.4);asset.rotation.set(.2,-.3,.1);library.add(asset);
    const nested=new Group();nested.position.set(1,.4,-.5);nested.scale.set(.7,1.3,.9);nested.rotation.set(.4,.2,-.3);asset.add(nested);
    const materials=[new MeshStandardMaterial({color:'#334422',alphaTest:.55}),new MeshStandardMaterial({color:'#aa7744'})];
    const source=new Mesh(new BoxGeometry(1,2,.7),materials);source.name='split';source.rotation.set(.3,.4,.2);nested.add(source);
    source.geometry.groups.forEach((group,i)=>{group.materialIndex=i%2;});
    const hidden=new Mesh(new BoxGeometry(),materials[0]);hidden.visible=false;nested.add(hidden);
    const placements:GardenDecorPlacement[]=[{asset:'fixture',position:[-3,0,7],scale:.72,heading:.63},{asset:'fixture',position:[6,2,-4],scale:1.1,heading:-.9}];
    // Independent oracle is the exact previous cloneAsset + place implementation.
    const refs=placements.map(placement=>{const root=asset.clone(true);root.position.set(0,0,0);root.position.set(...placement.position);root.scale.setScalar(placement.scale);root.rotation.y=placement.heading;root.updateMatrixWorld(true);return root;});
    let sourceDisposals=0,materialDisposals=0;source.geometry.addEventListener('dispose',()=>sourceDisposals++);materials.forEach(m=>m.addEventListener('dispose',()=>materialDisposals++));
    const originalRootTransform={position:asset.position.toArray(),scale:asset.scale.toArray(),rotation:asset.rotation.toArray()};
    const originalPositions=Array.from(source.geometry.getAttribute('position').array);
    const result=instanceGardenDecor(library,placements);
    try{
      expect(result.census.placements).toBe(2);expect(result.census.primitiveBatches).toBe(6);expect(result.census.triangles).toBe(24);
      result.group.children.forEach(object=>{
        expect(object).toBeInstanceOf(InstancedMesh);const batch=object as InstancedMesh;expect(batch.count).toBe(2);expect(batch.castShadow&&batch.receiveShadow&&batch.frustumCulled).toBe(true);
        expect(batch.material).toBe(materials[batch.userData.materialSlot]);expect((batch.material as MeshStandardMaterial).alphaTest).toBe(batch.userData.materialSlot===0?.55:0);
        placements.forEach((_,instance)=>{
          const reference=refs[instance].getObjectByName('split') as Mesh;
          const matrix=new Matrix4();batch.getMatrixAt(instance,matrix);
          const start=batch.geometry.drawRange.start,end=start+batch.geometry.drawRange.count;
          for(let item=start;item<end;item++){
            const index=batch.geometry.index?.getX(item)??item;
            const actual=new Vector3().fromBufferAttribute(batch.geometry.getAttribute('position'),index).applyMatrix4(matrix);
            const expected=new Vector3().fromBufferAttribute(reference.geometry.getAttribute('position'),index).applyMatrix4(reference.matrixWorld);
            closeVector(actual,expected);
            expect(batch.boundingBox!.clone().expandByScalar(2e-5).containsPoint(expected)).toBe(true);
            expect(batch.boundingSphere!.distanceToPoint(expected)).toBeLessThan(2e-5);
            const normal=new Vector3().fromBufferAttribute(batch.geometry.getAttribute('normal'),index).applyNormalMatrix(new Matrix3().getNormalMatrix(matrix));
            const expectedNormal=new Vector3().fromBufferAttribute(reference.geometry.getAttribute('normal'),index).applyNormalMatrix(new Matrix3().getNormalMatrix(reference.matrixWorld));
            closeVector(normal,expectedNormal);
          }
        });
      });
      expect(Array.from(source.geometry.getAttribute('position').array)).toEqual(originalPositions);
      expect({position:asset.position.toArray(),scale:asset.scale.toArray(),rotation:asset.rotation.toArray()}).toEqual(originalRootTransform);
    }finally{result.dispose();}
    expect(sourceDisposals).toBe(0);expect(materialDisposals).toBe(0);source.geometry.dispose();hidden.geometry.dispose();materials.forEach(m=>m.dispose());
  });
  it('batches the actual committed GLB placements without losing a visible primitive or triangle',async()=>{
    const bytes=readFileSync('public/world/garden-hero/hero-kit.glb');
    const gltf=await new GLTFLoader().parseAsync(bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength),'');
    const result=instanceGardenDecor(gltf.scene,GARDEN_DECOR);
    const counts=new Map<string,number>();GARDEN_DECOR.forEach(p=>counts.set(p.asset,(counts.get(p.asset)??0)+1));
    let referenceTriangles=0,referenceDraws=0;
    for(const placement of GARDEN_DECOR){const root=gltf.scene.getObjectByName(placement.asset)!;root.traverseVisible(o=>{if(o instanceof Mesh){referenceTriangles+=count(o.geometry);referenceDraws+=Array.isArray(o.material)?o.geometry.groups.length:1;}});}
    try{
      expect(result.census).toEqual({placements:55,primitiveBatches:42,instances:269,triangles:354658});
      expect(referenceTriangles).toBe(result.census.triangles);expect(referenceDraws).toBe(result.census.instances);
      result.group.children.forEach(object=>{const batch=object as InstancedMesh;expect(batch.count).toBe(counts.get(batch.userData.asset));expect(batch.boundingBox).toBeInstanceOf(Box3);expect(batch.boundingSphere).toBeInstanceOf(Sphere);expect(batch.boundingSphere!.radius).toBeGreaterThan(0);});
    }finally{result.dispose();const materials=new Set<MeshStandardMaterial>();const geometries=new Set<Mesh['geometry']>();gltf.scene.traverse(o=>{if(o instanceof Mesh){geometries.add(o.geometry);for(const material of Array.isArray(o.material)?o.material:[o.material])materials.add(material as MeshStandardMaterial);}});geometries.forEach(g=>g.dispose());materials.forEach(m=>m.dispose());}
  });
});
