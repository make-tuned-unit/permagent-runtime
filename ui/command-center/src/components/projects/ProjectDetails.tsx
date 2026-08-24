import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { ActivityPanel } from './ActivityPanel';
import { CodeIndexPanel } from './CodeIndexPanel';
import { DocumentsPanel } from './DocumentsPanel';
import { EcosystemPanel } from './EcosystemPanel';
import { LinksPanel } from './ProjectOverview';
import { MemoriesPanel } from './MemoriesPanel';
import { NotesPanel } from './NotesPanel';
import { PeoplePanel } from './PeoplePanel';
import { StackPanel } from './StackPanel';
import { VerificationApprovalPanel } from './VerificationApprovalPanel';
import type { Project } from './types';

export { ActivityPanel };

/**
 * ProjectDetails — the project's FULL RECORDS lens (#73).
 *
 * The division of labour with ProjectOverview (ruled 2026-07-26, after the two
 * lenses had drifted into showing the same five panels):
 *
 *   Overview → at a glance. What is this, where does it stand, what's next.
 *   Details  → the records themselves. Everything you read, edit, add or
 *              remove: stack entries, documents, notes, memories, people,
 *              ecosystem, resource links, indexed code.
 *
 * No panel appears in both. If you are adding a panel, ask whether the user
 * comes to it to GLANCE (Overview) or to WORK ON IT (Details).
 */
export function ProjectDetails({ project, onProjectUpdated }: {
  project: Project;
  onProjectUpdated?: () => void;
}) {
  const { colors, gradient } = useTheme();
  return (
    <div style={{ flex: 1, overflow: 'auto', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1.2fr) minmax(280px, .8fr)', gap: 16, padding: '20px 24px', alignItems: 'start' }}>
        {/* LEFT — the project's own material */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          {/* Stack organizer (#512): services + login identity, reference-only. */}
          <StackPanel project={project} />
          {/* Verification approval ladder: the allowlist + earned privilege
              governing Tier-1 auto-approval of model-authored check commands
              — editable ONLY here (one concept, one place). */}
          <VerificationApprovalPanel project={project} />
          <DocumentsPanel project={project} />
          <NotesPanel project={project} />
          <MemoriesPanel project={project} />
          <CodeIndexPanel project={project} />
        </div>

        {/* RIGHT — who and what it connects to */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          <PeoplePanel project={project} />
          <EcosystemPanel project={project} />
          <LinksPanel project={project} onProjectUpdated={onProjectUpdated} title="Resources" />
        </div>
      </div>
    </div>
  );
}
