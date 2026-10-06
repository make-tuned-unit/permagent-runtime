// Reproduce the frozen runtime texture conversion with locked local sharp.
import fs from 'node:fs/promises';
import {createRequire} from 'node:module';
import {createHash} from 'node:crypto';
const require=createRequire(new URL('../../../ui/command-center/package.json',import.meta.url));
const sharp=require('sharp'),here=new URL('./',import.meta.url),output=new URL('../../../ui/command-center/public/world/garden-materials/',import.meta.url);
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const surfaces=JSON.parse(await fs.readFile(new URL('surface-provenance.json',here),'utf8'));
for(const map of surfaces){
  const file=`${map.asset}_${map.channel.toLowerCase()}_2k.webp`,target=new URL(file,output);
  await sharp(new URL(map.file,here).pathname).webp({quality:92,effort:6}).toFile(target.pathname);
  const bytes=await fs.readFile(target);map.runtime={file:`ui/command-center/public/world/garden-materials/${file}`,bytes:bytes.length,sha256:hash(bytes)};
}
await fs.writeFile(new URL('surface-provenance.json',here),JSON.stringify(surfaces,null,2)+'\n');
const foliage=JSON.parse(await fs.readFile(new URL('foliage-provenance.json',here),'utf8'));
for(const [name,reference]of [['laurel-leaf','leaf-v1.png'],['fern-frond','fern-frond-v1.png']]){
  const target=new URL(`${name}-v1.webp`,output);
  await sharp(new URL(`references/${reference}`,here).pathname).resize({height:1024}).webp({quality:95,alphaQuality:100,effort:6}).toFile(target.pathname);
  const bytes=await fs.readFile(target);foliage[name].bytes=bytes.length;foliage[name].sha256=hash(bytes);
}
await fs.writeFile(new URL('foliage-provenance.json',here),JSON.stringify(foliage,null,2)+'\n');
console.log('Converted six surface maps and two original alpha foliage textures.');
