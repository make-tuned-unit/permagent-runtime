/** World W1 — evidence and interaction contract (consumed by W2 / W6). */
export * from './bounds';
export * from './types';
export * from './identity';
export * from './recordLinks';
export * from './evidenceClient';
export * from './snapshotLoader';
export * from './drafts';
export * from './placeSchema';
export { normalizeOrchestrator, normalizeRequests, normalizeActiveGoals, normalizeDecisions, normalizeBrainMemories, normalizeProjectMemories, normalizeNotes, normalizeWorkActivity, normalizeWorkGoals, normalizeWorkSpend, normalizeAgentState, normalizeSkills, normalizeMarket, normalizeCouncil, normalizeGrowth, goalPhase, growthVerdict, requestOutcome } from './normalize';
