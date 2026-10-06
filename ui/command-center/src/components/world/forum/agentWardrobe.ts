import { BufferGeometry, DoubleSide, Float32BufferAttribute } from 'three';
import { DetailBuilder, material } from './habitatGeometry';
import { stationFor, type Point3 } from './habitatLayout';

function mantle(top:number,bottom:number,length:number,angle:number,start:number) {
  const p:number[]=[],uv:number[]=[],ix:number[]=[],rows=10,cols=36;
  for(let j=0;j<=rows;j++)for(let i=0;i<=cols;i++){
    const t=j/rows,a=start+angle*i/cols,r=top+(bottom-top)*t+Math.sin(i/cols*Math.PI*14)*.013*t;
    p.push(Math.sin(a)*r,1.19-length*t,Math.cos(a)*r);uv.push(i/cols,t);
    if(j<rows&&i<cols){const k=j*(cols+1)+i;ix.push(k,k+1,k+cols+2,k,k+cols+2,k+cols+1);}
  }
  const g=new BufferGeometry();g.setAttribute('position',new Float32BufferAttribute(p,3));g.setAttribute('uv',new Float32BufferAttribute(uv,2));g.setIndex(ix);g.computeVertexNormals();return g;
}
export function createAgentWardrobe(id:string) {
  const s=stationFor(id),cloth=material(s.cloth,.95),trim=material(s.color,.45,.45),leather=material('#493e32'),paper=material('#d7cbb0'),dark=material('#173334',.25,.2);
  cloth.side=DoubleSide;const b=new DetailBuilder(),head=new DetailBuilder();
  const cape=(length=.7,angle=Math.PI*1.25)=>b.add(mantle(.255,.37,length,angle,Math.PI-angle/2),[0,0,0],[1,1,1],cloth);
  const sash=()=>{b.box([-.035,.94,.198],[.12,.48,.022],cloth,[0,0,-.45]);b.box([.075,.78,.224],[.1,.07,.025],trim,[0,0,-.45]);};
  const book=(at:Point3,size:Point3=[.3,.045,.24])=>{b.box(at,size,leather,[-.38,0,0]);b.box([at[0],at[1]+.018,at[2]+.008],[size[0]*.91,.025,size[2]*.92],paper,[-.38,0,0]);};
  b.ring([0,1.2,0],.16,.022,trim);
  if(id==='henry') {cape(.75);sash();b.ring([-.25,1.10,.10],.065,.018,trim,[0,0,0]);}
  if(id==='librarian'){
    cape(.76,Math.PI*1.62);b.cylinder([0,1.19,0],.19,.17,cloth,.21);b.box([0,.93,-.24],[.40,.5,.13],leather);
    for(let i=0;i<5;i++){b.box([-.16+i*.08,.96,-.33],[.06,.40+(i%2)*.07,.1],paper);b.box([-.16+i*.08,.84,-.39],[.035,.025,.01],trim);}book([0,.83,.31]);
    head.ring([.18,0,.263],.082,.014,trim,[0,0,0]);
  }
  if(id==='reader') {cape(.39,Math.PI*1.4);book([0,.92,.33],[.43,.045,.28]);head.ring([-.15,.01,.276],.105,.018,trim,[0,0,0]);head.cylinder([-.15,.01,.28],.09,.02,dark,.09,[Math.PI/2,0,0]);}
  if(id==='watcher') {
    cape(.61,Math.PI*.98);b.box([-.34,.77,.10],[.18,.12,.14],dark);b.cylinder([.19,1.4,-.2],.016,.7,trim);b.sphere([.19,1.77,-.2],[.04,.04,.04],trim);head.box([0,.12,.12],[.68,.045,.32],cloth,[.12,0,0]);
  }
  if(id==='steward') {
    b.box([0,.89,.207],[.39,.51,.035],cloth);b.box([0,.63,.20],[.54,.12,.065],leather);
    for(const x of [-.16,0,.16]){b.box([x,.77,.24],[.10,.11,.04],leather);b.cylinder([x,.80,.27],.015,.16,trim);}b.box([-.3,.7,.06],[.04,.36,.045],trim,[0,0,-.2]);b.ring([-.33,.87,.06],.066,.021,trim,[0,0,0]);head.box([0,.19,.10],[.43,.03,.21],trim);
  }
  if(id==='strix'){
    for(const sign of [-1,1]){b.sphere([sign*.3,1.09,0],[.17,.12,.18],cloth);b.sphere([sign*.31,1.12,0],[.16,.06,.18],trim);head.box([sign*.12,.29,0],[.07,.22,.12],trim,[0,0,sign*.24]);}
    b.box([.42,.80,.12],[.08,.35,.25],cloth);b.box([.47,.82,.12],[.012,.19,.14],trim);b.box([0,.95,.22],[.28,.22,.055],cloth);
  }
  if(id==='financier'){
    cape(.60,Math.PI*1.72);for(const x of [-.08,.08])for(let i=0;i<3;i++)b.sphere([x,.88+i*.09,.25],[.018,.018,.009],trim);book([-.28,.85,.22],[.19,.045,.34]);b.tube([[.1,1.1,.2],[.2,.92,.22],[.12,.81,.25]],.009,trim);
  }
  if(id==='forecaster'){
    cape(.84,Math.PI*1.5);sash();b.ring([-.29,.91,.29],.22,.018,trim,[Math.PI/5,0,.2]);b.ring([-.29,.91,.29],.18,.012,trim,[0,Math.PI/4,0]);b.cylinder([.28,.85,-.15],.06,.56,trim,.05,[.3,0,.12]);head.ring([0,.13,0],.3,.024,trim);
  }
  if(id==='council'){
    cape(.8,Math.PI*1.4);for(const x of [-.16,.16])b.box([x,.85,.235],[.105,.67,.026],cloth);b.box([0,1.1,0],[.7,.055,.27],trim);head.ring([0,.25,0],.19,.027,trim);for(let i=0;i<3;i++)head.box([Math.sin(i*2.094)*.18,.34,Math.cos(i*2.094)*.18],[.055,.18,.045],trim);
  }
  if(id==='polybot'){
    b.box([0,.92,-.27],[.39,.49,.24],cloth);for(let i=0;i<6;i++)b.box([-.2+i*.08,.95,-.40],[.025,.43,.11],trim);for(const x of [-.17,.17])b.box([x,.95,.2],[.06,.5,.045],leather);head.box([0,.12,.23],[.60,.08,.15],cloth);head.box([0,-.12,.23],[.60,.04,.15],cloth);
  }
  if(id==='picker'){
    cape(.36,Math.PI*.95);head.cylinder([0,.26,0],.44,.045,cloth,.44);head.cylinder([0,.32,0],.25,.15,cloth,.21);head.ring([0,.285,0],.26,.018,trim);b.box([.28,.72,-.03],[.19,.25,.25],leather);sash();for(let i=0;i<3;i++)book([-.25+i*.05,.86+i*.018,.27],[.16,.022,.22]);
  }
  if(id==='growth_measurement'){
    b.box([0,.82,.215],[.41,.60,.025],cloth);b.box([0,.95,-.24],[.32,.40,.12],leather);for(let i=0;i<4;i++){b.tube([[i*.08-.12,1.02,-.3],[i*.10-.15,1.47+(i%2)*.14,-.28]],.009,trim);b.sphere([i*.10-.15,1.44+(i%2)*.14,-.28],[.055,.09,.013],cloth);}b.box([.38,.65,.1],[.027,1.1,.04],trim);for(let i=0;i<8;i++)b.box([.4,.3+i*.1,.125],[.04,.014,.005],paper);
  }
  const body=b.finish(`${id} bespoke outfit`),headwear=head.finish(`${id} headwear`);
  return {body,headwear,materials:[cloth,trim,leather,paper,dark]};
}
