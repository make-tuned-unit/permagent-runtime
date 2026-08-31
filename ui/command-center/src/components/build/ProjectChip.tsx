import { useState, useRef, useEffect, type CSSProperties } from 'react';
import { FiChevronDown, FiFolder } from 'react-icons/fi';
import { font, radius, textSize } from '../../styles/tokens';
import { useProjects, Project } from './useProjects';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { Button } from '../common/Button';
import { launchTooltip, SUBSCRIPTION_FIRST_HINT } from '../grow/codingAgents';

const PERSONAL_ID = '00000000-0000-0000-0000-000000000001';

interface Props {
  onLaunch: (project: Project, agent: string) => void;
  onVisitSite: (url: string) => void;
}

export function ProjectChip({ onLaunch, onVisitSite }: Props) {
  const { colors } = useTheme();
  const { projects, loading, error, retry, touch } = useProjects();
  const [open, setOpen] = useState(false);
  const [sortMode, setSortMode] = useState<'recent' | 'az'>('recent');
  const ref = useRef<HTMLDivElement>(null);
  const pushOverlay = useCommandCenter(s => s.pushBrowserOverlay);
  const popOverlay = useCommandCenter(s => s.popBrowserOverlay);

  // Hide native browser webview while dropdown is open (z-index fix)
  useEffect(() => {
    if (open) { pushOverlay(); return () => { popOverlay(); }; }
  }, [open, pushOverlay, popOverlay]);

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

  // This chip is the only way to launch a coding agent against a project, so
  // it is never removed: a daemon failure, an empty list and a load in flight
  // are three different sentences inside it, not one missing control.
  return (
    <div ref={ref} style={{ position: 'relative' }}>
      {/* Chip button */}
      <Button
        colors={colors}
        data-testid="project-chip"
        onClick={() => setOpen(!open)}
        title={error ? "Couldn't load your projects" : undefined}
        style={{
          '--pa-btn-bg': colors.cyanSoft,
          '--pa-btn-fg': colors.textMuted,
          '--pa-btn-border': error ? colors.danger : colors.border,
          '--pa-btn-bg-hover': colors.cyanGlow,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-border-hover': error ? colors.danger : colors.borderHi,
          '--pa-btn-bg-active': colors.cyanSoft,
          '--pa-btn-pad': '0 10px',
          '--pa-btn-radius': `${radius.sm}px`,
          height: 28,
          fontFamily: font.body,
        } as CSSProperties}
      >
        <FiFolder size={10} />
        Projects
        <FiChevronDown size={8} />
      </Button>

      {/* Dropdown */}
      {open && (
        <div style={{
          // Anchored to the chip's LEFT edge so the menu opens rightward
          // across the header, not off toward the browser pane.
          position: 'absolute', top: '100%', left: 0, marginTop: 4,
          minWidth: 280, maxHeight: 360, overflowY: 'auto',
          background: colors.surface, border: `1px solid ${colors.border}`,
          borderRadius: radius.md, boxShadow: colors.cardShadow,
          zIndex: 50, padding: '4px 0',
        }}>
          {loading && (
            <div style={{ padding: '10px 12px', fontSize: textSize.micro, color: colors.textDim, fontFamily: font.body }}>
              Loading your projects…
            </div>
          )}

          {!loading && error && (
            <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-start' }}>
              <div style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.danger, fontFamily: font.body }}>
                Couldn't load your projects
              </div>
              <div style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.body, lineHeight: 1.45 }}>
                The projects service didn't respond. Check that the daemon is running.
              </div>
              <Button colors={colors} type="button" onClick={() => retry()}>Retry</Button>
            </div>
          )}

          {!loading && !error && projects.length === 0 && (
            <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-start' }}>
              <div style={{ fontSize: textSize.caption, color: colors.textMuted, fontFamily: font.body, lineHeight: 1.45 }}>
                No active projects yet — add one in Projects.
              </div>
              <Button
                colors={colors}
                type="button"
                onClick={() => { setOpen(false); navigateToTool('projects'); }}
              >
                Open Projects
              </Button>
            </div>
          )}

          {/* Sort toggle */}
          {!loading && !error && projects.length > 0 && (
          <div style={{
            padding: '6px 10px', display: 'flex', gap: 8,
            borderBottom: `1px solid ${colors.border}`, marginBottom: 2,
          }}>
            {(['recent', 'az'] as const).map(mode => (
              <Button
                key={mode}
                colors={colors}
                variant="bare"
                onClick={() => setSortMode(mode)}
                style={{
                  '--pa-btn-bg': sortMode === mode ? colors.cyanSoft : 'transparent',
                  '--pa-btn-fg': sortMode === mode ? colors.text : colors.textDim,
                  '--pa-btn-bg-hover': sortMode === mode ? colors.cyanSoft : colors.surfaceHi,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-bg-active': sortMode === mode ? colors.cyanSoft : colors.surfaceHi,
                  '--pa-btn-pad': '2px 6px',
                  '--pa-btn-radius': `${radius.xs}px`,
                  fontFamily: font.body,
                  fontSize: 10,
                } as CSSProperties}
              >
                {mode === 'recent' ? 'Recent' : 'A-Z'}
              </Button>
            ))}
          </div>
          )}

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

function ProjectRow({
project, onLaunch, onVisit }: {
  project: Project;
  onLaunch: (p: Project, agent: string) => void;
  onVisit: (p: Project) => void;
}) {
  const { colors } = useTheme();
  const [expanded, setExpanded] = useState(false);

  return (
    <div style={{ padding: '0 4px' }}>
      {/* A disclosure toggle, not an action: it opens the agent row below it and
          there is nothing to await, so the pending floor and the success tick
          would both be wrong for it. It takes the shared `.pa-btn` interaction
          rules directly instead — same treatment as FinanceView's PickRow — so
          its name and chevron stay its own flex children. */}
      <button
        type="button"
        className="pa-btn"
        aria-expanded={expanded}
        aria-controls={`project-agents-${project.id}`}
        onClick={() => setExpanded(!expanded)}
        style={{
          '--pa-btn-bg': expanded ? colors.cyanSoft : 'transparent',
          '--pa-btn-fg': colors.text,
          '--pa-btn-bg-hover': expanded ? colors.cyanSoft : colors.surfaceHi,
          '--pa-btn-bg-active': expanded ? colors.cyanSoft : colors.surface,
          '--pa-btn-pad': '7px 8px',
          '--pa-btn-radius': `${radius.sm}px`,
          display: 'flex', width: '100%', textAlign: 'left',
          justifyContent: 'flex-start', gap: 8,
          fontFamily: font.body, fontSize: textSize.caption,
        } as CSSProperties}
      >
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {project.name}
        </span>
        <FiChevronDown
          size={8} color={colors.textDim}
          style={{ transform: expanded ? 'rotate(180deg)' : 'none', transition: 'transform 0.15s' }}
        />
      </button>

      {expanded && (
        <div id={`project-agents-${project.id}`} style={{ padding: '4px 8px 8px 8px', display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          <div
            data-testid="subscription-first-hint"
            style={{
              width: '100%',
              fontSize: 10,
              fontFamily: font.body,
              color: colors.textDim,
              lineHeight: 1.35,
              marginBottom: 2,
            }}
          >
            {SUBSCRIPTION_FIRST_HINT}
          </div>
          <ActionBtn
            label="Claude"
            disabled={!project.rootPath}
            tooltip={launchTooltip('claude', !!project.rootPath)}
            onClick={() => onLaunch(project, 'claude')}
          />
          <ActionBtn
            label="Codex"
            disabled={!project.rootPath}
            tooltip={launchTooltip('codex', !!project.rootPath)}
            onClick={() => onLaunch(project, 'codex')}
          />
          <ActionBtn
            label="Cursor"
            disabled={!project.rootPath}
            tooltip={launchTooltip('cursor-agent', !!project.rootPath)}
            // `cursor-agent`, not the `agent` symlink its installer also drops
            // in ~/.local/bin: "agent" is generic enough to collide with an
            // unrelated binary already on PATH.
            onClick={() => onLaunch(project, 'cursor-agent')}
          />
          <ActionBtn
            label="Permagent"
            disabled={!project.rootPath}
            tooltip={launchTooltip(
              'permagent run --recipe permagent-coding --interactive',
              !!project.rootPath,
            )}
            onClick={() => onLaunch(project, 'permagent run --recipe permagent-coding --interactive')}
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

function ActionBtn({
label, disabled, tooltip, onClick }: {
  label: string; disabled?: boolean; tooltip?: string; onClick: () => void;
}) {
  const { colors } = useTheme();
  // `disabled` here has always been a look plus a dropped handler, never the
  // DOM attribute — the `title` explaining WHY it can't be pressed only shows
  // on a pointer-eventful element. Left exactly as it was; the button now
  // simply holds its resting look on hover when it is in that state.
  return (
    <Button
      colors={colors}
      onClick={disabled ? undefined : onClick}
      title={tooltip}
      style={{
        '--pa-btn-bg': disabled ? colors.border : colors.cyanSoft,
        '--pa-btn-fg': disabled ? colors.textDim : colors.cyan,
        '--pa-btn-border': disabled ? colors.border : colors.borderHi,
        '--pa-btn-bg-hover': disabled ? colors.border : colors.cyanGlow,
        '--pa-btn-border-hover': disabled ? colors.border : colors.cyan,
        '--pa-btn-bg-active': disabled ? colors.border : colors.cyanSoft,
        '--pa-btn-pad': '0 8px',
        '--pa-btn-radius': '5px',
        height: 24,
        fontFamily: font.body,
        fontSize: 10,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.5 : 1,
      } as CSSProperties}
    >
      {label}
    </Button>
  );
}
