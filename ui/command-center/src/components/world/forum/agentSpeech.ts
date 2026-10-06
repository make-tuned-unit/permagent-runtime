/** Only actual output audio opens the mouth; listening and thinking stay closed. */
export function speechOpening(state:string,bins:Uint8Array|null):number {
  if(state!=='playing'||!bins?.length)return 0;
  const n=Math.max(1,Math.floor(bins.length*.65));let energy=0;
  for(let i=0;i<n;i++)energy+=bins[i]/255;
  return Math.min(1,Math.max(0,(energy/n-.035)*2.8));
}
export function smoothSpeech(previous:number,target:number,dt:number){return previous+(target-previous)*(1-Math.exp(-Math.min(Math.max(dt,0),.1)*(target>previous?28:18)));}
