import { useEffect, useId, useState, type ReactNode } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import { Button } from '../common/Button';
import { CodeIndexPanel } from './CodeIndexPanel';
import { DocumentsPanel } from './DocumentsPanel';
import { EcosystemPanel } from './EcosystemPanel';
import { MarketPanel } from './MarketPanel';
import { KeyFactsPanel, LinksPanel } from './ProjectOverview';
import { MemoriesPanel } from './MemoriesPanel';
import { NotesPanel } from './NotesPanel';
import { PeoplePanel } from './PeoplePanel';
import { StackPanel } from './StackPanel';
import { VerificationApprovalPanel } from './VerificationApprovalPanel';
import { PublishSequencePanel } from './PublishSequencePanel';
import type { Project } from './types';
import './projectWorkspace.css';

export { ActivityPanel } from './ActivityPanel';

/** Load collections on first visit and retain unsaved edits across navigation. */
function Collections({ label, selected, onSelect, children }: {
  label: string;
  selected: string;
  onSelect: (id: string) => void;
  children: { id: string; label: string; content: ReactNode }[];
}) {
  const { colors } = useTheme();
  const prefix = useId();
  const [visited, setVisited] = useState(() => new Set([selected]));
  useEffect(() => { setVisited(prev => new Set([...prev, selected])); }, [selected]);
  return <>
    <nav aria-label={label} className="project-section-nav">
      {children.map(item => <Button key={item.id} colors={colors} variant={selected === item.id ? 'ghostOn' : 'ghost'} aria-pressed={selected === item.id}
        id={`${prefix}-${item.id}-button`} aria-controls={`${prefix}-${item.id}`} onClick={() => onSelect(item.id)}>
        {item.label}
      </Button>)}
    </nav>
    {children.map(item =>
      <section key={item.id} id={`${prefix}-${item.id}`} aria-labelledby={`${prefix}-${item.id}-button`} hidden={selected !== item.id}>
        <div className="project-section-content">{(visited.has(item.id) || selected === item.id) && item.content}</div>
      </section>)}
  </>;
}

export function ProjectDetails(props: { project: Project; onProjectUpdated?: () => void }) {
  return <ProjectRecords key={props.project.id} {...props} />;
}

function ProjectRecords({ project, onProjectUpdated }: { project: Project; onProjectUpdated?: () => void }) {
  const { colors, gradient } = useTheme();
  const pendingNote = useCommandCenter(s => s.pendingNoteNavigation);
  const [section, setSection] = useState(pendingNote?.projectId === project.id ? 'knowledge' : 'stack');
  const [collection, setCollection] = useState(pendingNote?.projectId === project.id ? 'notes' : 'documents');
  useEffect(() => {
    if (pendingNote?.projectId === project.id) { setSection('knowledge'); setCollection('notes'); }
  }, [pendingNote, project.id]);
  return (
    <div className="project-workspace-page" style={{ flex: 1, overflow: 'auto', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      <div className="project-workspace-content">
        <Collections label="Project details sections" selected={section} onSelect={setSection}>{[
          { id: 'stack', label: 'Stack & connections', content: <><StackPanel project={project} /><LinksPanel project={project} onProjectUpdated={onProjectUpdated} title="Resources" /></> },
          { id: 'knowledge', label: 'Knowledge', content:
            <Collections label="Project knowledge collections" selected={collection} onSelect={setCollection}>{[
              { id: 'documents', label: 'Documents', content: <DocumentsPanel project={project} /> },
              { id: 'notes', label: 'Notes', content: <NotesPanel project={project} /> },
              { id: 'memories', label: 'Memories', content: <MemoriesPanel project={project} /> },
              { id: 'code', label: 'Code', content: <CodeIndexPanel project={project} /> },
            ]}</Collections> },
          { id: 'relationships', label: 'People & market', content: <><PeoplePanel project={project} /><EcosystemPanel project={project} /><MarketPanel project={project} /></> },
          { id: 'settings', label: 'Project settings', content: <><KeyFactsPanel project={project} /><VerificationApprovalPanel project={project} /><PublishSequencePanel project={project} /></> },
        ]}</Collections>
      </div>
    </div>
  );
}
