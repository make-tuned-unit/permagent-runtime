import { useState, useRef, useEffect } from 'react';
import { font } from '../../styles/tokens';
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
      <button
        data-testid="project-chip"
        onClick={() => setOpen(!open)}
        title={error ? "Couldn't load your projects" : undefined}
        style={{
          height: 28, padding: '0 10px', borderRadius: 6,
          background: colors.cyanSoft,
          border: `1px solid ${error ? colors.danger : colors.border}`,
          fontFamily: font.body, fontSize: 11, fontWeight: 500,
          color: colors.textMuted, cursor: 'pointer',
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
          // Anchored to the chip's LEFT edge so the menu opens rightward
          // across the header, not off toward the browser pane.
          position: 'absolute', top: '100%', left: 0, marginTop: 4,
          minWidth: 280, maxHeight: 360, overflowY: 'auto',
          background: colors.surface, border: `1px solid ${colors.border}`,
          borderRadius: 8, boxShadow: colors.cardShadow,
          zIndex: 50, padding: '4px 0',
        }}>
          {loading && (
            <div style={{ padding: '10px 12px', fontSize: 11, color: colors.textDim, fontFamily: font.body }}>
              Loading your projects…
            </div>
          )}

          {!loading && error && (
            <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-start' }}>
              <div style={{ fontSize: 12, fontWeight: 600, color: colors.danger, fontFamily: font.body }}>
                Couldn't load your projects
              </div>
              <div style={{ fontSize: 11, color: colors.textDim, fontFamily: font.body, lineHeight: 1.45 }}>
                The projects service didn't respond. Check that the daemon is running.
              </div>
              <Button colors={colors} type="button" onClick={() => retry()}>Retry</Button>
            </div>
          )}

          {!loading && !error && projects.length === 0 && (
            <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-start' }}>
              <div style={{ fontSize: 12, color: colors.textMuted, fontFamily: font.body, lineHeight: 1.45 }}>
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
              <button
                key={mode}
                onClick={() => setSortMode(mode)}
                style={{
                  fontSize: 10, fontFamily: font.body, fontWeight: 500,
                  padding: '2px 6px', borderRadius: 4, cursor: 'pointer',
                  background: sortMode === mode ? colors.cyanSoft : 'transparent',
                  color: sortMode === mode ? colors.text : colors.textDim,
                  border: 'none',
                }}
              >
                {mode === 'recent' ? 'Recent' : 'A-Z'}
              </button>
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
      <button
        onClick={() => setExpanded(!expanded)}
        style={{
          width: '100%', padding: '7px 8px', borderRadius: 6,
          background: expanded ? colors.cyanSoft : 'transparent',
          border: 'none', cursor: 'pointer', textAlign: 'left',
          display: 'flex', alignItems: 'center', gap: 8,
          fontFamily: font.body, fontSize: 12, color: colors.text,
        }}
      >
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {project.name}
        </span>
        <svg
          width="8" height="8" viewBox="0 0 24 24" fill="none"
          stroke={colors.textDim} strokeWidth={2.5}
          style={{ transform: expanded ? 'rotate(180deg)' : 'none', transition: 'transform 0.15s' }}
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {expanded && (
        <div style={{ padding: '4px 8px 8px 8px', display: 'flex', gap: 6, flexWrap: 'wrap' }}>
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
  return (
    <button
      onClick={disabled ? undefined : onClick}
      title={tooltip}
      style={{
        height: 24, padding: '0 8px', borderRadius: 5,
        background: disabled ? colors.border : colors.cyanSoft,
        border: `1px solid ${disabled ? colors.border : colors.borderHi}`,
        fontFamily: font.body, fontSize: 10, fontWeight: 500,
        color: disabled ? colors.textDim : colors.cyan,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.5 : 1,
      }}
    >
      {label}
    </button>
  );
}
