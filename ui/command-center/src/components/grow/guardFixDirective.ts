/**
 * Turn a Guard finding into a prompt a coding agent can run. Same idea as
 * `codingAgentDirective` for Grow: the agent gets what, where, how to fix it,
 * and the rules that keep the fix honest. The commit-message tag is how the
 * Guard later finds the fix (`Guard: <finding id>`).
 */

export interface GuardDirectiveFinding {
  id: string;
  title: string;
  severity: string;
  cwe?: string | null;
  location?: string | null;
  remediation?: string | null;
}

export function guardCommitTag(findingId: string): string {
  return `Guard: ${findingId}`;
}

export function guardFixDirective(args: {
  projectName: string;
  projectRoot?: string | null;
  projectSlug?: string | null;
  finding: GuardDirectiveFinding;
}): string {
  const { projectName, projectRoot, projectSlug, finding } = args;
  const lines: string[] = [];
  lines.push(`You are a coding agent working in the repository for "${projectName}".`);
  if (projectRoot) lines.push(`Working directory: ${projectRoot}`);
  if (projectSlug?.trim()) {
    lines.push(
      `Permagent project: ${projectSlug.trim()} (use this, not the path, wherever a Permagent tool asks for the project).`,
    );
  }
  lines.push('Fix the one security finding below in this repo. Change the code; do not stop at explaining it.');
  lines.push('');
  lines.push('## Finding');
  lines.push(`Title: ${finding.title.trim()}`);
  lines.push(`Severity: ${finding.severity}`);
  if (finding.cwe?.trim()) lines.push(`Weakness: ${finding.cwe.trim()}`);
  if (finding.location?.trim()) lines.push(`Location: ${finding.location.trim()}`);
  lines.push(`Finding id: ${finding.id}`);
  lines.push('');
  if (finding.remediation?.trim()) {
    lines.push('## Suggested remediation');
    lines.push(finding.remediation.trim());
    lines.push('');
  }
  lines.push('## Rules');
  lines.push('- Fix only this finding. Do not refactor or change unrelated code.');
  lines.push('- Never disable, skip or weaken a check, a test or a scanner rule to make the finding go away.');
  lines.push('- Add or adjust a test that covers the fix where practical.');
  lines.push(`- Commit the fix with a message that contains exactly: ${guardCommitTag(finding.id)}`);
  lines.push('');
  lines.push('## When finished');
  lines.push('Reply with a short summary of what you changed and the commit you made.');
  return lines.join('\n').trimEnd() + '\n';
}
