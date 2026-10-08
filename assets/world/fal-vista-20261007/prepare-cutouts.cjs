/** Extract actual alpha from inspected solid-background FAL outputs. */
const path=require('node:path'),fs=require('node:fs'),crypto=require('node:crypto');
const sharp=require('../../../ui/command-center/node_modules/sharp');
const root=path.resolve(__dirname,'../../..');
const records=[];
const clamp=x=>Math.max(0,Math.min(1,x));
async function cutout(name,pipe,white=false,outDir='cards') {
  const {data,info}=await pipe.removeAlpha().raw().toBuffer({resolveWithObject:true});
  const rgba=Buffer.alloc(info.width*info.height*4);let zero=0,solid=0;
  for(let i=0;i<info.width*info.height;i++){
    const rgb=[data[i*3],data[i*3+1],data[i*3+2]];
    let a=white?clamp((248-Math.min(...rgb))/38):clamp((Math.max(...rgb)-8)/42);
    a=a*a*(3-2*a);
    if(a===0)zero++;if(a===1)solid++;
    for(let c=0;c<3;c++)rgba[i*4+c]=a>0?Math.round(Math.max(0,Math.min(255,(rgb[c]-(white?(1-a)*255:0))/a))):0;
    rgba[i*4+3]=Math.round(a*255);
  }
  // Trial mattes are archival only. Accepted runtime cards use semantic FAL mattes.
  const folder=path.join(__dirname,outDir==='cards'?'threshold-cards':'botanical/cutouts');fs.mkdirSync(folder,{recursive:true});
  const file=path.join(folder,name+'.png');
  await sharp(rgba,{raw:{width:info.width,height:info.height,channels:4}}).png().toFile(file);
  const b=fs.readFileSync(file);
  records.push({name,path:path.relative(root,file),width:info.width,height:info.height,bytes:b.length,sha256:crypto.createHash('sha256').update(b).digest('hex'),alpha_zero_fraction:zero/(info.width*info.height),alpha_opaque_fraction:solid/(info.width*info.height),background: white?'white':'black'});
  await sharp(rgba,{raw:{width:info.width,height:info.height,channels:4}}).flatten({background:'#91b5ba'}).resize({width:1000}).png().toFile(path.join(__dirname,name+'-alpha-preview.png'));
}
async function main(){
  for(const name of ['cliff-waterfall','floating-islet'])await cutout(name,sharp(path.join(__dirname,name,'panorama.png')).resize({width:2048}));
  const atlas=path.join(__dirname,'botanical/panorama.png'),meta=await sharp(atlas).metadata();
  const cellW=Math.floor(meta.width/3),cellH=Math.floor(meta.height/2),margin=10;
  const names=['vines','canopy','fern','flowers','moss','vines-alt'];
  for(let i=0;i<6;i++)await cutout(names[i],sharp(atlas).extract({left:i%3*cellW+margin,top:Math.floor(i/3)*cellH+margin,width:cellW-margin*2,height:cellH-margin*2}).resize({width:1024}),true,'botanical');
  fs.writeFileSync(path.join(__dirname,'cutouts.json'),JSON.stringify({method:'Sharp deterministic alpha extraction from inspected black/white matte, edge unpremultiplication, atlas grid cropped out. Original images preserved.',artifacts:records},null,2)+'\n');
  console.log(JSON.stringify(records));
}
main().catch(e=>{console.error(e);process.exitCode=1;});
