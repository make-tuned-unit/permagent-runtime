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
import type { Project } from './useProjects';

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

  const ghostBtn: React.CSSProperties = {
    height: 30, padding: '0 12px', borderRadius: 8,
    background: 'transparent', border: `1px solid ${colors.border}`,
    fontFamily: font.body, fontSize: 12, fontWeight: 500,
    color: colors.text, cursor: 'pointer',
    display: 'inline-flex', alignItems: 'center', gap: 6,
  };

  const primaryBtn: React.CSSProperties = {
    height: 30, padding: '0 14px', borderRadius: 8,
    background: colors.cyan, color: colors.bg, border: 'none',
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
  useEffect(() => {
    if (!pendingTerminalLaunch) return;
    const { rootPath, label, command } = pendingTerminalLaunch;
    terminalRef.current?.createProjectTab(rootPath, label, command);
    setPendingTerminalLaunch(null);
  }, [pendingTerminalLaunch, setPendingTerminalLaunch]);

  const agentName = data?.agent.name ?? 'Agent';
  const hasActive = (data?.in_flight.length ?? 0) > 0;
  const activeTask = hasActive ? data!.in_flight[0] : null;
  const mobiusState = hasActive ? 'thinking' : 'idle';

  const handleLaunch = useCallback((project: Project, agent: string) => {
    if (!project.rootPath) return;
    const label = `${project.slug} · ${agent}`;
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
      {/* Title strip */}
      <div style={{
        padding: '14px 24px', display: 'flex', alignItems: 'center', gap: 14,
        borderBottom: `1px solid ${colors.border}`, flexShrink: 0,
      }}>
        <Mobius size={36} state={mobiusState as any} glow={0.9} />
        <div style={{ minWidth: 0 }}>
          <div style={{ fontFamily: font.display, fontSize: 15, fontWeight: 600, letterSpacing: '-0.01em' }}>
            {activeTask ? activeTask.title : 'Build'}
          </div>
          <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2, display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{
              width: 5, height: 5, borderRadius: '50%',
              background: hasActive ? colors.cyan : colors.textDim,
              boxShadow: hasActive ? `0 0 6px ${colors.cyanGlow}` : 'none',
            }} />
            {agentName} · {hasActive ? 'thinking' : 'idle'}
          </div>
        </div>
        <ProjectChip onLaunch={handleLaunch} onVisitSite={handleVisitSite} />
        <div style={{ flex: 1 }} />

        {/* Progress rail */}
        <div style={{ display: 'flex', gap: 6 }}>
          {[1, 2, 3, 4, 5].map(n => {
            const step = hasActive ? 3 : 0;
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

        {hasActive && (
          <>
            <button style={ghostBtn}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
                <rect x="6" y="4" width="4" height="16" rx="1" /><rect x="14" y="4" width="4" height="16" rx="1" /></svg>
              Pause
            </button>
            <button style={primaryBtn}>Take over</button>
          </>
        )}
      </div>

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
