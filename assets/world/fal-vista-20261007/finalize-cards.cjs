/** Publish inspected semantic matte outputs, preserving actual alpha. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const sharp=require('../../../ui/command-center/node_modules/sharp');
const root=path.resolve(__dirname,'../../..');
(async()=>{
  const records=[];
  for(const name of ['cliff-waterfall','floating-islet']){
    const source=path.join(__dirname,name+'-alpha/panorama.png');
    const file=path.join(root,'ui/command-center/public/world/vista/cards',name+'.png');
    fs.mkdirSync(path.dirname(file),{recursive:true});
    const meta=await sharp(source).metadata();
    if(!meta.hasAlpha)throw new Error('Semantic matte must have alpha');
    await sharp(source).resize({width:2048}).png().toFile(file);
    const {data,info}=await sharp(file).raw().toBuffer({resolveWithObject:true});
    let zero=0,opaque=0;
    for(let i=3;i<data.length;i+=4){if(data[i]===0)zero++;if(data[i]===255)opaque++;}
    const b=fs.readFileSync(file);
    records.push({name,path:path.relative(root,file),width:info.width,height:info.height,hasAlpha:true,bytes:b.length,sha256:crypto.createHash('sha256').update(b).digest('hex'),alpha_zero_fraction:zero/(info.width*info.height),alpha_opaque_fraction:opaque/(info.width*info.height),method:'FAL Ideogram semantic background removal; resized to 2048 wide preserving true alpha'});
  }
  fs.writeFileSync(path.join(__dirname,'cards-final.json'),JSON.stringify(records,null,2)+'\n');
  console.log(JSON.stringify(records));
})().catch(e=>{console.error(e);process.exitCode=1;});
