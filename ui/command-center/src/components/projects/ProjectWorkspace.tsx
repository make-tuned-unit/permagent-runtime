import { useState, useRef, useEffect, type CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { ProjectKanban } from './ProjectsView';
import { ProjectOverview } from './ProjectOverview';
import { ProjectDetails } from './ProjectDetails';
import type { Project, ProjectLens } from './types';
import { useCommandCenter } from '../../lib/store';

// ── Project Workspace ───────────────────────────────────────────────────────
//
// The Projects-tab surface for a single selected project (#471). Two orthogonal
// axes share one state: PROJECT (the switcher) × VIEW (the toggle). Switching
// the view keeps the same project; switching the project re-renders the active
// view. The chrome (back · switcher · toggle) is persistent above both lenses,
// so neither lens carries its own navigation and they can never desync.

export function ProjectWorkspace({ project, projects, onSwitchProject, onBack, onProjectUpdated }: {
  project: Project;
  projects: Project[];
  onSwitchProject: (id: string) => void;
  onBack: () => void;
  /** Parent refetch after an Overview summary edit persists (#472). */
  onProjectUpdated?: () => void;
}) {
  const { colors, gradient } = useTheme();
  // VIEW axis — persists across project switches (single source, no reset).
  const [lens, setLens] = useState<ProjectLens>('overview');

  // Deep-link from the dashboard's to-do list: a card lives on the board, so
  // force the Kanban lens when a card navigation lands on ITS project. Guarded
  // on the project id because the pending navigation and the project selection
  // settle in separate renders — without the guard, arriving here would yank
  // whatever project happened to be open over to Kanban.
  const pendingCard = useCommandCenter(s => s.pendingCardNavigation);
  useEffect(() => {
    if (pendingCard && pendingCard.projectId === project.id) setLens('kanban');
  }, [pendingCard, project.id]);

  // Same contract for the "note saved" deep link: notes render on the Details
  // lens only, so force it when the pending note belongs to THIS project.
  const pendingNote = useCommandCenter(s => s.pendingNoteNavigation);
  useEffect(() => {
    if (pendingNote && pendingNote.projectId === project.id) setLens('details');
  }, [pendingNote, project.id]);

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
        <Button
          colors={colors}
          variant="bare"
          onClick={onBack}
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-bg-active': 'transparent',
            '--pa-btn-pad': '4px 8px',
            '--pa-btn-radius': `${radius.sm}px`,
            fontSize: textSize.caption,
            fontFamily: font.body,
            gap: 4,
          } as CSSProperties}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M19 12H5M12 19l-7-7 7-7" />
          </svg>
          All Projects
        </Button>
        <div style={{ width: 1, height: 16, background: colors.border }} />

        {/* PROJECT axis — switcher */}
        <ProjectSwitcher project={project} projects={projects} onSwitch={onSwitchProject} />

        <div style={{ flex: 1 }} />

        {/* VIEW axis — toggle */}
        <ViewToggle lens={lens} onChange={setLens} />
      </div>

      {/* Active lens — both render the SAME selected project */}
      {lens === 'overview' && <ProjectOverview project={project} onProjectUpdated={onProjectUpdated} />}
      {lens === 'details' && <ProjectDetails project={project} onProjectUpdated={onProjectUpdated} />}
      {lens === 'kanban' && <ProjectKanban project={project} />}
    </div>
  );
}

// ── Project switcher ────────────────────────────────────────────────────────

function ProjectSwitcher({ project, projects, onSwitch }: {
  project: Project;
  projects: Project[];
  onSwitch: (id: string) => void;
}) {
  const { colors, gradient } = useTheme();
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
      <Button
        colors={colors}
        variant="bare"
        onClick={() => setOpen(o => !o)}
        style={{
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-bg-active': 'transparent',
          '--pa-btn-pad': '2px 4px',
          '--pa-btn-radius': `${radius.xs}px`,
          '--pa-btn-weight': 600,
          fontFamily: font.display, fontSize: textSize.body, letterSpacing: '-0.01em',
        } as CSSProperties}
      >
        {project.name}
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke={colors.textMuted} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round"
          style={{ marginLeft: 8, verticalAlign: 'middle', transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 150ms' }}>
          <path d="M6 9l6 6 6-6" />
        </svg>
      </Button>
      <div style={{ fontSize: 10, color: colors.textMuted, marginTop: -2 }}>{project.slug}</div>

      {open && (
        <div style={{
          position: 'absolute', top: '100%', left: 0, marginTop: 6, zIndex: 50,
          minWidth: 200, maxHeight: 320, overflow: 'auto',
          background: gradient.dropdown, border: `1px solid ${colors.border}`, borderRadius: radius.md,
          boxShadow: colors.elevationOverlay, padding: 4,
        }}>
          {sorted.map(p => {
            const isCurrent = p.id === project.id;
            return (
              <Button
                key={p.id}
                colors={colors}
                variant="bare"
                onClick={() => { setOpen(false); if (!isCurrent) onSwitch(p.id); }}
                style={{
                  '--pa-btn-bg': isCurrent ? colors.cyanSoft : 'transparent',
                  '--pa-btn-fg': isCurrent ? colors.cyan : colors.text,
                  // The row the user is already on stayed inert under the mouse
                  // before, and still does — hover is a "you can go here" signal.
                  '--pa-btn-bg-hover': isCurrent ? colors.cyanSoft : 'rgba(255,255,255,0.05)',
                  '--pa-btn-bg-active': isCurrent ? colors.cyanSoft : 'rgba(255,255,255,0.09)',
                  '--pa-btn-pad': '6px 10px',
                  '--pa-btn-radius': '5px',
                  '--pa-btn-weight': isCurrent ? 600 : 400,
                  width: '100%',
                  justifyContent: 'flex-start',
                  textAlign: 'left',
                  fontFamily: font.body,
                  fontSize: textSize.caption,
                } as CSSProperties}
              >
                {p.name}
              </Button>
            );
          })}
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
    { key: 'details', label: 'Details' },
    { key: 'kanban', label: 'Kanban' },
  ];
  return (
    <div style={{
      display: 'flex', gap: 2, padding: 2, borderRadius: radius.md,
      background: 'rgba(255,255,255,0.04)', border: `1px solid ${colors.border}`,
    }}>
      {tabs.map(t => {
        const active = lens === t.key;
        return (
          <Button
            key={t.key}
            colors={colors}
            variant="bare"
            onClick={() => onChange(t.key)}
            style={{
              '--pa-btn-bg': active ? colors.cyanSoft : 'transparent',
              '--pa-btn-fg': active ? colors.cyan : colors.textMuted,
              // The selected tab is already where you are: hover only offers the
              // other two.
              '--pa-btn-bg-hover': active ? colors.cyanSoft : 'rgba(255,255,255,0.05)',
              '--pa-btn-fg-hover': active ? colors.cyan : colors.text,
              '--pa-btn-bg-active': active ? colors.cyanSoft : 'rgba(255,255,255,0.09)',
              '--pa-btn-pad': '4px 12px',
              '--pa-btn-radius': `${radius.sm}px`,
              '--pa-btn-weight': active ? 600 : 500,
              fontFamily: font.body, fontSize: textSize.caption,
            } as CSSProperties}
          >
            {t.label}
          </Button>
        );
      })}
    </div>
  );
}
