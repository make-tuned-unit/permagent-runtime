/** @vitest-environment jsdom */
import { beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(()=>({
  isStreaming:false, sessionLoadError:null as string|null,
  ensureSession:vi.fn(), loadSessionMessages:vi.fn(), connectSession:vi.fn(), sendMessage:vi.fn(),
}));
vi.mock('../../../lib/store',()=>({useCommandCenter:{getState:()=>state}}));
import { askFromForum } from './forumQuery';
beforeEach(()=>{
  vi.clearAllMocks(); state.isStreaming=false; state.sessionLoadError=null;
  state.ensureSession.mockResolvedValue('session-1'); state.loadSessionMessages.mockResolvedValue(undefined);
  state.connectSession.mockResolvedValue(undefined); state.sendMessage.mockResolvedValue(undefined);
});
it('uses the existing session and connects its event channel before sending context to the orchestrator',async()=>{
  await askFromForum('How do you verify memory?','The Librarian','Cite sources');
  expect(state.connectSession).toHaveBeenCalledWith('session-1');
  expect(state.connectSession.mock.invocationCallOrder[0]).toBeLessThan(state.sendMessage.mock.invocationCallOrder[0]);
  const text=state.sendMessage.mock.calls[0][0];
  expect(text).toContain('not a claim that this specialist has been directly assigned');
  expect(text).toContain('Cite sources');
});
it('does not create or send while another reply is active',async()=>{
  state.isStreaming=true;
  await expect(askFromForum('Question','Henry','')).rejects.toThrow('current reply');
  expect(state.ensureSession).not.toHaveBeenCalled(); expect(state.sendMessage).not.toHaveBeenCalled();
});
it('retains the draft on a session connection failure',async()=>{
  state.ensureSession.mockResolvedValue(null);
  await expect(askFromForum('Question','Henry','')).rejects.toThrow('brief is still here');
  expect(state.sendMessage).not.toHaveBeenCalled();
});
it('does not send over a failed history load',async()=>{
  state.sessionLoadError='offline';
  await expect(askFromForum('Question','Henry','')).rejects.toThrow('load the conversation');
  expect(state.sendMessage).not.toHaveBeenCalled();
});
it('sends a job execution request through the shared orchestrator without claiming direct specialist dispatch',async()=>{
 await askFromForum('Build the report','The Librarian','Provide the report',undefined,'job');
 const text=state.sendMessage.mock.calls[0][0];
 expect(text).toContain('Job request');expect(text).toContain('track progress and evidence');
 expect(text).toContain('not a claim that this specialist has been directly assigned');
});
it('uses conversation context and the existing permissions for capability growth',async()=>{
 await askFromForum('Learn to verify exports','Henry','Test a representative export',undefined,'capability');
 const text=state.sendMessage.mock.calls[0][0];
 expect(text).toContain('Capability development request');expect(text).toContain('register the reusable skill with evidence');
 expect(text).toContain('Preserve the runtime permissions and approval flow');
});
