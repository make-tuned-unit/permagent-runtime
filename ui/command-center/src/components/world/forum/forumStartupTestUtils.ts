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

/** The forum Canvas' scene children, looking through its in-Canvas Suspense boundary. */
export function sceneChildren(props: { children?: unknown }): any[] {
  const out: any[] = [];
  const visit = (node: any) => {
    if (Array.isArray(node)) { node.forEach(visit); return; }
    if (node?.type === Symbol.for('react.suspense')) { visit(node.props.children); return; }
    out.push(node);
  };
  visit(props.children);
  return out;
}
