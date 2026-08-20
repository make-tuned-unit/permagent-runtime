/**
 * Turn a growth Action into a prompt a coding agent can actually run.
 *
 * Copy used to dump `action.artifact` raw. For SEO that is often a drafted
 * blog post — no repo, no path, no "what this is" — so pasting it into Claude
 * or Cursor produces a post sitting in chat, not a file in the tree. Every
 * copy/send path goes through here so the agent always gets: what the action
 * is, why, where it belongs, the deliverable, and how to signal done.
 */

export const GROW_ACTION_DONE_PREFIX = 'GROW_ACTION_DONE';

export interface DirectiveAction {
  title: string;
  recommendation: string;
  evidence?: string;
  steps?: string[];
  artifactKind?: string;
  artifact?: string | null;
  category?: string;
  targetMetric?: string | null;
  targetDir?: string | null;
  identity?: {
    id?: string | null;
    targetMetric?: string | null;
    targetDir?: string | null;
  } | null;
}

export function codingAgentDirective(args: {
  projectName: string;
  projectRoot?: string | null;
  action: DirectiveAction;
}): string {
  const { projectName, projectRoot, action } = args;
  const id = action.identity?.id?.trim() || null;
  const metric =
    action.identity?.targetMetric ?? action.targetMetric ?? null;
  const dir = action.identity?.targetDir ?? action.targetDir ?? null;
  const kind = (action.artifactKind || 'none').trim().toLowerCase();
  const artifact = action.artifact?.trim() || '';
  const category = action.category?.trim() || 'growth';

  const lines: string[] = [];
  lines.push(
    `You are a coding agent working in the repository for "${projectName}".`,
  );
  if (projectRoot) {
    lines.push(`Working directory: ${projectRoot}`);
  }
  lines.push(
    'Implement the growth action below in this repo. Do not stop at drafting copy in chat — write the change into the right files.',
  );
  lines.push('');
  lines.push('## Task');
  lines.push(action.title.trim() || 'Untitled growth action');
  lines.push(`Category: ${category}`);
  lines.push('');
  if (action.evidence?.trim()) {
    lines.push('## Why (from this project\'s analytics)');
    lines.push(action.evidence.trim());
    lines.push('');
  }
  lines.push('## What to do');
  lines.push(action.recommendation.trim() || action.title.trim());
  lines.push('');
  const steps = (action.steps ?? []).map((s) => s.trim()).filter(Boolean);
  if (steps.length > 0) {
    lines.push('## Steps');
    steps.forEach((step, i) => lines.push(`${i + 1}. ${step}`));
    lines.push('');
  }
  lines.push('## Deliverable');
  lines.push(deliverableInstructions(kind, artifact));
  if (artifact) {
    lines.push('');
    lines.push('-----');
    lines.push(artifact);
    lines.push('-----');
  }
  lines.push('');
  if (metric && dir) {
    lines.push('## Measurement');
    lines.push(
      `After this ships, the change is measured on ${metric} going ${dir}. Do not retarget a different metric, and do not mix in unrelated analytics work.`,
    );
    lines.push('');
  }
  lines.push('## When finished');
  lines.push(
    'Reply with a short summary of the files you changed and where the work lives in the repo.',
  );
  if (id) {
    lines.push(`Your last line MUST be exactly: ${GROW_ACTION_DONE_PREFIX} ${id}`);
  }
  return lines.join('\n').trimEnd() + '\n';
}

function deliverableInstructions(kind: string, artifact: string): string {
  if (kind === 'post') {
    return [
      'The block below is copy to publish, not a finished task on its own.',
      'Find the correct place in this repo (blog/content collection, markdown in the CMS, marketing route, or the existing content folder) and add it as a real file or route.',
      'Name the file from the task title, following this project\'s conventions. If no content directory exists, create one that matches neighbouring files and say where you put it.',
      artifact
        ? 'Do not rewrite the copy unless a factual or house-style fix is required.'
        : 'Draft the copy in the product\'s voice, then write it to that path.',
    ].join(' ');
  }
  if (kind === 'prompt' && artifact) {
    return 'Follow the instruction in the block below. Name concrete files, routes, and tags. The dashboard is not available to you, so treat the block as the full brief.';
  }
  if (artifact) {
    return 'Use the block below as the brief for the change.';
  }
  return 'There is no separate artifact. Implement the change described above, in the files this repo already uses for that kind of work.';
}
