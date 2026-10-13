import { act } from 'react';
import { vi } from 'vitest';
/** Advance paint/task turns in GPU fixtures. Each turn has its own React commit. */
export function settleStartup(): void | Promise<void> {
  if (vi.isFakeTimers()) {
    for (let i=0;i<8;i++) act(()=>vi.advanceTimersByTime(20));
    return;
  }
  return (async()=>{for(let i=0;i<8;i++)await act(async()=>{await new Promise(resolve=>setTimeout(resolve,20));});})();
}
