/** @vitest-environment jsdom */
import { act, StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { ForumActivity } from './ForumActivity';
import { useForumBuild } from './useForumBuild';
import { scheduleForumStartup, setForumStartupActivity, releaseForumStartup } from './forumStartup';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT=true;
it('pauses partially initialized geometry, resumes it, and releases partial and completed owners',()=>{
  vi.useFakeTimers();let slices=0,disposed=0,aborted=0;
  function* build(){let done=false;try{for(let i=0;i<4;i++){slices++;yield;}done=true;return {dispose(){disposed++;}};}finally{if(!done)aborted++;}}
  // Model slow enough slices that a task must yield after each step.
  let clock=0;vi.spyOn(performance,'now').mockImplementation(()=>clock+=12);
  function Scene(){const built=useForumBuild('lifecycle fixture',build,[],value=>value.dispose());return <span>{built?'ready':'loading'}</span>;}
  const host=document.createElement('div'),root=createRoot(host);
  const show=(active:boolean)=>act(()=>root.render(<StrictMode><ForumActivity.Provider value={active}><Scene/></ForumActivity.Provider></StrictMode>));
  try{
    show(true);act(()=>vi.advanceTimersByTime(20));expect(slices).toBe(1);
    show(false);expect(vi.getTimerCount()).toBe(0);
    act(()=>vi.advanceTimersByTime(200));expect(slices).toBe(1);
    show(true);act(()=>vi.advanceTimersByTime(100));expect(host.textContent).toBe('ready');expect(slices).toBe(4);
    act(()=>root.render(null));expect(disposed).toBe(1);
    show(true);act(()=>vi.advanceTimersByTime(20));act(()=>root.render(null));expect(aborted).toBe(1);expect(vi.getTimerCount()).toBe(0);
  }finally{act(()=>root.unmount());vi.useRealTimers();vi.restoreAllMocks();}
});
it('cancels hidden GPU scheduling and resumes only owned tasks',()=>{
  vi.useFakeTimers();const owner={},work=vi.fn(()=>true);
  try{
    setForumStartupActivity(owner,false);scheduleForumStartup('texture upload',work,owner);
    expect(vi.getTimerCount()).toBe(0);
    setForumStartupActivity(owner,true);act(()=>vi.advanceTimersByTime(20));expect(work).toHaveBeenCalledTimes(1);
    scheduleForumStartup('texture upload',work,owner);releaseForumStartup(owner);
    expect(vi.getTimerCount()).toBe(0);act(()=>vi.advanceTimersByTime(100));expect(work).toHaveBeenCalledTimes(1);
  }finally{releaseForumStartup(owner);vi.useRealTimers();}
});
