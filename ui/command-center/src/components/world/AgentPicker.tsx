import { useState, type CSSProperties } from 'react';
import { ROSTER } from './agents';
import { useOrchestratorName } from './shared/useOrchestratorName';
import { useCommandCenter } from '../../lib/store';
import { agentIdForWorldAgent } from '../../lib/worldAgentIds';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { Button } from '../common/Button';
import {
  HUD_GEOM,
  HUD_PANEL_RADIUS,
  hudBareVars,
  hudTransition,
} from './hudChrome';

interface AgentPickerProps {
  selectedAgentId: string | null;
  onSelectAgent: (id: string) => void;
}

// The dropdown lists exactly the world's real inhabitants (the ROSTER): Henry the
// orchestrator, the Reader, and the Librarian. Sourcing from the ROSTER — rather than
// the daemon's /api/agents — guarantees every entry maps to a 3D agent the camera can
// fly to and a HUD that exists, so selecting always "brings you to that agent."
export function AgentPicker({ selectedAgentId, onSelectAgent }: AgentPickerProps) {
  const [open, setOpen] = useState(false);
  const { colors, reduceMotion } = useTheme();
  const glass = useGlass('glass');
  const orchestratorName = useOrchestratorName();
  const openAgentSettings = useCommandCenter(s => s.openAgentSettings);

  const displayName = (id: string, fallback: string) =>
    id === 'henry' ? orchestratorName ?? fallback : fallback;
  const roleLabel = (role: string) => (role === 'orchestrator' ? 'orchestrator' : 'worker');

  const selected = ROSTER.find((a) => a.id === selectedAgentId);
  // Not every in-world character is an agent the API knows: Henry is the
  // orchestrator and the Reader is a surface, neither has a roster entry, and
  // the Steward's id differs across the two namespaces. Offer the deep-link only
  // where it resolves, rather than a button that lands on "no agent named …".
  const manageableAgentId = selectedAgentId ? agentIdForWorldAgent(selectedAgentId) : null;

  // One glass plane wraps the trigger; the Button itself is transparent with
  // fillHover/fillActive (D2/D10) — never a second backdrop-filter.
  const glassChip: CSSProperties = {
    ...glass,
    border: `1px solid ${colors.border}`,
    borderRadius: HUD_PANEL_RADIUS,
    overflow: 'hidden',
    transition: hudTransition(reduceMotion),
  };

  const triggerVars = {
    ...hudBareVars(colors, {
      fg: colors.text,
      fgHover: colors.text,
      pad: `${space.md}px ${HUD_GEOM.panelPadX}px`,
      radiusPx: 0,
    }),
    fontFamily: font.mono,
    transition: hudTransition(reduceMotion),
  } as CSSProperties;

  const dropdownStyle: CSSProperties = {
    position: 'absolute',
    bottom: '100%',
    left: 0,
    marginBottom: space.xs,
    minWidth: 200,
    // Opaque elevated menu on glass trigger — not a second glass plane (D2).
    background: colors.surface,
    boxShadow: colors.elevationOverlay,
    border: `1px solid ${colors.border}`,
    borderRadius: HUD_PANEL_RADIUS,
    overflow: 'hidden',
    fontFamily: font.mono,
  };

  const itemVars = {
    ...hudBareVars(colors, {
      pad: `${space.md}px ${HUD_GEOM.panelPadX}px`,
      radiusPx: 0,
    }),
    justifyContent: 'space-between',
    width: '100%',
    fontFamily: font.mono,
    fontSize: textSize.micro,
    textAlign: 'left' as const,
    transition: hudTransition(reduceMotion),
  } as CSSProperties;

  return (
    <div style={{
      position: 'absolute',
      bottom: HUD_GEOM.panelInset,
      left: HUD_GEOM.panelInset,
      zIndex: 10,
      pointerEvents: 'auto',
      display: 'flex',
      alignItems: 'center',
      gap: space.md,
    }}>
      <div style={{ position: 'relative' }}>
        <div style={glassChip}>
          <Button
            colors={colors}
            type="button"
            onClick={() => setOpen(!open)}
            flashSuccess={false}
            style={triggerVars}
          >
            <span style={{ fontSize: textSize.micro, color: colors.text }}>
              {selected ? displayName(selected.id, selected.name) : 'Select agent'}
            </span>
            <span style={{ fontSize: textSize.micro, color: colors.textMuted, marginLeft: space.sm }}>
              {open ? '▲' : '▼'}
            </span>
          </Button>
        </div>

        {open && (
          <div style={dropdownStyle}>
            {ROSTER.map((agent) => {
              const selectedRow = agent.id === selectedAgentId;
              return (
                <Button
                  key={agent.id}
                  colors={colors}
                  onClick={() => {
                    onSelectAgent(agent.id);
                    setOpen(false);
                  }}
                  style={{
                    ...itemVars,
                    '--pa-btn-bg': selectedRow ? colors.cyanSoft : 'transparent',
                    '--pa-btn-bg-hover': selectedRow ? colors.cyanSoft : colors.fillHover,
                    '--pa-btn-bg-active': selectedRow ? colors.cyanSoft : colors.fillActive,
                    '--pa-btn-fg': selectedRow ? colors.cyan : colors.text,
                  } as CSSProperties}
                >
                  <span style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
                    {/* Identity trim swatch — matches the agent's toga trim in-world. */}
                    <span style={{
                      width: space.md,
                      height: space.md,
                      borderRadius: radius.pill,
                      background: agent.trimColor,
                    }} />
                    <span style={{
                      fontWeight: selectedRow ? 600 : 400,
                      color: selectedRow ? colors.cyan : colors.text,
                    }}>
                      {displayName(agent.id, agent.name)}
                    </span>
                  </span>
                  <span style={{ fontSize: textSize.micro, color: colors.textMuted, marginLeft: space.md }}>
                    {roleLabel(agent.role)}
                  </span>
                </Button>
              );
            })}
          </div>
        )}
      </div>

      {manageableAgentId && (
        <div style={glassChip}>
          <Button
            colors={colors}
            type="button"
            onClick={() => openAgentSettings(manageableAgentId)}
            style={{
              ...triggerVars,
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-fg-hover': colors.cyan,
              fontSize: textSize.micro,
            } as CSSProperties}
          >
            Manage in Settings
          </Button>
        </div>
      )}
    </div>
  );
}
