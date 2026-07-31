import { useCallback, useEffect, useRef, useState } from 'react';
import { Panel, Group, Separator } from 'react-resizable-panels';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { useDashboard } from '../dashboard/useDashboard';
import { useCommandCenter } from '../../lib/store';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { TerminalManager } from '../terminal/TerminalManager';
import type { TerminalManagerHandle } from '../terminal/TerminalManager';
import { Browser } from '../browser';
import { ProjectChip } from './ProjectChip';
import { CostStatusline } from './CostStatusline';
import { progressRailStep } from '../../lib/buildProgress';
import type { Project } from './useProjects';
import { ViewHeader } from '../common/ViewHeader';

// Ensure a project site_url has a scheme so the in-app browser navigates
// instead of treating it as a search query (e.g. www.reckonize.org → https://…).
function ensureScheme(url: string): string {
  const trimmed = url.trim();
  if (!trimmed) return '';
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  return `https://${trimmed}`;
}

/**
 * Pane-visibility toggle in the Build toolbar. It is a true toggle button:
 * `active` = the pane is currently shown, surfaced to assistive tech via
 * `aria-pressed`. Hover / focus / press states are driven from local state
 * because the toolbar is styled inline (no stylesheet pseudo-classes here).
 */
function ToggleChip({
  active, label, title, colors, reduceMotion, onToggle, children,
}: {
  active: boolean;
  label: string;
  title: string;
  colors: ThemeColors;
  reduceMotion: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  const [hover, setHover] = useState(false);
  const [focus, setFocus] = useState(false);
  const [pressed, setPressed] = useState(false);

  const borderColor = active || hover || focus ? colors.borderHi : colors.border;
  const ring = focus ? `, 0 0 0 3px ${colors.cyanGlow}` : '';

  const style: React.CSSProperties = {
    height: 30, padding: '0 12px', borderRadius: 8,
    background: active ? colors.cyanSoft : hover ? colors.surfaceHi : 'transparent',
    border: `1px solid ${borderColor}`,
    fontFamily: font.body, fontSize: 12, fontWeight: 500,
    color: active ? colors.text : colors.textMuted,
    opacity: active ? 1 : 0.7,
    cursor: 'pointer',
    display: 'inline-flex', alignItems: 'center', gap: 6,
    outline: 'none',
    boxShadow: `none${ring}`,
    transform: pressed ? 'translateY(0.5px)' : 'none',
    transition: reduceMotion
      ? 'none'
      : 'background 140ms ease, border-color 140ms ease, color 140ms ease, box-shadow 140ms ease, opacity 140ms ease',
  };

  return (
    <button
      type="button"
      style={style}
      aria-pressed={active}
      aria-label={label}
      title={title}
      onClick={onToggle}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => { setHover(false); setPressed(false); }}
      onMouseDown={() => setPressed(true)}
      onMouseUp={() => setPressed(false)}
      onFocus={() => setFocus(true)}
      onBlur={() => { setFocus(false); setPressed(false); }}
    >
      {children}
    </button>
  );
}

export function BuildView() {
  const { gradient, colors, reduceMotion } = useTheme();

  const primaryBtn: React.CSSProperties = {
    height: 30, padding: '0 14px', borderRadius: 8,
    background: colors.cyan, color: colors.textOnCyan, border: 'none',
    fontFamily: font.body, fontSize: 12, fontWeight: 600,
    cursor: 'pointer', boxShadow: `0 0 14px ${colors.cyanGlow}`,
  };
  const { data } = useDashboard();
  const terminalRef = useRef<TerminalManagerHandle>(null);

  // Agent-driven launch (project_launch event): useAppNavigate switches to this
  // tab and queues a pending launch; consume it here via the TerminalManager ref
  // — the same createProjectTab path a human gets from the project's launch button.
  const pendingTerminalLaunch = useCommandCenter(s => s.pendingTerminalLaunch);
  const buildTerminalHidden = useCommandCenter(s => s.buildTerminalHidden);
  const buildBrowserHidden = useCommandCenter(s => s.buildBrowserHidden);
  const toggleBuildTerminal = useCommandCenter(s => s.toggleBuildTerminal);
  const toggleBuildBrowser = useCommandCenter(s => s.toggleBuildBrowser);
  const setPendingTerminalLaunch = useCommandCenter(s => s.setPendingTerminalLaunch);
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const openChatDock = useCommandCenter(s => s.openChatDock);
  useEffect(() => {
    if (!pendingTerminalLaunch) return;
    const { rootPath, label, command, supervisedSessionId } = pendingTerminalLaunch;
    terminalRef.current?.createProjectTab(rootPath, label, command, supervisedSessionId);
    setPendingTerminalLaunch(null);
  }, [pendingTerminalLaunch, setPendingTerminalLaunch]);

  const agentName = data?.agent.name ?? 'Agent';
  const hasActive = (data?.in_flight.length ?? 0) > 0;
  const activeTask = hasActive ? data!.in_flight[0] : null;
  const mobiusState = hasActive ? 'thinking' : 'idle';

  // Take over: open the in-flight session in the chat dock so the user can
  // steer (or stop) the run directly. `in_flight[i].id` IS a session id — the
  // dashboard builds it from active sessions.
  const handleTakeOver = useCallback(() => {
    if (!activeTask) return;
    switchToSession(activeTask.id).catch(err => console.error('[build] take-over failed:', err));
    openChatDock();
  }, [activeTask, switchToSession, openChatDock]);

  const handleLaunch = useCallback((project: Project, agent: string) => {
    if (!project.rootPath) return;
    // `agent` may be a bare CLI name ("claude", "codex") or a full command
    // (the Permagent harness launches `permagent run --recipe …`). Show only
    // the program name in the tab label.
    const display = agent.split(' ')[0] || agent;
    const label = `${project.slug} · ${display}`;
    terminalRef.current?.createProjectTab(project.rootPath, label, agent);
  }, []);

  const openInBrowser = useBrowserNavigate();
  const handleVisitSite = useCallback((url: string) => {
    const normalized = ensureScheme(url);
    if (!normalized) return;
    openInBrowser(normalized);
  }, [openInBrowser]);

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
      background: gradient.workspace,
      color: colors.text, fontFamily: font.body,
    }}>
      <ViewHeader
        leading={<Mobius size={36} state={mobiusState as any} glow={0.9} />}
        // A tab's title is the tab's name. Build used to swap in the active
        // task's title, which made it the one view whose header didn't say
        // what tab you were on. The task moved to the subtitle — still
        // visible, just no longer impersonating the page title.
        title="Build"
        subtitle={
          <span style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
            <span style={{
              width: 5, height: 5, borderRadius: '50%', flexShrink: 0,
              background: hasActive ? colors.cyan : colors.textDim,
              boxShadow: hasActive ? `0 0 6px ${colors.cyanGlow}` : 'none',
            }} />
            {activeTask && (
              // Task titles are user/agent text and can be long — truncate
              // here rather than let the subtitle push the bar to two rows.
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>
                {activeTask.title}
              </span>
            )}
            <span style={{ flexShrink: 0 }}>
              {activeTask ? '· ' : ''}{agentName} · {hasActive ? 'thinking' : 'idle'}
            </span>
          </span>
        }
        actions={<>
          <ProjectChip onLaunch={handleLaunch} onVisitSite={handleVisitSite} />

        {/* Progress rail — driven by the daemon's per-task progress estimate
            (dashboard in_flight[].progress, 0..0.95). Previously hardcoded to
            step 3 whenever anything ran (2026-07 wiring audit). */}
        <div style={{ display: 'flex', gap: 6 }} aria-hidden={!hasActive}>
          {[1, 2, 3, 4, 5].map(n => {
            const step = progressRailStep(activeTask?.progress);
            return (
              <div key={n} style={{
                width: 26, height: 4, borderRadius: 2,
                background: n < step ? colors.success : n === step ? colors.cyan : colors.border,
                boxShadow: n === step ? `0 0 6px ${colors.cyanGlow}` : 'none',
              }} />
            );
          })}
        </div>

        {/* Pane visibility: hide one pane to give the other the full canvas.
            The store guarantees both are never hidden at once. Each chip is a
            toggle button — `aria-pressed` reflects whether its pane is shown. */}
        <ToggleChip
          active={!buildTerminalHidden}
          label={buildTerminalHidden ? 'Show terminal panel' : 'Hide terminal panel'}
          title={buildTerminalHidden ? 'Show terminal' : 'Hide terminal — full-screen browser'}
          colors={colors}
          reduceMotion={reduceMotion}
          onToggle={toggleBuildTerminal}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden="true">
            <polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" /></svg>
          Terminal
        </ToggleChip>
        <ToggleChip
          active={!buildBrowserHidden}
          label={buildBrowserHidden ? 'Show browser panel' : 'Hide browser panel'}
          title={buildBrowserHidden ? 'Show browser' : 'Hide browser — full-screen terminal'}
          colors={colors}
          reduceMotion={reduceMotion}
          onToggle={toggleBuildBrowser}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden="true">
            <circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18" /></svg>
          Browser
        </ToggleChip>

        {/* Pause removed (2026-07 wiring audit): it had no handler and the
            daemon has no pause verb for an in-flight run — an action-styled
            button that does nothing is worse than no button. Take over is
            real: it opens the running session in the chat dock. */}
        {hasActive && (
          <button
            style={primaryBtn}
            onClick={handleTakeOver}
            title="Open this run's session in the chat dock to steer or stop it"
          >Take over</button>
        )}
        </>}
      />

      {/* Terminal + Browser side by side, resizable */}
      <div style={{ flex: 1, minHeight: 0, padding: '12px 18px' }}>
        <Group
          orientation="horizontal"
          key={`${buildTerminalHidden}-${buildBrowserHidden}`}
        >
          {!buildTerminalHidden && (
          <Panel id="build-terminal" defaultSize={buildBrowserHidden ? 100 : 50} minSize={20}>
            <div style={{ height: '100%', borderRadius: radius.md, overflow: 'hidden', border: `1px solid ${colors.border}` }}>
              <TerminalManager ref={terminalRef} />
            </div>
          </Panel>
          )}
          {!buildTerminalHidden && !buildBrowserHidden && (
          <Separator
            className="relative flex items-center justify-center w-1"
            onMouseEnter={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = `${colors.cyan}80`; }}
            onMouseLeave={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = colors.border; }}
            onMouseDown={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = colors.cyan; }}
            onMouseUp={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = `${colors.cyan}80`; }}
          >
            <div className="transition-colors w-px h-full" style={{ backgroundColor: colors.border }} />
          </Separator>
          )}
          {!buildBrowserHidden && (
          <Panel id="build-browser" defaultSize={buildTerminalHidden ? 100 : 50} minSize={20}>
            <div style={{ height: '100%', borderRadius: radius.md, overflow: 'hidden', border: `1px solid ${colors.border}` }}>
              <Browser />
            </div>
          </Panel>
          )}
        </Group>
      </div>

      {/* Always-on cost meter: live, single-sourced session $ from the SSE ledger. */}
      <CostStatusline />
    </div>
  );
}
