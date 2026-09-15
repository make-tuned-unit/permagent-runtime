import { useCommandCenter } from '../../../lib/store';

/** Reuse the app's conversation, authenticated SSE, trace and cancellation.
 * Selecting a character supplies context; it never claims specialist routing.
 * This function is invoked only by the explicit Ask button, never by a mount.
 */
export async function askFromForum(question: string, agentName: string, criteria: string, voice?: string, intent: 'query' | 'job' | 'capability' = 'query'): Promise<void> {
  if (!question.trim()) throw new Error('Write a question first.');
  if (useCommandCenter.getState().isStreaming) throw new Error('Wait for the current reply, or stop it first.');
  let sessionId = await useCommandCenter.getState().ensureSession();
  if (!sessionId) throw new Error('Could not connect to a conversation. Your brief is still here.');
  await useCommandCenter.getState().loadSessionMessages(sessionId);
  if (useCommandCenter.getState().sessionLoadError) throw new Error('Could not load the conversation. Try again when connected.');
  // A stale persisted session is cleared by the canonical loader on 404.
  sessionId = await useCommandCenter.getState().ensureSession();
  if (!sessionId) throw new Error('Could not create a conversation.');
  if (useCommandCenter.getState().isStreaming) throw new Error('A reply is already running in this conversation.');
  await useCommandCenter.getState().connectSession(sessionId);
  const label = intent === 'query' ? 'Question' : intent === 'job' ? 'Job request' : 'Capability development request';
  const instruction = intent === 'job'
    ? 'Execute this brief using the existing runtime and track progress and evidence in the appropriate project or task.'
    : intent === 'capability'
      ? 'Use our conversation context and inspect existing skills and tools. Implement and test the needed capability through the existing runtime, then register the reusable skill with evidence. Preserve the runtime permissions and approval flow.'
      : '';
  await useCommandCenter.getState().sendMessage(
    `${label} from the World forum. Selected agent for context: ${agentName}. This is a request to the orchestrator, not a claim that this specialist has been directly assigned.${voice ? `\nPresentation preference: ${voice}` : ''}${instruction ? `\n${instruction}` : ''}\n\n${question.trim()}${criteria.trim() ? `\n\nUseful answer criteria: ${criteria.trim()}` : ''}`,
  );
}
