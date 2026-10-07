import { useEffect, useMemo } from 'react';
import { useTexture } from '@react-three/drei';
import { DoubleSide, Mesh, MeshStandardMaterial, LatheGeometry, RingGeometry, RepeatWrapping, SphereGeometry, SRGBColorSpace, Vector2 } from 'three';
import { OBSERVATORY_BENCHES, OBSERVATORY_BRIDGE, OBSERVATORY_PAD, OBSERVATORY_PLINTH, OBSERVATORY_WING_HEIGHT, onObservatoryBridgeOpening, observatoryGuardSegments } from './habitatObservatoryWing';
import { buildObservatoryTelescope } from './habitatObservatoryTelescope';
import { observatoryPadPerimeter, observatoryWingHullGeometry, terraceHullGeometry, terracePerimeter, terraceSurfaceGeometry } from './habitatTerrace';
import { CraftedLibrary } from './CraftedLibrary';
import { CraftedDesks } from './CraftedDesks';
import { DetailBuilder, finishDetails, material } from './habitatGeometry';
import { COMMONS_SURFACE_Y, HABITAT_BEDS, HABITAT_STATIONS, habitatElevation, habitatHeight, type Point3 } from './habitatLayout';
import { WATER_COURT } from './habitatWaterFlow';

export function HabitatArchitecture(){
  const maps=useTexture([
    ...['diff','nor_dx','rough'].map(k=>`white_sandstone_blocks_02_${k}_2k.jpg`),
    ...['diffuse','nor_gl','rough'].map(k=>`monastery_stone_floor_${k}_2k.webp`),
    ...['diffuse','nor_gl','rough'].map(k=>`wood_floor_${k}_2k.webp`),
  ].map(file=>`${import.meta.env.BASE_URL}world/garden-materials/${file}`));
  const built=useMemo(()=>{
    const stone=material('#d4c29a',.91),paving=material('#c2b394',.87),edge=material('#bba987'),wood=material('#68462e',.73),bronze=material('#9f7950',.35,.72),dark=material('#243b3c',.52),soil=material('#3c4130'),cloth=material('#dbd0aa',.93),glass=material('#477e79',.3,.25),glow=material('#ffe5ac'),domeGlass=material('#8ebabc',.26,.18);
    domeGlass.transparent=true;domeGlass.opacity=.16;domeGlass.depthWrite=false;domeGlass.side=DoubleSide;
    glow.emissive.set('#ffc56d');glow.emissiveIntensity=1.3;cloth.side=DoubleSide;
    const ownMaps=maps.map(t=>t.clone());
    ownMaps.forEach((t,i)=>{t.wrapS=t.wrapT=RepeatWrapping;t.anisotropy=4;t.repeat.set(1,1);if(i%3===0)t.colorSpace=SRGBColorSpace;t.needsUpdate=true;});
    for(const [surface,start,scale,strength,normalY] of [[stone,0,.26,.35,-1],[paving,3,1/1.8,.55,1],[wood,6,1/1.7,.32,1]] as const){
      surface.map=ownMaps[start];surface.normalMap=ownMaps[start+1];surface.roughnessMap=ownMaps[start+2];surface.normalScale=new Vector2(strength,strength*normalY);
      // Continuous world mapping preserves the photographed metre scale across
      // merged walls, bridge planks and the connected sloping paving surface.
      surface.onBeforeCompile=shader=>{
        shader.vertexShader=shader.vertexShader.replace('#include <uv_vertex>', `
          #include <uv_vertex>
          vec3 surfacePosition=(modelMatrix*vec4(position,1.)).xyz;
          vec3 surfaceNormal=abs(normalize(mat3(modelMatrix)*normal));
          vec2 surfaceUv=surfaceNormal.y>max(surfaceNormal.x,surfaceNormal.z)?surfacePosition.xz:
            surfaceNormal.x>surfaceNormal.z?vec2(surfacePosition.z,surfacePosition.y):surfacePosition.xy;
          vMapUv=surfaceUv*${scale.toFixed(6)};
          vNormalMapUv=vMapUv;
          vRoughnessMapUv=vMapUv;
        `);
      };
      surface.customProgramCacheKey=()=>`habitat-surface-v2-${start}-${scale}`;
    }
    const projectPaving=paving.onBeforeCompile;
    paving.onBeforeCompile=(shader,renderer)=>{
      projectPaving(shader,renderer);
      shader.fragmentShader=shader.fragmentShader.replace('#include <color_fragment>', `
        #include <color_fragment>
        float stoneLuminance=dot(diffuseColor.rgb,vec3(.2126,.7152,.0722));
        diffuseColor.rgb=mix(diffuseColor.rgb,stoneLuminance*vec3(1.08,1.035,.95),.66)*1.65;
      `);
    };
    paving.customProgramCacheKey=()=> 'habitat-pale-paving-v3';
    paving.color.set('#ffffff');wood.color.set('#c2a98d');
    const b=new DetailBuilder();
    b.add(terraceSurfaceGeometry(true),[0,-.001,0],[1,1,1],paving);
    b.add(terraceHullGeometry(),[0,0,0],[1,1,1],stone);
    b.add(observatoryWingHullGeometry(),[0,0,0],[1,1,1],stone);
    for(const bed of HABITAT_BEDS){
      const y=habitatHeight(bed.z);b.sphere([bed.x,y+.22,bed.z],[bed.rx,.24,bed.rz],soil);
      for(let i=0;i<44;i++){
        const a=i/44*Math.PI*2,next=(i+1)/44*Math.PI*2;
        const x1=bed.x+Math.cos(a)*bed.rx,z1=bed.z+Math.sin(a)*bed.rz,x2=bed.x+Math.cos(next)*bed.rx,z2=bed.z+Math.sin(next)*bed.rz;
        const z=(z1+z2)/2,base=habitatHeight(z)-.06,top=y+.44,height=Math.max(.16,top-base);
        b.box([(x1+x2)/2,top-height/2,z],[Math.hypot(x2-x1,z2-z1)*1.025,height,.22],stone,[0,Math.atan2(-(z2-z1),x2-x1),0]);
      }
    }
    // Continuous coping replaces the unconnected ellipsoid skirt. The narrow
    // bronze reveal separates the garden floor from its tapered stone support.
    const perimeter=terracePerimeter(.05);
    for(let i=0;i<perimeter.length;i++){
      const [x,z]=perimeter[i],[nx,nz]=perimeter[(i+1)%perimeter.length];
      const y=habitatHeight(z),ny=habitatHeight(nz),length=Math.hypot(nx-x,nz-z);
      b.tube([[x,y-.19,z],[nx,ny-.19,nz]],.022,bronze);
      if(i%3===0)b.box([x,y-.43,z],[.10,.44,.10],bronze,[0,Math.atan2(nx-x,nz-z),0]);
      if(length>.04)b.tube([[x,y-.045,z],[nx,ny-.045,nz]],.04,edge);
    }
    // Bronze edge rail, planted on the safe side of the connected floor.
    for(const x of [-15.7,15.7])for(let z=-24;z<13;z+=1.6){const y=habitatHeight(z);b.cylinder([x,y+.48,z],.035,.96,bronze);b.box([x,y+.96,z+.8],[.07,.07,1.6],bronze,[z<-8&&z>-14?Math.atan(3.4/6):0,0,0]);}
    for(const [x0,x1,z] of [[-12,OBSERVATORY_BRIDGE.x0,-27.6],[OBSERVATORY_BRIDGE.x1,12,-27.6],[-12,12,15.6]]){
      const y=habitatHeight(z),n=Math.ceil((x1-x0)/1.5);
      b.box([(x0+x1)/2,y+.96,z],[x1-x0,.07,.07],bronze);
      for(let i=0;i<=n;i++)b.cylinder([x0+(x1-x0)*i/n,y+.48,z],.035,.96,bronze);
    }
    // Arrival arbor: curved side ribs, fine timber slats and hanging lanterns.
    for(const z of [1,6.6]){
      b.tube([[-7.5,0,z],[-7.5,2.8,z],[-6.7,3.6,z],[-3.5,3.9,z],[-.5,3.45,z]],.10,bronze);

    }
    for(let x=-7;x<-.7;x+=.6)b.box([x,3.6+(1-Math.abs(x+4)/4)*.25,3.8],[.10,.10,5.7],wood);
    for(const x of [-6.8,-4.6,-2.4]){
      b.cylinder([x,3.1,1.3],.009,.92,bronze);
      for(const y of [2.38,2.70]){b.cylinder([x,y,1.3],.15,.038,bronze);b.ring([x,y,1.3],.137,.014,bronze);}
      for(let i=0;i<6;i++){const a=i*Math.PI/3;b.cylinder([x+Math.cos(a)*.128,2.54,1.3+Math.sin(a)*.128],.008,.30,bronze);}
      b.sphere([x,2.54,1.3],[.061,.116,.061],glow);b.cylinder([x,2.40,1.3],.082,.028,dark);
    }
    // One reachable garden arch frames the upper terraces.
    for(const z of [-11.5,-12.5]){
      const points=Array.from({length:25},(_,i)=>{const a=i/24*Math.PI;return [12.5*Math.cos(a),.8+11*Math.sin(a),z] as Point3;});
      b.tube(points,.23,stone);
      const inner=points.map(([x,y,z])=>[x*.98,y-.1,z+.18] as Point3);b.tube(inner,.025,bronze);
    }
    for(let i=0;i<13;i++){const a=i/12*Math.PI;b.box([12.5*Math.cos(a),.8+11*Math.sin(a),-12],[.13,.13,1.25],bronze,[0,0,a]);}
    // Segmented shell and inset solar panels give the great arch a legible
    // architectural depth, with all work areas on the connected terraces below.
    for(let i=0;i<48;i++){
      const a=(i+.5)/48*Math.PI,tangent=Math.atan2(11*Math.cos(a),-12.5*Math.sin(a));
      const x=12.5*Math.cos(a),y=.8+11*Math.sin(a),length=Math.hypot(12.5*Math.sin(a),11*Math.cos(a))*Math.PI/48;
      b.box([x,y,-12],[length*1.01,.17,1.65],stone,[0,0,tangent]);
      b.box([x+Math.cos(a)*.11,y+Math.sin(a)*.11,-12],[length*.83,.04,1.22],dark,[0,0,tangent]);
      b.box([x,y,-12],[.022,.20,1.72],bronze,[0,0,tangent]);
    }
    // Archive pavilion: open front, layered cornice, warm book-lined interior.
    b.box([-10,5,-23],[8,3.2,.35],stone);
    b.box([-10,7.1,-19.4],[8.6,.25,7.9],stone);b.box([-10,7.28,-19.4],[8.8,.12,8.1],edge);
    b.box([-10,6.92,-15.65],[8.1,.055,.05],glow);
    const archiveArch=Array.from({length:21},(_,i)=>[-14+i*.4,5.78+Math.sin(i/20*Math.PI)*1.08,-16] as Point3);
    b.tube(archiveArch,.14,stone);b.tube(archiveArch.map(([x,y,z])=>[x,y-.16,z+.045] as Point3),.024,bronze);
    for(let i=0;i<27;i++)b.box([-13.9+i*.3,7.02,-15.41],[.085,.20,.12],bronze);
    // Workshop: airy timber frame, solar canopy, shelves and suspended tools.
    for(const x of [7,14])for(const z of [-16,-22])b.cylinder([x,5.1,z],.10,3.4,bronze);
    b.box([10.5,6.85,-19],[7.7,.10,6.8],cloth,[.04,0,.07]);
    for(let i=0;i<6;i++){b.box([7.5+i*1.15,7.04,-20.1],[1.04,.06,3.4],dark,[.04,0,.07]);for(let j=0;j<5;j++)b.box([7.5+i*1.15,7.08,-21.4+j*.64],[.96,.015,.014],bronze);}
    b.box([11,4.7,-22.4],[6,2.4,.12],wood);
    for(const x of [7,14]){
      b.tube([[x,5.65,-16],[x,6.70,-17.1]],.045,bronze);
      b.tube([[x,5.65,-22],[x,6.70,-20.9]],.045,bronze);
    }
    for(let i=0;i<11;i++){const x=8.4+i*.48;b.ring([x,4.9,-22.28],.085,.022,bronze,[0,0,0]);b.box([x,4.65,-22.25],[.035,.4,.035],bronze);}
    // Open observatory: a ribbed glazed canopy above a reachable instrument
    // court. Its lower edge clears standing agents; four posts carry the roof.
    b.add(new SphereGeometry(3.8,48,24,0,Math.PI*2,0,Math.PI/2),[3,5.6,-24],[1,1,1],domeGlass);
    for(const x of [-.2,8.8])for(const z of [-21.7,-26.3]){
      b.cylinder([x,4.5,z],.07,2.2,bronze);b.cylinder([x,3.49,z],.17,.18,stone);b.ring([x,5.55,z],.16,.025,bronze);
    }
    for(let i=0;i<16;i++){
      const a=i*Math.PI/8,points=Array.from({length:19},(_,j)=>{const t=j/18*Math.PI/2;return [3+Math.sin(t)*Math.cos(a)*3.8,5.6+Math.cos(t)*3.8,-24+Math.sin(t)*Math.sin(a)*3.8] as Point3;});
      b.tube(points,.023,bronze);
    }
    for(const angle of [.35,.70,1.05,Math.PI/2])b.ring([3,5.6+Math.cos(angle)*3.8,-24],Math.sin(angle)*3.8,.026,bronze);
    b.ring([3,3.415,-24],2.7,.012,bronze);b.ring([3,3.415,-24],2.85,.01,bronze);
    // Connected observatory garden wing: open bridge rails, layered borders, instrument court.
    const wingPerimeter=observatoryPadPerimeter(.05);
    for(let i=0;i<wingPerimeter.length;i++){
      const [x,z]=wingPerimeter[i],[nx,nz]=wingPerimeter[(i+1)%wingPerimeter.length],length=Math.hypot(nx-x,nz-z);
      b.tube([[x,OBSERVATORY_WING_HEIGHT-.19,z],[nx,OBSERVATORY_WING_HEIGHT-.19,nz]],.022,bronze);
      if(length>.04)b.tube([[x,OBSERVATORY_WING_HEIGHT-.045,z],[nx,OBSERVATORY_WING_HEIGHT-.045,nz]],.04,edge);
    }
    // A supported five-metre crossing, with longitudinal guards shared with collision.
    const bridgeNorth=OBSERVATORY_PAD.cz+OBSERVATORY_PAD.hz;
    b.box([(OBSERVATORY_BRIDGE.x0+OBSERVATORY_BRIDGE.x1)/2,OBSERVATORY_WING_HEIGHT-.13,(-28+bridgeNorth)/2],[OBSERVATORY_BRIDGE.x1-OBSERVATORY_BRIDGE.x0,.24,-28-bridgeNorth],stone);
    for(const {a,c} of observatoryGuardSegments().map(s=>({a:s.a,c:s.b}))){
      const length=Math.hypot(c[0]-a[0],c[1]-a[1]),n=Math.max(1,Math.ceil(length/1.6));
      b.tube([[a[0],OBSERVATORY_WING_HEIGHT+.98,a[1]],[c[0],OBSERVATORY_WING_HEIGHT+.98,c[1]]],.035,bronze);
      b.tube([[a[0],OBSERVATORY_WING_HEIGHT+.36,a[1]],[c[0],OBSERVATORY_WING_HEIGHT+.36,c[1]]],.018,bronze);
      for(let i=0;i<n;i++)b.cylinder([a[0]+(c[0]-a[0])*i/n,OBSERVATORY_WING_HEIGHT+.49,a[1]+(c[1]-a[1])*i/n],.027,.98,bronze);
    }
    for(const [x,z,r] of [[OBSERVATORY_PAD.cx-OBSERVATORY_PAD.hx+1.2,OBSERVATORY_PAD.cz,.9],[OBSERVATORY_PAD.cx+OBSERVATORY_PAD.hx-1.2,OBSERVATORY_PAD.cz,.9],[OBSERVATORY_PAD.cx,OBSERVATORY_PAD.cz-OBSERVATORY_PAD.hz+1.1,.85]] as [number,number,number][]){
      for(let i=0;i<8;i++){const a=i/8*Math.PI*2,bx=x+Math.cos(a)*r,bz=z+Math.sin(a)*r*.55;if(!onObservatoryBridgeOpening(bx,bz,.2))b.sphere([bx,OBSERVATORY_WING_HEIGHT+.12,bz],[.22,.14,.22],soil);}
    }
    // Inlaid azimuth court: fine brass divisions and dark stone concentric bands.
    const courtY=OBSERVATORY_WING_HEIGHT+.013,cx=OBSERVATORY_PLINTH.x,cz=OBSERVATORY_PLINTH.z;
    for(const [r,width] of [[3.6,.085],[5.7,.19],[7.6,.075]]){
      b.add(new RingGeometry(r-width,r,128),[cx,courtY,cz],[1,1,1],dark,[-Math.PI/2,0,0]);
      b.ring([cx,courtY+.004,cz],r,.012,bronze);
    }
    for(let i=0;i<96;i++){
      const a=i*Math.PI/48,major=i%8===0,r=5.42;
      b.box([cx+Math.sin(a)*r,courtY+.004,cz+Math.cos(a)*r],[major?.035:.018,.009,major?.38:.14],bronze,[0,a,0]);
    }
    // Flush bridge wayfinding strips and small shielded lanterns.
    for(const x of [OBSERVATORY_BRIDGE.x0+.32,OBSERVATORY_BRIDGE.x1-.32]){
      b.box([x,courtY,-30.5],[.045,.014,5.2],bronze);
      for(const z of [-28.7,-32.2]){b.cylinder([x,OBSERVATORY_WING_HEIGHT+.18,z],.075,.36,bronze);b.cylinder([x,OBSERVATORY_WING_HEIGHT+.365,z],.054,.028,glow);}
    }
    for(const {x,z} of OBSERVATORY_BENCHES){
      for(const dx of [-1.32,1.32])b.box([x+dx,OBSERVATORY_WING_HEIGHT+.23,z],[.19,.46,.65],stone);
      for(let i=0;i<6;i++)b.box([x,OBSERVATORY_WING_HEIGHT+.49,z-.27+i*.105],[3.4,.075,.085],wood);
      for(let i=0;i<4;i++)b.box([x,OBSERVATORY_WING_HEIGHT+.67+i*.09,z-.34],[3.4,.065,.045],wood,[-.12,0,0]);
      for(const dx of [-1.4,1.4]){b.box([x+dx,OBSERVATORY_WING_HEIGHT+.66,z-.3],[.045,.62,.05],bronze);b.box([x+dx,OBSERVATORY_WING_HEIGHT+.69,z],[.08,.04,.6],bronze);}
      const lx=x+(x<cx?-2:2);
      b.cylinder([lx,OBSERVATORY_WING_HEIGHT+.45,z],.065,.9,bronze);
      b.cylinder([lx,OBSERVATORY_WING_HEIGHT+.91,z],.11,.08,glow);
      b.cylinder([lx,OBSERVATORY_WING_HEIGHT+.99,z],.16,.035,bronze);
    }
    b.cylinder([OBSERVATORY_PLINTH.x,OBSERVATORY_WING_HEIGHT+OBSERVATORY_PLINTH.height/2,OBSERVATORY_PLINTH.z],OBSERVATORY_PLINTH.radius,OBSERVATORY_PLINTH.height,stone);
    b.ring([OBSERVATORY_PLINTH.x,OBSERVATORY_WING_HEIGHT+OBSERVATORY_PLINTH.height,OBSERVATORY_PLINTH.z],OBSERVATORY_PLINTH.radius+.08,.025,bronze);
    b.cylinder([3,9.55,-24],.055,.55,bronze,.025);b.sphere([3,9.85,-24],[.055,.055,.055],glow);
    for(const x of [-14,-6,7,14]){
      const h=x===-14||x===14?3.6:2.6;
      b.cylinder([x,7.15+h/2,-22],.045,h,bronze,.025);
      for(const side of [-1,1]){b.box([x+side*.14,7.5+h*.28,-22],[.045,h*.7,.18],edge,[0,0,-side*.055]);b.box([x+side*.14,7.5+h*.28,-21.90],[.017,h*.64,.012],glow,[0,0,-side*.055]);}
      b.ring([x,7.2,-22],.19,.025,bronze);b.sphere([x,7.18+h,-22],[.035,.06,.035],glow);
    }
    // A traversable timber bridge, with low ramps at both landings.
    for(let x=1.125;x<7;x+=.25){const slope=x<2?Math.atan(.45):x>6?-Math.atan(.45):0;b.box([x,habitatElevation(x,-2.2)-.035,-2.2],[.238/Math.cos(slope),.065,2],wood,[0,0,slope]);}
    for(const z of [-3.23,-1.17]){
      const points:Point3[]=[];
      for(let x=1.8;x<=6.2;x+=.55){const y=habitatElevation(x,-2.2);b.cylinder([x,y+.43,z],.025,.86,bronze);points.push([x,y+.86,z]);}
      b.tube(points,.038,bronze);
    }
    // Task benches are individual, with physical tools suited to their roles.
    for(const s of HABITAT_STATIONS){
      if(s.id==='henry')continue;
      const [x,y,stationZ]=s.position,z=stationZ+.15;
      b.box([x,y+.92,z-1.04],[s.id==='growth_measurement'?1.6:.72,.06,.40],s.id==='growth_measurement'?soil:dark);
      b.box([x+.66,y+1.15,z-1.05],[.035,.65,.035],bronze);b.sphere([x+.66,y+1.45,z-1.05],[.12,.07,.12],glow);
      if(['librarian','reader','financier','picker'].includes(s.id)){
        for(let i=0;i<4;i++)b.box([x-.55,y+.915+i*.05,z-.94],[.34,.05,.42],[cloth,wood,edge][i%3],[0,i*.07,0]);
        b.box([x,y+.97,z-.92],[.58,.035,.37],cloth,[.10,0,0]);b.box([x+.05,y+1.01,z-.92],[.014,.014,.34],bronze);
      }else if(s.id==='forecaster'){
        b.cylinder([x,y+1.25,z-1.03],.035,.6,bronze);b.ring([x,y+1.65,z-1.03],.47,.025,bronze,[Math.PI/3,0,0]);b.ring([x,y+1.65,z-1.03],.4,.017,bronze,[0,Math.PI/4,0]);b.sphere([x,y+1.65,z-1.03],[.18,.18,.18],glass);
      }else if(s.id==='growth_measurement'){
        for(let i=0;i<5;i++){b.cylinder([x-.6+i*.26,y+1.05,z-.96],.095,.2,edge,.12);b.tube([[x-.6+i*.26,y+1.12,z-.96],[x-.61+i*.26,y+1.4+i%2*.15,z-.94]],.012,soil);b.sphere([x-.57+i*.26,y+1.35,z-.94],[.07,.035,.13],soil);}
      }else{
        b.cylinder([x,y+1.02,z-1.26],.032,.14,bronze);
        b.box([x,y+1.2,z-1.26],[.78,.46,.05],bronze,[-.15,0,0]);b.box([x,y+1.2,z-1.225],[.70,.38,.015],glass,[-.15,0,0]);
        for(let i=0;i<5;i++)b.box([x-.25+i*.12,y+.96,z-.88],[.085,.02,.1],edge);
      }
    }
    // One continuous water court: a supported upper basin spills into a
    // receiving rill and the bridge pool. Coping follows the water's edges.
    // Beds sit below the water planes so translucency shows real depth rather
    // than a dark plate millimetres under the surface. Collision solids stay
    // the conservative HABITAT_SOLIDS footprints.
    const wetBed=material('#1a3334',.62);
    for(const x of [2,6]){
      b.box([x,.15,-2],[.22,.3,6.25],stone);
      b.box([x,.307,-2],[.25,.035,6.28],edge);
      b.box([x+(x===2?.13:-.13),.20,-2],[.012,.015,5.95],bronze);
    }
    b.box([4,.15,1],[4.2,.3,.22],stone);
    for(const x of [2.25,5.75])b.box([x,.15,-5],[.72,.3,.22],stone);
    // Hollow raised basin: outer walls + recessed floor (was a solid stone fill
    // topped by a near-surface plate that erased any sense of depth). Inner
    // faces sit at x≈2.48/5.52 and z≈-9.32/-6.68; the bed fills that span.
    b.box([2.34,.61,-8],[.28,1.22,3.2],stone);
    b.box([5.66,.61,-8],[.28,1.22,3.2],stone);
    b.box([4,.61,-9.46],[3.04,1.22,.28],stone);
    b.box([4,.55,-6.54],[3.04,1.1,.28],stone);
    b.box([...WATER_COURT.beds.upper.center],[...WATER_COURT.beds.upper.size],wetBed);
    for(const x of [2.34,5.66]){
      b.box([x,1.285,-8.04],[.25,.19,3.45],stone);
      b.box([x,1.39,-8.04],[.29,.035,3.48],edge);
      b.box([x+(x<4?.13:-.13),1.29,-8.04],[.012,.012,3.30],bronze);
    }
    b.box([4,1.285,-9.73],[3.6,.19,.22],stone);
    b.box([4,1.236,-6.40],[2.84,.035,.20],bronze);
    // Shallow wet beds under the receiving rill and bridge pool.
    b.box([...WATER_COURT.beds.rill.center],[...WATER_COURT.beds.rill.size],wetBed);
    b.box([...WATER_COURT.beds.lower.center],[...WATER_COURT.beds.lower.size],wetBed);
    for(const x of [2.48,5.52]){
      b.box([x,.15,-5.67],[.18,.3,1.55],stone);
      b.box([x,.315,-5.67],[.21,.03,1.59],edge);
    }
    for(let i=0;i<7;i++)b.box([2.66+i*.45,.30,-6.39],[.24,.45,.015],dark);
    for(const [x,z,a] of [[-4.5,5.3,0],[-5.3,1.5,Math.PI/2]]){
      b.sphere([x,.63,z],[.55,.17,.25],cloth);b.box([x,.79,z+.13],[.65,.27,.11],cloth,[.18,a,0]);
      for(let i=0;i<7;i++)b.box([x-.26+i*.085,.944,z+.13],[.015,.005,.1],wood,[0,a,0]);
    }
    // Everyday objects meet the measured tabletop rather than floating above it.
    const tableY=COMMONS_SURFACE_Y;
    for(const [x,z] of [[-3.7,3.4],[-2.5,2.8]]){b.add(new LatheGeometry([[0,0],[.060,0],[.080,.02],[.086,.15],[.082,.17],[.071,.17],[.067,.035],[0,.035]].map(p=>new Vector2(...p as [number,number])),32),[x,tableY,z],[1,1,1],edge);b.cylinder([x,tableY+.139,z],.068,.004,wood);b.ring([x+.105,tableY+.105,z],.062,.014,bronze,[0,0,0]);}
    b.box([-3.2,tableY+.0225,3.7],[.5,.045,.34],wood,[0,.2,0]);b.box([-3.2,tableY+.051,3.7],[.46,.012,.30],cloth,[0,.2,0]);
    // Field notebook edge, page block and a small brass pencil make the table
    // read as a used work surface in both arrival directions.
    b.box([-3.2,tableY+.062,3.7],[.40,.012,.25],edge,[0,.2,0]);
    b.cylinder([-3.02,tableY+.083,3.70],.012,.31,bronze,.012,[0,0,Math.PI/2]);
    // Low path lights establish depth instead of a field of bright pin lights.
    for(const x of [-5.2,8.1])for(const z of [-13,-7,1,9]){const y=habitatHeight(z);b.cylinder([x,y+.23,z],.09,.46,bronze);b.cylinder([x,y+.47,z],.065,.05,glow);}
    const group=b.finish('Authored habitat architecture');
    group.children.forEach(mesh=>{if('material' in mesh&&mesh.material===domeGlass)mesh.castShadow=false;});
    const telescope=buildObservatoryTelescope();
    telescope.position.set(OBSERVATORY_PLINTH.x,OBSERVATORY_WING_HEIGHT+OBSERVATORY_PLINTH.height,OBSERVATORY_PLINTH.z);
    telescope.rotation.y=Math.PI/7;
    // The pedestal uses the same owned photographed sandstone as the architecture.
    const replaced=new Set<MeshStandardMaterial>();
    telescope.traverse(o=>{if(o instanceof Mesh&&!Array.isArray(o.material)&&o.material.name==='telescope sandstone'){
      replaced.add(o.material as MeshStandardMaterial);o.material=stone;
    }});replaced.forEach(m=>m.dispose());
    group.add(telescope);
    return {group,materials:[stone,paving,edge,wood,bronze,dark,wetBed,soil,cloth,glass,glow,domeGlass],maps:ownMaps,telescope};
  },[maps]);
  useEffect(()=>()=>{
    const materials=new Set(built.materials);
    built.telescope.traverse(o=>{if(o instanceof Mesh)for(const m of Array.isArray(o.material)?o.material:[o.material])materials.add(m as MeshStandardMaterial);});
    finishDetails(built.group);materials.forEach(m=>m.dispose());built.maps.forEach(t=>t.dispose());
  },[built]);

  return <><primitive object={built.group} dispose={null}/><CraftedLibrary/><CraftedDesks/></>;
}
