import { beforeEach, expect, it, vi } from 'vitest';
const fetch = vi.hoisted(()=>vi.fn());
vi.mock('../../../lib/api',()=>({apiFetch:fetch}));
import { loadForumRuntime } from './forumRuntime';
beforeEach(()=>fetch.mockReset());
it('loads the registered workers and growing skill library through the authenticated API client',async()=>{
 fetch.mockResolvedValueOnce({agents:[{id:'new_worker',name:'New worker',role:'Research',source:'configured'}]})
  .mockResolvedValueOnce([{id:'skill-1',name:'Verified export'}]).mockResolvedValueOnce([{description:'Repeated export',sourceTaskIds:['task-1']}]);
 const data=await loadForumRuntime();
 expect(fetch.mock.calls.map(c=>c[0])).toEqual(['/api/agents','/permagent/skills','/permagent/skills/proposals']);
 expect(data.agents?.[0].id).toBe('new_worker');expect(data.skills).toHaveLength(1);expect(data.proposals).toHaveLength(1);expect(data.unavailable).toEqual([]);
});
it('keeps unavailable feeds distinct from successfully empty feeds',async()=>{
 fetch.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce([]).mockRejectedValueOnce(new Error('unauthorized'));
 const data=await loadForumRuntime();
 expect(data.agents).toBeNull();expect(data.skills).toEqual([]);expect(data.proposals).toBeNull();
 expect(data.unavailable).toEqual(['agent registry','skill proposals']);
});
