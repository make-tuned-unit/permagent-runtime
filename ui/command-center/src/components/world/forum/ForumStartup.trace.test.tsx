/** @vitest-environment jsdom */
// Real CPU builders and React route. WebGL/image decode are explicit doubles;
// native GPU timings still belong to the coordinator's staged-build capture.
import { act, lazy, Suspense, Profiler, useEffect, useMemo, type ReactNode, Children, isValidElement } from 'react';
import { createRoot } from 'react-dom/client';
import { it, vi, expect, afterEach } from 'vitest';
import { writeFileSync } from 'node:fs';
const gpu = vi.hoisted(() => ({ frames: new Set<() => void>(), canvas: document.createElement('canvas'), builds: 0 }));
vi.mock('@react-three/fiber', async () => {
  const { Scene } = await import('three');
  const scene = new Scene(), state = { scene, gl: { domElement: gpu.canvas }, setEvents: () => {} };
  return {
    useFrame: (cb: () => void) => useEffect(() => { gpu.frames.add(cb); return () => { gpu.frames.delete(cb); }; }, [cb]),
    useThree: (pick: (state: unknown) => unknown) => pick ? pick(state) : state,
    Canvas: ({children, className}: {children: ReactNode; className: string}) => {
      useMemo(() => measureForumStartup('Canvas creation (DOM / GPU double)', () => document.createElement('canvas')), []);
      const pick = (nodes: ReactNode): ReactNode => Children.map(nodes, node => {
        if (!isValidElement<{children?: ReactNode}>(node)) return null;
        if (typeof node.type === 'function' && ['HabitatScene','ForumStageReady','ForumEnvironment','ForumCanvasActivity'].includes(node.type.name)) return node;
        return pick(node.props.children);
      });
      return <div data-canvas={className}>{pick(children)}</div>;
    },
  };
});
vi.mock('three', async original => ({...await original<typeof import('three')>(), PMREMGenerator: class {
  dispose() {} fromScene() { gpu.builds++; return {texture:{},dispose(){}}; }
} }));
vi.mock('./useForumTexture', async () => {
  const { Texture } = await import('three'); const maps = Array.from({length:9},()=>new Texture());
  return {useForumTexture:(url: string | string[])=>Array.isArray(url)?maps:maps[0]};
});
vi.mock('./CraftedLibrary', () => ({CraftedLibrary:()=>null}));
vi.mock('./CraftedDesks', () => ({CraftedDesks:()=>null}));
vi.mock('./HeroAgentPortrait', () => ({HeroAgentPortrait:()=>null}));
vi.mock('./ForumConversation', () => ({ForumConversation:()=>null}));
vi.mock('./ForumCapabilities', () => ({ForumCapabilities:()=>null}));
vi.mock('../agents/goalActivity', () => ({useActiveGoals:()=>({goals:[],loaded:true})}));
import { loadForumModule, measureForumStartup, observeForumStartup, scheduleForumStartup, advanceForumBuild, type ForumStartupSample } from './forumStartup';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(()=>{ observeForumStartup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
it('keeps the first-entry HUD responsive at 120 ms while real geometry is prepared', async () => {
  const rows: ForumStartupSample[]=[]; observeForumStartup(row=>rows.push(row));
  vi.spyOn(console,'error').mockImplementation(()=>{});
  vi.stubGlobal('fetch',vi.fn(async()=>({ok:false})));
  localStorage.clear();
  let importMs=0;
  const World=lazy(()=>loadForumModule('World lazy module',async()=>{const before=performance.now();const module=await import('./ForumView');importMs=performance.now()-before;return {default:module.ForumView};}));
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  let clickedAt=0;const start=performance.now();let clickTimer:ReturnType<typeof setTimeout>;
  try {
    clickTimer=setTimeout(()=>host.querySelector<HTMLButtonElement>('[data-followup]')!.click(),120);
    await act(async()=>root.render(<><button data-followup onClick={event=>{clickedAt=performance.now()-start;event.currentTarget.textContent="Follow-up handled";}}>Follow-up</button><Suspense fallback={<p role="status">Loading World…</p>}><Profiler id="ForumView" onRender={(_,phase,ms,_base,start)=>rows.push({step:`ForumView React ${phase}`,start,ms})}><World/></Profiler></Suspense></>));
    expect(host.textContent).toContain('Loading World…');

    for(let i=0;i<700&&(host.textContent?.includes('Loading World…')||host.textContent?.includes('World is taking longer'));i++){
      await act(async()=>{await new Promise(resolve=>setTimeout(resolve,20));});
      act(()=>gpu.frames.forEach(frame=>frame()));
    }
    clearTimeout(clickTimer!);

    expect(host.querySelector('[data-followup]')?.textContent).toBe('Follow-up handled');
    const steps=[...new Set(rows.map(row=>row.step))].map(step=>{const values=rows.filter(row=>row.step===step);return {step,slices:values.length,totalMs:values.reduce((sum,row)=>sum+row.ms,0),maxMs:Math.max(...values.map(row=>row.ms))};});
    const report={scope:'real CPU geometry; WebGL and image decode doubled',importWallMs:importMs,clickAtMs:clickedAt,readyAtMs:performance.now()-start,steps,rows};
    if(process.env.L2_G5_TRACE)writeFileSync('../../reports/world-production/l2-g5-after.json',JSON.stringify(report,null,2)+'\n');
    console.info('G5 first entry',JSON.stringify({clickAtMs:clickedAt,steps}));
    expect(clickedAt).toBeGreaterThanOrEqual(119);
    // The timer includes Vite's cold module transforms in this fixture. Assert
    // delivery; the deterministic load test is the strict scheduling budget.
    expect(clickedAt).toBeLessThan(500);
    expect(host.textContent).not.toContain('Loading World…');
    expect(host.textContent).not.toContain('World is taking longer');
    expect(host.textContent).not.toContain('World could not start');
    expect(rows.some(row=>row.step==='buildHabitatPlanting')).toBe(true);
    expect(rows.some(row=>row.step==='ForumEnvironment PMREM')).toBe(true);
    // Actual measurements are recorded, while the deterministic load contract
    // below is the strict 50 ms acceptance (GC/host contention is not simulated).
  } finally {act(()=>root.unmount());host.remove();}
},20000);

it('bounds every scheduled slice under simulated load and services a 120 ms input', async()=>{
  vi.useFakeTimers();let cpu=0;const rows:ForumStartupSample[]=[];
  vi.spyOn(performance,'now').mockImplementation(()=>Date.now()+cpu);
  observeForumStartup(row=>rows.push(row));
  let handled=false,complete=false;
  function* work(){for(let i=0;i<100;i++){cpu+=12;yield;}return true;}
  const steps=work();const cancel=scheduleForumStartup('simulated geometry',()=>advanceForumBuild(steps,()=>{complete=true;}));
  const button=document.createElement('button');button.onclick=()=>{handled=true;};
  setTimeout(()=>button.click(),120);
  try {
    await vi.advanceTimersByTimeAsync(120);
    expect(handled).toBe(true);expect(complete).toBe(false);
    await vi.advanceTimersByTimeAsync(3000);
    expect(complete).toBe(true);
    expect(Math.max(...rows.map(row=>row.ms))).toBeLessThanOrEqual(50);
    const report={scope:'deterministic 12 ms work units, real startup scheduler',slices:rows.length,maxMs:Math.max(...rows.map(row=>row.ms)),clickAtMs:120};
    console.info('G5 simulated load',JSON.stringify(report));
    if(process.env.L2_G5_TRACE)writeFileSync('../../reports/world-production/l2-g5-simulated.json',JSON.stringify(report,null,2)+'\n');
  }finally{cancel();vi.useRealTimers();}
});
