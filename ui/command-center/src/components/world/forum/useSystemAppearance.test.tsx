/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { useSystemAppearance } from './useSystemAppearance';
import { FORUM_LIGHT } from './ForumLighting';
import { ENV } from '../shared/palette';
let dark=false;
const listeners = new Set<()=>void>();
let root: Root, host: HTMLDivElement;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT=true;
function Probe() { return <span>{useSystemAppearance()}</span>; }
beforeEach(()=>{
  dark=false; listeners.clear();
  vi.stubGlobal('matchMedia',vi.fn(()=>({ get matches(){return dark;}, addEventListener:(_t:string,f:()=>void)=>listeners.add(f),removeEventListener:(_t:string,f:()=>void)=>listeners.delete(f) })));
  host=document.createElement('div');root=createRoot(host);
});
afterEach(()=>{act(()=>root.unmount());vi.unstubAllGlobals();});
it('switches day → night → day on system changes without remounting or a clock',()=>{
  act(()=>root.render(<Probe/>));expect(host.textContent).toBe('day');
  act(()=>{dark=true;listeners.forEach(f=>f());});expect(host.textContent).toBe('night');
  act(()=>{dark=false;listeners.forEach(f=>f());});expect(host.textContent).toBe('day');
  act(()=>root.render(null));expect(listeners.size).toBe(0);
});
it('uses Permagent neutral backgrounds and lower night lighting',()=>{
  expect(FORUM_LIGHT.day.background).toBe(ENV.marble);
  expect(FORUM_LIGHT.night.background).toBe(ENV.deepVoid);
  expect(FORUM_LIGHT.night.strength).toBeLessThan(FORUM_LIGHT.day.strength);
});
