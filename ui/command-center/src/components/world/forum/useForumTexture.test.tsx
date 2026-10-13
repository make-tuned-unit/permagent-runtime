/** @vitest-environment jsdom */
import { act, Suspense } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const fixture=vi.hoisted(()=>({textures:[] as unknown[],renderer:{initTexture:vi.fn()}}));
vi.mock('@react-three/fiber',()=>({useLoader:Object.assign(()=>fixture.textures,{clear:vi.fn()}),useThree:(pick:(s:unknown)=>unknown)=>pick({gl:fixture.renderer})}));
import { Texture, SRGBColorSpace } from 'three';
import { ForumTextureLoader, useForumTexture } from './useForumTexture';
import { observeForumStartup, setForumStartupActivity, releaseForumStartup } from './forumStartup';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT=true;
it('uploads texture arrays on separate tasks, shares preparation, and pauses while hidden',async()=>{
  vi.useFakeTimers();fixture.textures=[new Texture(),new Texture(),new Texture()];fixture.renderer.initTexture.mockClear();
  const steps:string[]=[];observeForumStartup(row=>steps.push(row.step));
  const host=document.createElement('div'),root=createRoot(host);
  function Sky(){useForumTexture(['a','b','c']);return <span>ready</span>;}
  try{
    setForumStartupActivity(fixture.renderer,false);
    await act(async()=>root.render(<Suspense fallback="loading"><Sky/></Suspense>));
    expect(host.textContent).toBe('loading');expect(vi.getTimerCount()).toBe(0);
    setForumStartupActivity(fixture.renderer,true);
    await act(async()=>{await vi.advanceTimersByTimeAsync(20);});expect(fixture.renderer.initTexture).toHaveBeenCalledTimes(1);
    await act(async()=>{await vi.advanceTimersByTimeAsync(20);});expect(fixture.renderer.initTexture).toHaveBeenCalledTimes(2);
    await act(async()=>{await vi.advanceTimersByTimeAsync(20);});expect(host.textContent).toBe('ready');
    const versions=fixture.textures.map(texture=>(texture as Texture).version);
    expect(fixture.textures.every(texture=>(texture as Texture).colorSpace===SRGBColorSpace)).toBe(true);
    await act(async()=>root.render(<Suspense fallback="loading"><Sky/><Sky/></Suspense>));
    expect(fixture.textures.map(texture=>(texture as Texture).version)).toEqual(versions);
    expect(fixture.renderer.initTexture).toHaveBeenCalledTimes(3);expect(steps).toEqual(['texture GPU upload','texture GPU upload','texture GPU upload']);
  }finally{act(()=>root.unmount());releaseForumStartup(fixture.renderer);observeForumStartup();vi.useRealTimers();}
});
it('awaits browser bitmap decode without decoding image bytes on the render stack',async()=>{
  let finish:((bitmap:ImageBitmap)=>void)|undefined;
  const bitmap={close:vi.fn()} as unknown as ImageBitmap;
  const decode=vi.fn(()=>new Promise<ImageBitmap>(resolve=>{finish=resolve;}));
  vi.stubGlobal('createImageBitmap',decode);vi.stubGlobal('fetch',vi.fn(async()=>({blob:async()=>new Blob(['image'])})));
  const loaded=vi.fn();
  try{
    const texture=new ForumTextureLoader().load('/world/test.webp',loaded);
    await vi.waitFor(()=>expect(decode).toHaveBeenCalled());
    expect(loaded).not.toHaveBeenCalled();expect(decode).toHaveBeenCalled();
    finish!(bitmap);await vi.waitFor(()=>expect(loaded).toHaveBeenCalled());
    expect(loaded).toHaveBeenCalledWith(texture);expect(texture.flipY).toBe(false);
    texture.dispose();expect(bitmap.close).toHaveBeenCalledTimes(1);
  }finally{vi.unstubAllGlobals();}
});
