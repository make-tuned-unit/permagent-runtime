// ActivityPanel — the project's recent activity feed.
//
// Lives in its own file because BOTH lenses need it without importing each
// other: Overview renders it as its at-a-glance "what happened lately", and it
// used to live inside ProjectDetails (which already imports LinksPanel from
// ProjectOverview — putting it back there would close an import cycle).
//
// Uses only mounted, project-scoped endpoints. The daemon currently has no
// project-specific commit/deploy/conversation read, so those are deliberately
// not synthesized here.

import { useCallback, useEffect, useMemo, useState, type CSSProperties } from 'react';
import { FiActivity, FiBookOpen, FiCheckSquare, FiFileText, FiRefreshCw } from 'react-icons/fi';
import { api, apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import type { Card, Project, ProjectMemory, ProjectNote } from './types';

type ActivityItem = {
  id: string;
  kind: 'memory' | 'note' | 'task';
  title: string;
  detail: string;
  timestamp: string;
};

export function ActivityPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const rowVeil = colors.fillSubtle;
  // `fillSubtle` IS this idiom, tokenised (#1162). The hand-written pair it
  // replaces predates the token and was a shade off on both themes; the token
  // carries the THEME's own ink, so one name reads as a lift on the void and
  // as a shade on the pearl without a conditional here.
  const projectsRev = useCommandCenter(s => s.projectsRev);
  const [memories, setMemories] = useState<ProjectMemory[]>([]);
  const [notes, setNotes] = useState<ProjectNote[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');

  // Resolves `false` on failure so the refresh button's success tick can never
  // fire over a load that actually failed (Button's contract).
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
      return true;
    } catch {
      setStatus('error');
      return false;
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
      <Button
        colors={colors}
        variant="bare"
        onClick={load}
        aria-label="Refresh activity"
        title="Refresh"
        style={{
          '--pa-btn-fg': colors.textDim,
          '--pa-btn-fg-hover': colors.textMuted,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '0',
        } as CSSProperties}
      >
        <FiRefreshCw size={12} />
      </Button>
    ) : undefined}>
      {status === 'loading' && <div style={{ fontSize: textSize.micro, color: colors.textDim }}>Loading activity…</div>}
      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: textSize.micro, color: colors.danger }}>Couldn't load activity.</span>
          <Button
            colors={colors}
            variant="bare"
            className="hover:underline"
            onClick={load}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 600,
              fontFamily: font.body,
              fontSize: textSize.micro,
            } as CSSProperties}
          >
            Retry
          </Button>
        </div>
      )}
      {status === 'ready' && items.length === 0 && (
        <div style={{ display: 'flex', gap: 8, alignItems: 'center', fontSize: textSize.micro, color: colors.textDim }}>
          <FiActivity size={13} /> No project activity yet.
        </div>
      )}
      {status === 'ready' && items.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {items.map(item => (
            <div key={item.id} style={{ display: 'flex', gap: 9, padding: '8px 10px', borderRadius: radius.md, background: rowVeil, border: `1px solid ${colors.border}` }}>
              <span style={{ color: colors.cyan, marginTop: 2, flexShrink: 0 }}>{icon(item.kind)}</span>
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{ fontSize: textSize.caption, color: colors.text, fontWeight: 600 }}>{item.title}</div>
                <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{item.detail}</div>
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
