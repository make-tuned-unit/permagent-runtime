/** @vitest-environment jsdom */
import { expect, it, vi } from 'vitest';
import { MeshoptDecoder } from 'three/examples/jsm/libs/meshopt_decoder.module.js';
import { extendForumLoader } from './forumGltf';
it('sends compressed geometry to a same-origin worker on a later task',async()=>{
  vi.useFakeTimers();const workers:FakeWorker[]=[];
  class FakeWorker {
    onmessage?: (event: {data:unknown})=>void;
    messages: {id:number;args:unknown[]}[]=[];
    constructor(public url:URL, public options:WorkerOptions){workers.push(this);}
    postMessage(message:{id:number;args:unknown[]}){this.messages.push(message);}
  }
  vi.stubGlobal('Worker',FakeWorker);
  let decoder:typeof MeshoptDecoder|undefined;
  extendForumLoader({setMeshoptDecoder(value){decoder=value;}});
  const sync=vi.spyOn(MeshoptDecoder,'decodeGltfBuffer');
  const source=new Uint8Array([1,2,3]);
  try{
    const result=decoder!.decodeGltfBufferAsync(1,3,source,'ATTRIBUTES');
    expect(workers).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(20);
    expect(workers).toHaveLength(1);
    expect(workers[0].options.type).toBe('module');expect(workers[0].url.pathname).toContain('forumMeshopt.worker.ts');
    const request=workers[0].messages[0];expect(request.args[2]).not.toBe(source);
    expect(sync).not.toHaveBeenCalled();
    const decoded=new Uint8Array([4,5,6]);workers[0].onmessage!({data:{id:request.id,result:decoded}});
    await expect(result).resolves.toBe(decoded);expect(source).toEqual(new Uint8Array([1,2,3]));
  }finally{vi.useRealTimers();vi.unstubAllGlobals();vi.restoreAllMocks();}
});
