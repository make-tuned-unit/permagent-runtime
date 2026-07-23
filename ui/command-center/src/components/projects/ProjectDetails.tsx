import { useCallback, useEffect, useMemo, useState } from 'react';
import { FiActivity, FiBookOpen, FiCheckSquare, FiFileText, FiRefreshCw } from 'react-icons/fi';
import { api, apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { DocumentsPanel } from './DocumentsPanel';
import { LinksPanel } from './ProjectOverview';
import { NotesPanel } from './NotesPanel';
import { Panel } from './Panel';
import { PeoplePanel } from './PeoplePanel';
import { EcosystemPanel } from './EcosystemPanel';
import type { Card, Project, ProjectMemory, ProjectNote } from './types';

/** The focused, operational details lens requested by #73. */
export function ProjectDetails({ project, onProjectUpdated }: {
  project: Project;
  onProjectUpdated?: () => void;
}) {
  const { colors, gradient } = useTheme();
  return (
    <div style={{ flex: 1, overflow: 'auto', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1.2fr) minmax(280px, .8fr)', gap: 16, padding: '20px 24px', alignItems: 'start' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          <LinksPanel project={project} onProjectUpdated={onProjectUpdated} title="Resources" />
          <DocumentsPanel project={project} />
          <ActivityPanel project={project} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          <PeoplePanel project={project} />
          <EcosystemPanel project={project} />
          <NotesPanel project={project} />
        </div>
      </div>
    </div>
  );
}

type ActivityItem = {
  id: string;
  kind: 'memory' | 'note' | 'task';
  title: string;
  detail: string;
  timestamp: string;
};

/**
 * Uses only mounted, project-scoped endpoints. The daemon currently has no
 * project-specific commit/deploy/conversation read, so those are deliberately
 * not synthesized here.
 */
export function ActivityPanel({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const projectsRev = useCommandCenter(s => s.projectsRev);
  const [memories, setMemories] = useState<ProjectMemory[]>([]);
  const [notes, setNotes] = useState<ProjectNote[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');

  const load = useCallback(async () => {
    setStatus('loading');
    try {
      const [memoryRows, noteRows, cardRows] = await Promise.all([
        api.listProjectMemories(project.id),
        api.listProjectNotes(project.id),
        apiFetch<Card[]>(`/api/projects/${encodeURIComponent(project.id)}/cards`),
      ]);
      setMemories(memoryRows);
      setNotes(noteRows);
      setCards(cardRows);
      setStatus('ready');
    } catch {
      setStatus('error');
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load, projectsRev]);

  const items = useMemo<ActivityItem[]>(() => [
    ...memories.map(m => ({ id: `memory-${m.id}`, kind: 'memory' as const, title: m.description || 'Memory linked', detail: m.content, timestamp: m.associated_at })),
    ...notes.map(n => ({ id: `note-${n.id}`, kind: 'note' as const, title: n.title || 'Note added', detail: n.body, timestamp: n.created_at })),
    ...cards.map(c => ({ id: `task-${c.id}`, kind: 'task' as const, title: c.title, detail: c.archivedAt ? 'Task archived' : 'Task updated', timestamp: c.updatedAt })),
  ].sort((a, b) => Date.parse(b.timestamp) - Date.parse(a.timestamp)).slice(0, 12), [memories, notes, cards]);

  const icon = (kind: ActivityItem['kind']) => kind === 'memory'
    ? <FiBookOpen size={13} />
    : kind === 'note' ? <FiFileText size={13} /> : <FiCheckSquare size={13} />;

  return (
    <Panel title="Recent activity" action={status === 'ready' ? (
      <button onClick={load} aria-label="Refresh activity" title="Refresh" style={{ background: 'none', border: 'none', color: colors.textDim, cursor: 'pointer', padding: 0, display: 'flex' }}>
        <FiRefreshCw size={12} />
      </button>
    ) : undefined}>
      {status === 'loading' && <div style={{ fontSize: 11, color: colors.textDim }}>Loading activity…</div>}
      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: 11, color: colors.danger }}>Couldn't load activity.</span>
          <button onClick={load} style={{ fontSize: 11, color: colors.cyan, background: 'none', border: 'none', cursor: 'pointer', padding: 0, fontFamily: font.body, fontWeight: 600 }}>Retry</button>
        </div>
      )}
      {status === 'ready' && items.length === 0 && (
        <div style={{ display: 'flex', gap: 8, alignItems: 'center', fontSize: 11, color: colors.textDim }}>
          <FiActivity size={13} /> No project activity yet.
        </div>
      )}
      {status === 'ready' && items.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {items.map(item => (
            <div key={item.id} style={{ display: 'flex', gap: 9, padding: '8px 10px', borderRadius: 7, background: rowVeil, border: `1px solid ${colors.border}` }}>
              <span style={{ color: colors.cyan, marginTop: 2, flexShrink: 0 }}>{icon(item.kind)}</span>
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{ fontSize: 12, color: colors.text, fontWeight: 600 }}>{item.title}</div>
                <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{item.detail}</div>
                <div style={{ fontSize: 10, color: colors.textDim, marginTop: 3 }}>{formatActivityTime(item.timestamp)}</div>
              </div>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

function formatActivityTime(value: string): string {
  const time = Date.parse(value);
  if (Number.isNaN(time)) return value;
  return new Date(time).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}
