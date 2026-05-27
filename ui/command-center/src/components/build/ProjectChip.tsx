import { useState, useRef, useEffect } from 'react';
import { color, font } from '../../styles/tokens';
import { useProjects, Project } from './useProjects';

const PERSONAL_ID = '00000000-0000-0000-0000-000000000001';

interface Props {
  onLaunch: (project: Project, agent: string) => void;
  onVisitSite: (url: string) => void;
}

export function ProjectChip({ onLaunch, onVisitSite }: Props) {
  const { projects, loading, touch } = useProjects();
  const [open, setOpen] = useState(false);
  const [sortMode, setSortMode] = useState<'recent' | 'az'>('recent');
  const ref = useRef<HTMLDivElement>(null);

  // Close dropdown on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const sorted = [...projects].sort((a, b) => {
    // Personal always first
    if (a.id === PERSONAL_ID) return -1;
    if (b.id === PERSONAL_ID) return 1;
    if (sortMode === 'az') return a.name.localeCompare(b.name);
    return new Date(b.lastOpenedAt).getTime() - new Date(a.lastOpenedAt).getTime();
  });

  const handleLaunch = (project: Project, agent: string) => {
    touch(project.id);
    onLaunch(project, agent);
    setOpen(false);
  };

  const handleVisit = (project: Project) => {
    if (!project.siteUrl) return;
    touch(project.id);
    onVisitSite(project.siteUrl);
    setOpen(false);
  };

  if (loading || projects.length === 0) return null;

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      {/* Chip button */}
      <button
        onClick={() => setOpen(!open)}
        style={{
          height: 28, padding: '0 10px', borderRadius: 6,
          background: 'rgba(255,255,255,0.06)', border: `1px solid ${color.border}`,
          fontFamily: font.body, fontSize: 11, fontWeight: 500,
          color: color.textMuted, cursor: 'pointer',
          display: 'inline-flex', alignItems: 'center', gap: 5,
        }}
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
          <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
        </svg>
        Projects
        <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5}>
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {/* Dropdown */}
      {open && (
        <div style={{
          position: 'absolute', top: '100%', left: 0, marginTop: 4,
          minWidth: 280, maxHeight: 360, overflowY: 'auto',
          background: '#0F1729', border: `1px solid ${color.border}`,
          borderRadius: 8, boxShadow: '0 8px 32px rgba(0,0,0,0.5)',
          zIndex: 50, padding: '4px 0',
        }}>
          {/* Sort toggle */}
          <div style={{
            padding: '6px 10px', display: 'flex', gap: 8,
            borderBottom: `1px solid ${color.border}`, marginBottom: 2,
          }}>
            {(['recent', 'az'] as const).map(mode => (
              <button
                key={mode}
                onClick={() => setSortMode(mode)}
                style={{
                  fontSize: 10, fontFamily: font.body, fontWeight: 500,
                  padding: '2px 6px', borderRadius: 4, cursor: 'pointer',
                  background: sortMode === mode ? 'rgba(255,255,255,0.1)' : 'transparent',
                  color: sortMode === mode ? color.text : color.textDim,
                  border: 'none',
                }}
              >
                {mode === 'recent' ? 'Recent' : 'A-Z'}
              </button>
            ))}
          </div>

          {sorted.map(project => (
            <ProjectRow
              key={project.id}
              project={project}
              onLaunch={handleLaunch}
              onVisit={handleVisit}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ProjectRow({ project, onLaunch, onVisit }: {
  project: Project;
  onLaunch: (p: Project, agent: string) => void;
  onVisit: (p: Project) => void;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div style={{ padding: '0 4px' }}>
      <button
        onClick={() => setExpanded(!expanded)}
        style={{
          width: '100%', padding: '7px 8px', borderRadius: 6,
          background: expanded ? 'rgba(255,255,255,0.05)' : 'transparent',
          border: 'none', cursor: 'pointer', textAlign: 'left',
          display: 'flex', alignItems: 'center', gap: 8,
          fontFamily: font.body, fontSize: 12, color: color.text,
        }}
      >
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {project.name}
        </span>
        <svg
          width="8" height="8" viewBox="0 0 24 24" fill="none"
          stroke={color.textDim} strokeWidth={2.5}
          style={{ transform: expanded ? 'rotate(180deg)' : 'none', transition: 'transform 0.15s' }}
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {expanded && (
        <div style={{ padding: '4px 8px 8px 8px', display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          <ActionBtn
            label="Claude"
            disabled={!project.rootPath}
            tooltip={!project.rootPath ? 'Add a root path to launch a terminal here.' : undefined}
            onClick={() => onLaunch(project, 'claude')}
          />
          <ActionBtn
            label="Codex"
            disabled={!project.rootPath}
            tooltip={!project.rootPath ? 'Add a root path to launch a terminal here.' : undefined}
            onClick={() => onLaunch(project, 'codex')}
          />
          <ActionBtn
            label="Visit Site"
            disabled={!project.siteUrl}
            tooltip={!project.siteUrl ? "Add a site URL to open the project's site." : undefined}
            onClick={() => onVisit(project)}
          />
        </div>
      )}
    </div>
  );
}

function ActionBtn({ label, disabled, tooltip, onClick }: {
  label: string; disabled?: boolean; tooltip?: string; onClick: () => void;
}) {
  return (
    <button
      onClick={disabled ? undefined : onClick}
      title={tooltip}
      style={{
        height: 24, padding: '0 8px', borderRadius: 5,
        background: disabled ? 'rgba(255,255,255,0.03)' : 'rgba(0,217,255,0.1)',
        border: `1px solid ${disabled ? 'rgba(255,255,255,0.06)' : 'rgba(0,217,255,0.25)'}`,
        fontFamily: font.body, fontSize: 10, fontWeight: 500,
        color: disabled ? color.textDim : color.cyan,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.5 : 1,
      }}
    >
      {label}
    </button>
  );
}
