import { useState, type CSSProperties } from 'react';
import { COLORS } from './constants';
import { ROSTER } from './agents';
import { useOrchestratorName } from './shared/useOrchestratorName';
import { useCommandCenter } from '../../lib/store';
import { agentIdForWorldAgent } from '../../lib/worldAgentIds';
import { radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

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
  // World chrome keeps the world palette; `colors` only feeds the button
  // primitive's variant defaults — every visible value below is a COLORS one.
  const { colors } = useTheme();
  const orchestratorName = useOrchestratorName();
  const openAgentSettings = useCommandCenter(s => s.openAgentSettings);

  // Henry's live persona name overrides the roster fallback; others use their role.
  const displayName = (id: string, fallback: string) =>
    id === 'henry' ? orchestratorName ?? fallback : fallback;
  const roleLabel = (role: string) => (role === 'orchestrator' ? 'orchestrator' : 'worker');

  const selected = ROSTER.find((a) => a.id === selectedAgentId);
  // Not every in-world character is an agent the API knows: Henry is the
  // orchestrator and the Reader is a surface, neither has a roster entry, and
  // the Steward's id differs across the two namespaces. Offer the deep-link only
  // where it resolves, rather than a button that lands on "no agent named …".
  const manageableAgentId = selectedAgentId ? agentIdForWorldAgent(selectedAgentId) : null;

  return (
    <div style={containerStyle}>
      <Button
        colors={colors}
        type="button"
        onClick={() => setOpen(!open)}
        flashSuccess={false}
        style={triggerVars}
      >
        <span style={{ fontSize: textSize.micro, color: COLORS.primaryMarble }}>
          {selected ? displayName(selected.id, selected.name) : 'Select agent'}
        </span>
        <span style={{ fontSize: 10, color: '#6B7280', marginLeft: 6 }}>
          {open ? '▲' : '▼'}
        </span>
      </Button>

      {manageableAgentId && (
        <Button
          colors={colors}
          type="button"
          onClick={() => openAgentSettings(manageableAgentId)}
          style={{
            ...triggerVars,
            '--pa-btn-fg': COLORS.neonCyan,
            '--pa-btn-fg-hover': COLORS.neonCyan,
            marginLeft: 8,
            fontSize: 10,
          } as CSSProperties}
        >
          Manage in Settings
        </Button>
      )}

      {open && (
        <div style={dropdownStyle}>
          {/* A name group pushed apart from a role label: the children have to
              stay the button's own flex children, which `.pa-btn__label`'s
              `display: contents` gives them. They had no hover or pressed state
              at all before this — the list read as inert. */}
          {ROSTER.map((agent) => (
            <Button
              key={agent.id}
              colors={colors}
              onClick={() => {
                onSelectAgent(agent.id);
                setOpen(false);
              }}
              style={{
                ...itemVars,
                '--pa-btn-bg': agent.id === selectedAgentId ? 'rgba(0,213,255,0.12)' : 'transparent',
                '--pa-btn-bg-hover': agent.id === selectedAgentId ? 'rgba(0,213,255,0.20)' : 'rgba(255,255,255,0.06)',
                '--pa-btn-bg-active': agent.id === selectedAgentId ? 'rgba(0,213,255,0.12)' : 'transparent',
              } as CSSProperties}
            >
              <span style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                {/* Identity trim swatch — matches the agent's toga trim in-world. */}
                <span style={{ width: 8, height: 8, borderRadius: '50%', background: agent.trimColor }} />
                <span style={{
                  fontWeight: agent.id === selectedAgentId ? 600 : 400,
                  color: agent.id === selectedAgentId ? COLORS.neonCyan : COLORS.primaryMarble,
                }}>
                  {displayName(agent.id, agent.name)}
                </span>
              </span>
              <span style={{ fontSize: 10, color: '#6B7280', marginLeft: 8 }}>
                {roleLabel(agent.role)}
              </span>
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}

const containerStyle: React.CSSProperties = {
  position: 'absolute',
  bottom: 16,
  left: 16,
  zIndex: 10,
  pointerEvents: 'auto',
  display: 'flex',
  alignItems: 'center',
};

const triggerVars = {
  '--pa-btn-bg': 'rgba(10, 14, 26, 0.88)',
  '--pa-btn-fg': COLORS.primaryMarble,
  '--pa-btn-border': `${COLORS.marbleVeining}25`,
  '--pa-btn-bg-hover': 'rgba(16, 22, 38, 0.92)',
  '--pa-btn-border-hover': `${COLORS.marbleVeining}45`,
  '--pa-btn-bg-active': 'rgba(10, 14, 26, 0.88)',
  '--pa-btn-pad': '8px 14px',
  '--pa-btn-radius': `${radius.md}px`,
  backdropFilter: 'blur(12px)',
  fontFamily: 'monospace',
} as CSSProperties;

const dropdownStyle: React.CSSProperties = {
  position: 'absolute',
  bottom: '100%',
  left: 0,
  marginBottom: 4,
  minWidth: 200,
  background: 'rgba(10, 14, 26, 0.92)',
  backdropFilter: 'blur(16px)',
  border: `1px solid ${COLORS.marbleVeining}25`,
  borderRadius: radius.md,
  overflow: 'hidden',
  fontFamily: 'monospace',
};

const itemVars = {
  '--pa-btn-border': 'transparent',
  '--pa-btn-border-hover': 'transparent',
  '--pa-btn-pad': '8px 14px',
  '--pa-btn-radius': '0',
  justifyContent: 'space-between',
  width: '100%',
  fontFamily: 'monospace',
  fontSize: textSize.micro,
  textAlign: 'left',
} as CSSProperties;
