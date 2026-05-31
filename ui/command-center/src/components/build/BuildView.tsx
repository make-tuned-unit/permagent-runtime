import { useCallback } from 'react';
import { Panel, Group, Separator } from 'react-resizable-panels';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { useDashboard } from '../dashboard/useDashboard';
import { TerminalManager } from '../terminal/TerminalManager';
import { Browser } from '../browser';
import { ProjectChip } from './ProjectChip';
import type { Project } from './useProjects';

export function BuildView() {
  const { gradient, colors } = useTheme();

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

  const agentName = data?.agent.name ?? 'Agent';
  const hasActive = (data?.in_flight.length ?? 0) > 0;
  const activeTask = hasActive ? data!.in_flight[0] : null;
  const mobiusState = hasActive ? 'thinking' : 'idle';

  const handleLaunch = useCallback((_project: Project, _agent: string) => {
    // TODO: spawn PTY tab with cwd=project.rootPath, label=slug·agent
    // For now this is a placeholder — terminal spawning with project affinity
    // requires TerminalManager API extension (next iteration)
  }, []);

  const handleVisitSite = useCallback((url: string) => {
    window.open(url, '_blank');
  }, []);

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
              boxShadow: hasActive ? '0 0 6px rgba(0,213,255,0.7)' : 'none',
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
                background: n < step ? '#5BD17F' : n === step ? colors.cyan : colors.border,
                boxShadow: n === step ? `0 0 6px ${colors.cyanGlow}` : 'none',
              }} />
            );
          })}
        </div>

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
        <Group orientation="horizontal">
          <Panel id="build-terminal" defaultSize={50} minSize={20}>
            <div style={{ height: '100%', borderRadius: radius.md, overflow: 'hidden', border: `1px solid ${colors.border}` }}>
              <TerminalManager />
            </div>
          </Panel>
          <Separator className="group relative flex items-center justify-center w-1">
            <div className="bg-dark-border group-hover:bg-accent/50 group-active:bg-accent transition-colors w-px h-full" />
          </Separator>
          <Panel id="build-browser" defaultSize={50} minSize={20}>
            <div style={{ height: '100%', borderRadius: radius.md, overflow: 'hidden', border: `1px solid ${colors.border}` }}>
              <Browser />
            </div>
          </Panel>
        </Group>
      </div>
    </div>
  );
}
