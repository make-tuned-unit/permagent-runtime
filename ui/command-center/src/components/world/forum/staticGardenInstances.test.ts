import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { Box3, BoxGeometry, Group, InstancedMesh, Matrix3, Matrix4, Mesh, MeshStandardMaterial, Sphere, Vector3, DoubleSide, SRGBColorSpace } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { instanceGardenDecor } from './staticGardenInstances';
import { GARDEN_DECOR, PLANT_BASE_Y, type GardenDecorPlacement } from './gardenLayout';

const closeVector=(a:Vector3,b:Vector3)=>expect(a.distanceTo(b)).toBeLessThan(2e-5);
function count(geometry: Mesh['geometry']) { return Math.min(geometry.drawRange.count,(geometry.index?.count??geometry.getAttribute('position').count)-geometry.drawRange.start)/3; }
// Clip a triangle to the closed half-space below head height. Every surviving
// polygon vertex (including edge/plane intersections) bounds its convex face.
function clipBelowHeight(triangle: Vector3[], height:number):Vector3[]{
  const polygon:Vector3[]=[];
  triangle.forEach((point,i)=>{
    const next=triangle[(i+1)%triangle.length];
    const inside=point.y<=height,nextInside=next.y<=height;
    if(inside)polygon.push(point.clone());
    if(inside!==nextInside)polygon.push(point.clone().lerp(next,(height-point.y)/(next.y-point.y)));
  });
  return polygon;
}
describe('static garden instance parity',()=>{
  it('includes intersections of triangle edges crossing the head plane',()=>{
    const clipped=clipBelowHeight([new Vector3(0,0,0),new Vector3(2,2,0),new Vector3(0,2,2)],1);
    expect(clipped.map(p=>p.toArray())).toEqual([[0,0,0],[1,1,0],[0,1,1]]);
    expect(clipBelowHeight([new Vector3(0,2,0),new Vector3(2,2,0),new Vector3(0,2,2)],1)).toEqual([]);
  });
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
    const bytes=readFileSync('public/world/garden-hero/hero-kit-v3b.glb');
    // CPU-only embedded PNG decode; no browser/WebGL/network rendering.
    const oldBitmap=globalThis.createImageBitmap;
    const oldSelf=globalThis.self;globalThis.self=globalThis as unknown as Window & typeof globalThis;
    const sharp=(await import('sharp')).default;
    globalThis.createImageBitmap=async(blob:Blob)=>{const decoded=await sharp(Buffer.from(await blob.arrayBuffer())).ensureAlpha().raw().toBuffer({resolveWithObject:true});return {width:decoded.info.width,height:decoded.info.height,data:decoded.data,close(){}} as unknown as ImageBitmap;};
    let gltf;
    try{gltf=await new GLTFLoader().parseAsync(bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength),'');}finally{globalThis.createImageBitmap=oldBitmap;globalThis.self=oldSelf;}
    const result=instanceGardenDecor(gltf.scene,GARDEN_DECOR);
    const counts=new Map<string,number>();GARDEN_DECOR.forEach(p=>counts.set(p.asset,(counts.get(p.asset)??0)+1));
    let referenceTriangles=0,referenceDraws=0;
    for(const placement of GARDEN_DECOR){const root=gltf.scene.getObjectByName(placement.asset)!;root.traverseVisible(o=>{if(o instanceof Mesh){referenceTriangles+=count(o.geometry);referenceDraws+=Array.isArray(o.material)?o.geometry.groups.length:1;}});}
    try{
      expect(result.census).toEqual({placements:55,primitiveBatches:34,instances:201,triangles:651030});
      // Woody geometry is conservative inside proxies through the 1.85 m head envelope.
      for(const placement of GARDEN_DECOR.filter(p=>p.supportY!==undefined)){
        const asset=gltf.scene.getObjectByName(placement.asset)!;
        let minY=Infinity, woodyCount=0;
        asset.traverseVisible(o=>{if(!(o instanceof Mesh))return;
          const material=o.material as MeshStandardMaterial;
          const wood=/Bark|branch/i.test(material.name);
          const vertices=o.geometry.getAttribute('position');
          for(let i=0;i<vertices.count;i++){
            const point=new Vector3().fromBufferAttribute(vertices,i).applyMatrix4(o.matrixWorld);
            minY=Math.min(minY,point.y);
          }
          if(wood){
            const geometry=o.geometry;
            const end=Math.min(geometry.drawRange.start+geometry.drawRange.count,geometry.index?.count??vertices.count);
            const cosine=Math.cos(placement.heading),sine=Math.sin(placement.heading);
            const placeVertex=(index:number)=>{
              const p=new Vector3().fromBufferAttribute(vertices,index).applyMatrix4(o.matrixWorld).multiplyScalar(placement.scale);
              return new Vector3(p.x*cosine+p.z*sine+placement.position[0],p.y+placement.position[1],-p.x*sine+p.z*cosine+placement.position[2]);
            };
            const inradius=placement.solid![0]/2*placement.scale*Math.cos(Math.PI/24);
            let maxRadius=0;
            for(let index=geometry.drawRange.start;index+2<end;index+=3){
              const triangle=[0,1,2].map(offset=>placeVertex(geometry.index?.getX(index+offset)??index+offset));
              for(const point of clipBelowHeight(triangle,1.85)){
                woodyCount++;maxRadius=Math.max(maxRadius,Math.hypot(point.x-placement.position[0],point.z-placement.position[2]));
              }
            }
            expect(maxRadius,`${placement.asset} clipped woody face envelope at ${placement.position}`).toBeLessThanOrEqual(inradius);
          }
          if(material.map){expect(material.map.image.width).toBe(1254);expect(material.map.colorSpace).toBe(SRGBColorSpace);expect(material.alphaTest).toBe(.55);expect(material.side).toBe(DoubleSide);expect(material.transparent).toBe(false);expect(o.geometry.getAttribute('uv')).toBeDefined();}
        });
        expect(minY).toBeCloseTo(PLANT_BASE_Y[placement.asset as keyof typeof PLANT_BASE_Y],6);
        expect(minY*placement.scale+placement.position[1]).toBeCloseTo(placement.supportY!,6);
        if(placement.asset==='canopy'||placement.asset==='shrub')expect(woodyCount).toBeGreaterThan(0);
      }
      expect(referenceTriangles).toBe(result.census.triangles);expect(referenceDraws).toBe(result.census.instances);
      result.group.children.forEach(object=>{const batch=object as InstancedMesh;expect(batch.count).toBe(counts.get(batch.userData.asset));expect(batch.boundingBox).toBeInstanceOf(Box3);expect(batch.boundingSphere).toBeInstanceOf(Sphere);expect(batch.boundingSphere!.radius).toBeGreaterThan(0);});
    }finally{result.dispose();const materials=new Set<MeshStandardMaterial>();const geometries=new Set<Mesh['geometry']>();gltf.scene.traverse(o=>{if(o instanceof Mesh){geometries.add(o.geometry);for(const material of Array.isArray(o.material)?o.material:[o.material])materials.add(material as MeshStandardMaterial);}});geometries.forEach(g=>g.dispose());materials.forEach(m=>m.dispose());}
  });
});
