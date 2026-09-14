import { apiFetch, type Skill } from '../../../lib/api';
export interface RegisteredAgent { id: string; name: string; role: string; source: string }
export interface SkillProposal { toolUsed: string; argumentShapeHash: string; occurrenceCount: number; description: string; sourceTaskIds: string[] }
export interface ForumRuntimeSnapshot {
  agents: RegisteredAgent[] | null;
  skills: Skill[] | null;
  proposals: SkillProposal[] | null;
  unavailable: string[];
}
/** These are the app's authenticated endpoints; failures never become empty success. */
export async function loadForumRuntime(): Promise<ForumRuntimeSnapshot> {
  const [agents, skills, proposals] = await Promise.allSettled([
    apiFetch<{agents:RegisteredAgent[]}>('/api/agents'),
    apiFetch<Skill[]>('/permagent/skills'),
    apiFetch<SkillProposal[]>('/permagent/skills/proposals'),
  ]);
  return {
    agents: agents.status === 'fulfilled' ? agents.value.agents : null,
    skills: skills.status === 'fulfilled' ? skills.value : null,
    proposals: proposals.status === 'fulfilled' ? proposals.value : null,
    unavailable: [agents.status === 'rejected' ? 'agent registry' : '', skills.status === 'rejected' ? 'skills' : '', proposals.status === 'rejected' ? 'skill proposals' : ''].filter(Boolean),
  };
}
