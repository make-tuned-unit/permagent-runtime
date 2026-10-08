/** Deterministic runtime resizing and narrow edge/pole treatment; originals retained. */
const path = require('node:path');
const fs = require('node:fs');
const crypto = require('node:crypto');
const sharp = require('../../../ui/command-center/node_modules/sharp');
const root = path.resolve(__dirname, '../../..');
async function main() {
  const candidate = process.argv[2] || 'candidate-a';
  if (!/^candidate-[abc]$/.test(candidate)) throw new Error('Unexpected candidate');
  const source = path.join(__dirname, candidate, 'panorama.png');
  const target = process.argv.includes('--archive-only')
    ? path.join(__dirname, candidate)
    : path.join(root, 'ui/command-center/public/world/vista');
  fs.mkdirSync(target, {recursive:true});
  const w=4096,h=2048;
  const {data}=await sharp(source).resize(w,h,{fit:'fill'}).removeAlpha().raw().toBuffer({resolveWithObject:true});
  const original=Buffer.from(data), seam=96, pole=60;
  // Symmetric edge blend makes both meridian edges identical without mirroring objects.
  for(let y=0;y<h;y++) for(let x=0;x<seam;x++) {
    const t=(1-x/(seam-1))**2;
    for(let c=0;c<3;c++) {
      const a=(y*w+x)*3+c,b=(y*w+w-1-x)*3+c;
      const mean=(original[a]+original[b])/2;
      data[a]=Math.round(original[a]*(1-t)+mean*t);
      data[b]=Math.round(original[b]*(1-t)+mean*t);
    }
  }
  // Collapse the last few pole rows to their mean to avoid pinwheel singularities.
  for(const bottom of [false,true]) for(let dy=0;dy<pole;dy++) {
    const y=bottom?h-1-dy:dy, mean=[0,0,0],t=(1-dy/(pole-1))**2;
    for(let x=0;x<w;x++)for(let c=0;c<3;c++)mean[c]+=data[(y*w+x)*3+c]/w;
    for(let x=0;x<w;x++)for(let c=0;c<3;c++)data[(y*w+x)*3+c]=Math.round(data[(y*w+x)*3+c]*(1-t)+mean[c]*t);
  }
  const raw={raw:{width:w,height:h,channels:3}};
  const artifacts=[];
  for(const width of [4096,2048]) {
    const file=path.join(target,`golden-hour-panorama-${width}.jpg`);
    await sharp(data,raw).resize(width,width/2).jpeg({quality:85,chromaSubsampling:'4:4:4'}).toFile(file);
    const b=fs.readFileSync(file);
    artifacts.push({path:path.relative(root,file),width,height:width/2,bytes:b.length,sha256:crypto.createHash('sha256').update(b).digest('hex')});
  }
  await sharp(data,raw).resize(1536,768).png().toFile(path.join(__dirname,candidate,'runtime-preview.png'));
  const shift=2048;
  const wrapped=Buffer.alloc(data.length);
  for(let y=0;y<h;y++)for(let x=0;x<w;x++)for(let c=0;c<3;c++)wrapped[(y*w+x)*3+c]=data[(y*w+(x+shift)%w)*3+c];
  await sharp(wrapped,raw).resize(1536,768).png().toFile(path.join(__dirname,candidate,'seam-preview.png'));
  const report={candidate,source:path.relative(root,source),derivation:'21:9 source resampled to 2:1, 96-pixel symmetric meridian crossfade and 60-pixel polar convergence at 4096x2048. Not a calibrated native equirectangular capture.',artifacts};
  fs.writeFileSync(path.join(__dirname,candidate,'derivatives.json'),JSON.stringify(report,null,2)+'\n');
  console.log(JSON.stringify(report));
}
main().catch(e=>{console.error(e);process.exitCode=1;});
