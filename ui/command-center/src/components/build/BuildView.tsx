import { useCallback, useEffect, useRef, type CSSProperties } from 'react';
import { Panel, Group, Separator } from 'react-resizable-panels';
import { FiGlobe, FiTerminal } from 'react-icons/fi';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';
import { useDashboard } from '../dashboard/useDashboard';
import { useCommandCenter } from '../../lib/store';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { TerminalManager } from '../terminal/TerminalManager';
import type { TerminalManagerHandle } from '../terminal/TerminalManager';
import { claimLaunch } from './pendingLaunch';
import { Browser } from '../browser';
import { ProjectChip } from './ProjectChip';
import { CostStatusline } from './CostStatusline';
import { progressRailStep } from '../../lib/buildProgress';
import type { Project } from './useProjects';
import { ViewHeader } from '../common/ViewHeader';
import { Button } from '../common/Button';

import { Tooltip } from '../common/Tooltip';
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
 * `aria-pressed`. Hover / focus / press used to be three pieces of local React
 * state re-deriving an inline `style` on every pointer event; they are now the
 * shared `.pa-btn` rules, fed the same colors through `--pa-btn-*`. Toggling a
 * pane is synchronous, so this never spins or ticks.
 */
function ToggleChip({
  active, label, title, colors, onToggle, children,
}: {
  active: boolean;
  label: string;
  title: string;
  colors: ThemeColors;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip content={title}>
      <Button
        colors={colors}
        type="button"
        aria-pressed={active}
        aria-label={label}
        onClick={onToggle}
        style={{
          '--pa-btn-bg': active ? colors.cyanSoft : 'transparent',
          '--pa-btn-fg': active ? colors.text : colors.textMuted,
          '--pa-btn-border': active ? colors.borderHi : colors.border,
          '--pa-btn-bg-hover': active ? colors.cyanSoft : colors.surfaceHi,
          '--pa-btn-border-hover': colors.borderHi,
          '--pa-btn-bg-active': active ? colors.cyanSoft : colors.surfaceHi,
          '--pa-btn-pad': '0 12px',
          '--pa-btn-radius': `${radius.md}px`,
          height: 30,
          fontSize: textSize.caption,
          gap: 6,
          opacity: active ? 1 : 0.7,
        } as CSSProperties}
      >
        {children}
      </Button>
    </Tooltip>
  );
}

export function BuildView() {
  const { gradient, colors } = useTheme();

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
    // When the terminal pane is hidden, TerminalManager isn't mounted at all
    // (BuildView renders it only inside `!buildTerminalHidden`) — the ref
    // below is null. Claiming and clearing anyway is how a "Send to Claude"
    // press turned into nothing: the launch looked consumed but no tab was
    // ever created. Unhide instead and return without claiming; that flips
    // buildTerminalHidden, which is in this effect's deps, so the effect
    // re-runs on the next commit with TerminalManager mounted and the ref
    // live (its useImperativeHandle is a layout effect and runs before this
    // passive one). If the ref is null for some other reason — the pane
    // isn't hidden, so there's nothing to unhide — leave the launch queued
    // rather than drop it; a later mount can still pick it up.
    if (!terminalRef.current) {
      if (buildTerminalHidden) toggleBuildTerminal();
      return;
    }
    // Claim SYNCHRONOUSLY, before createProjectTab: the clear below is async
    // (a store update), so a StrictMode double-invoke or a remount racing the
    // same tick would otherwise still see this launch and open a second tab
    // (see pendingLaunch.ts).
    if (!claimLaunch(pendingTerminalLaunch.id)) {
      setPendingTerminalLaunch(null);
      return;
    }
    const { id, rootPath, label, command, supervisedSessionId, followUpInput, growthAction } = pendingTerminalLaunch;
    terminalRef.current.createProjectTab(rootPath, label, command, supervisedSessionId, {
      followUpInput,
      growthAction,
      launchId: id,
    });
    setPendingTerminalLaunch(null);
  }, [pendingTerminalLaunch, setPendingTerminalLaunch, buildTerminalHidden, toggleBuildTerminal]);

  const agentName = data?.agent.name ?? 'Agent';
  const hasActive = (data?.in_flight.length ?? 0) > 0;
  const activeTask = hasActive ? data!.in_flight[0] : null;

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
        // A tab's title is the tab's name. Build used to swap in the active
        // task's title, which made it the one view whose header didn't say
        // what tab you were on. The task moved to the subtitle — still
        // visible, just no longer impersonating the page title.
        title="Build"
        // ProjectChip sits immediately right of the title (afterTitle), NOT in
        // `actions` at the far edge. The Build browser is a native webview that
        // composites above all DOM, so a menu opening on the right drops into
        // it and gets sliced; keeping the chip left, over the DOM pane, avoids
        // that. The Mobius that used to lead the header is gone — Build now
        // matches every other tab, whose header is a left-aligned title.
        afterTitle={<ProjectChip onLaunch={handleLaunch} onVisitSite={handleVisitSite} />}
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
              {/* Idle Build said only "idle" — true, and no help at all to
                  someone who has just arrived and is looking at an empty
                  terminal. One clause on the line that is already there. */}
              {!hasActive && !activeTask && ' — the terminal below runs your coding agent'}
            </span>
          </span>
        }
        actions={<>
        {/* Progress rail — driven by the daemon's per-task progress estimate
            (dashboard in_flight[].progress, 0..0.95). Previously hardcoded to
            step 3 whenever anything ran (2026-07 wiring audit).

            It used to render permanently: five flat grey bars sitting in the
            header of an idle Build tab, with no label, no title and no adjacent
            text. A shape that never changes and never says anything is
            decoration wearing an instrument's clothes — and worse, a reader who
            eventually saw it move had no way to know what it had been measuring
            all along. It now appears when there is progress to show, and says
            what it is measuring while it does. */}
        {hasActive && (
          <Tooltip content={`Step ${progressRailStep(activeTask?.progress)} of 5 — the daemon's own estimate of how far along this task is`}>
            <span tabIndex={0} style={{ outline: 'none' }}>
              <div
                data-testid="build-progress-rail"
                role="progressbar"
                aria-label={`Progress on ${activeTask?.title ?? 'the current task'}`}
                aria-valuemin={0}
                aria-valuemax={5}
                aria-valuenow={progressRailStep(activeTask?.progress)}
                style={{ display: 'flex', alignItems: 'center', gap: 8 }}
              >
                <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.body, whiteSpace: 'nowrap' }}>
                  Step {progressRailStep(activeTask?.progress)} of 5
                </span>
                <span style={{ display: 'flex', gap: 6 }}>
                  {[1, 2, 3, 4, 5].map(n => {
                    const step = progressRailStep(activeTask?.progress);
                    return (
                      <span key={n} style={{
                        width: 26, height: 4, borderRadius: 2,
                        background: n < step ? colors.success : n === step ? colors.cyan : colors.border,
                        boxShadow: n === step ? `0 0 6px ${colors.cyanGlow}` : 'none',
                      }} />
                    );
                  })}
                </span>
              </div>
            </span>
          </Tooltip>
        )}

        {/* Pane visibility: hide one pane to give the other the full canvas.
            The store guarantees both are never hidden at once. Each chip is a
            toggle button — `aria-pressed` reflects whether its pane is shown. */}
        <ToggleChip
          active={!buildTerminalHidden}
          label={buildTerminalHidden ? 'Show terminal panel' : 'Hide terminal panel'}
          title={buildTerminalHidden ? 'Show terminal' : 'Hide terminal — full-screen browser'}
          colors={colors}
          onToggle={toggleBuildTerminal}
        >
          <FiTerminal size={12} aria-hidden="true" />
          Terminal
        </ToggleChip>
        <ToggleChip
          active={!buildBrowserHidden}
          label={buildBrowserHidden ? 'Show browser panel' : 'Hide browser panel'}
          title={buildBrowserHidden ? 'Show browser' : 'Hide browser — full-screen terminal'}
          colors={colors}
          onToggle={toggleBuildBrowser}
        >
          <FiGlobe size={12} aria-hidden="true" />
          Browser
        </ToggleChip>

        {/* Pause removed (2026-07 wiring audit): it had no handler and the
            daemon has no pause verb for an in-flight run — an action-styled
            button that does nothing is worse than no button. Take over is
            real: it opens the running session in the chat dock. */}
        {hasActive && (
          <Tooltip content="Open this run's session in the chat dock to steer or stop it">
            <Button
              colors={colors}
              variant="primary"
              onClick={handleTakeOver}
              style={{
                '--pa-btn-pad': '0 14px',
                '--pa-btn-radius': `${radius.md}px`,
                height: 30,
                fontSize: textSize.caption,
                boxShadow: `0 0 14px ${colors.cyanGlow}`,
              } as CSSProperties}
            >Take over</Button>
          </Tooltip>
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
