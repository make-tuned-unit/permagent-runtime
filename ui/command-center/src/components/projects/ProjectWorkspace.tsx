import { useState, useRef, useEffect } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { ProjectKanban } from './ProjectsView';
import { ProjectOverview } from './ProjectOverview';
import type { Project, ProjectLens } from './types';

// ── Project Workspace ───────────────────────────────────────────────────────
//
// The Projects-tab surface for a single selected project (#471). Two orthogonal
// axes share one state: PROJECT (the switcher) × VIEW (the toggle). Switching
// the view keeps the same project; switching the project re-renders the active
// view. The chrome (back · switcher · toggle) is persistent above both lenses,
// so neither lens carries its own navigation and they can never desync.

export function ProjectWorkspace({ project, projects, onSwitchProject, onBack }: {
  project: Project;
  projects: Project[];
  onSwitchProject: (id: string) => void;
  onBack: () => void;
}) {
  const { colors, gradient } = useTheme();
  // VIEW axis — persists across project switches (single source, no reset).
  const [lens, setLens] = useState<ProjectLens>('overview');

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
      background: gradient.workspace, color: colors.text, fontFamily: font.body,
    }}>
      {/* Shared chrome */}
      <div style={{
        padding: '10px 24px', borderBottom: `1px solid ${colors.border}`, flexShrink: 0,
        display: 'flex', alignItems: 'center', gap: 12,
      }}>
        <button onClick={onBack} style={backBtn(colors)}
          onMouseEnter={e => { (e.currentTarget as HTMLElement).style.color = colors.text; }}
          onMouseLeave={e => { (e.currentTarget as HTMLElement).style.color = colors.textMuted; }}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M19 12H5M12 19l-7-7 7-7" />
          </svg>
          All Projects
        </button>
        <div style={{ width: 1, height: 16, background: colors.border }} />

        {/* PROJECT axis — switcher */}
        <ProjectSwitcher project={project} projects={projects} onSwitch={onSwitchProject} />

        <div style={{ flex: 1 }} />

        {/* VIEW axis — toggle */}
        <ViewToggle lens={lens} onChange={setLens} />
      </div>

      {/* Active lens — both render the SAME selected project */}
      {lens === 'overview'
        ? <ProjectOverview project={project} />
        : <ProjectKanban project={project} />}
    </div>
  );
}

// ── Project switcher ────────────────────────────────────────────────────────

function ProjectSwitcher({ project, projects, onSwitch }: {
  project: Project;
  projects: Project[];
  onSwitch: (id: string) => void;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);

  const sorted = [...projects].sort((a, b) => a.name.localeCompare(b.name));

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <button
        onClick={() => setOpen(o => !o)}
        style={{
          display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer',
          background: 'none', border: 'none', padding: '2px 4px', color: colors.text,
          fontFamily: font.display, fontSize: 15, fontWeight: 600, letterSpacing: '-0.01em',
        }}
      >
        {project.name}
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke={colors.textMuted} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round"
          style={{ transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 150ms' }}>
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>
      <div style={{ fontSize: 10, color: colors.textMuted, marginTop: -2 }}>{project.slug}</div>

      {open && (
        <div style={{
          position: 'absolute', top: '100%', left: 0, marginTop: 6, zIndex: 50,
          minWidth: 200, maxHeight: 320, overflow: 'auto',
          background: '#0F1729', border: `1px solid ${colors.border}`, borderRadius: 8,
          boxShadow: '0 8px 24px rgba(0,0,0,0.4)', padding: 4,
        }}>
          {sorted.map(p => (
            <button
              key={p.id}
              onClick={() => { setOpen(false); if (p.id !== project.id) onSwitch(p.id); }}
              style={{
                display: 'block', width: '100%', textAlign: 'left',
                padding: '6px 10px', borderRadius: 5, cursor: 'pointer',
                background: p.id === project.id ? 'rgba(0,213,255,0.1)' : 'transparent',
                border: 'none', color: p.id === project.id ? colors.cyan : colors.text,
                fontFamily: font.body, fontSize: 12, fontWeight: p.id === project.id ? 600 : 400,
              }}
              onMouseEnter={e => { if (p.id !== project.id) (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.05)'; }}
              onMouseLeave={e => { if (p.id !== project.id) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
            >
              {p.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ── View toggle ─────────────────────────────────────────────────────────────

function ViewToggle({ lens, onChange }: { lens: ProjectLens; onChange: (l: ProjectLens) => void }) {
  const { colors } = useTheme();
  const tabs: { key: ProjectLens; label: string }[] = [
    { key: 'overview', label: 'Overview' },
    { key: 'kanban', label: 'Kanban' },
  ];
  return (
    <div style={{
      display: 'flex', gap: 2, padding: 2, borderRadius: 8,
      background: 'rgba(255,255,255,0.04)', border: `1px solid ${colors.border}`,
    }}>
      {tabs.map(t => {
        const active = lens === t.key;
        return (
          <button
            key={t.key}
            onClick={() => onChange(t.key)}
            style={{
              padding: '4px 12px', borderRadius: 6, cursor: 'pointer', border: 'none',
              background: active ? 'rgba(0,213,255,0.15)' : 'transparent',
              color: active ? colors.cyan : colors.textMuted,
              fontFamily: font.body, fontSize: 12, fontWeight: active ? 600 : 500,
              transition: 'all 150ms',
            }}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

function backBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    background: 'none', border: 'none', color: colors.textMuted, cursor: 'pointer',
    padding: '4px 8px', borderRadius: 6, fontSize: 12, fontFamily: font.body,
    display: 'flex', alignItems: 'center', gap: 4,
  };
}
