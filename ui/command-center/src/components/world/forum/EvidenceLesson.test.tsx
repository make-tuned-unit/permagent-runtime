/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it } from 'vitest';
import { EvidenceLesson } from './EvidenceLesson';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
it('offers corrective feedback for either answer and can be revisited',()=>{
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  act(()=>root.render(<EvidenceLesson />));
  const choose=(name:string)=>{const button=[...host.querySelectorAll('button')].find(b=>b.textContent===name)!;act(()=>button.click());};
  choose('Every meeting room is step-free');expect(host.querySelector('[role=status]')?.textContent).toContain('narrower claim');
  choose('Garden room confirmed; Observatory uncertain');expect(host.querySelector('[role=status]')?.textContent).toContain('Supported by the records');
  choose('Revisit the question');expect(host.querySelector('[role=status]')).toBeNull();
  act(()=>root.unmount());host.remove();
});
