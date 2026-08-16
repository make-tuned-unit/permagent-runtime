import { useState } from 'react';
import { COLORS } from './constants';
import { ROSTER } from './agents';
import { useOrchestratorName } from './shared/useOrchestratorName';
import { useCommandCenter } from '../../lib/store';

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
  const orchestratorName = useOrchestratorName();
  const openAgentSettings = useCommandCenter(s => s.openAgentSettings);

  // Henry's live persona name overrides the roster fallback; others use their role.
  const displayName = (id: string, fallback: string) =>
    id === 'henry' ? orchestratorName ?? fallback : fallback;
  const roleLabel = (role: string) => (role === 'orchestrator' ? 'orchestrator' : 'worker');

  const selected = ROSTER.find((a) => a.id === selectedAgentId);

  return (
    <div style={containerStyle}>
      <button onClick={() => setOpen(!open)} style={triggerStyle}>
        <span style={{ fontSize: 11, color: COLORS.primaryMarble }}>
          {selected ? displayName(selected.id, selected.name) : 'Select agent'}
        </span>
        <span style={{ fontSize: 10, color: '#6B7280', marginLeft: 6 }}>
          {open ? '▲' : '▼'}
        </span>
      </button>

      {selectedAgentId && (
        <button
          type="button"
          onClick={() => openAgentSettings(selectedAgentId)}
          style={{
            ...triggerStyle,
            marginLeft: 8,
            fontSize: 10,
            color: COLORS.neonCyan,
          }}
        >
          Manage in Settings
        </button>
      )}

      {open && (
        <div style={dropdownStyle}>
          {ROSTER.map((agent) => (
            <button
              key={agent.id}
              onClick={() => {
                onSelectAgent(agent.id);
                setOpen(false);
              }}
              style={{
                ...itemStyle,
                background: agent.id === selectedAgentId ? 'rgba(0,213,255,0.12)' : 'transparent',
              }}
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
            </button>
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

const triggerStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  padding: '8px 14px',
  background: 'rgba(10, 14, 26, 0.88)',
  backdropFilter: 'blur(12px)',
  border: `1px solid ${COLORS.marbleVeining}25`,
  borderRadius: 8,
  cursor: 'pointer',
  fontFamily: 'monospace',
};

const dropdownStyle: React.CSSProperties = {
  position: 'absolute',
  bottom: '100%',
  left: 0,
  marginBottom: 4,
  minWidth: 200,
  background: 'rgba(10, 14, 26, 0.92)',
  backdropFilter: 'blur(16px)',
  border: `1px solid ${COLORS.marbleVeining}25`,
  borderRadius: 8,
  overflow: 'hidden',
  fontFamily: 'monospace',
};

const itemStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  width: '100%',
  padding: '8px 14px',
  border: 'none',
  cursor: 'pointer',
  fontFamily: 'monospace',
  fontSize: 11,
  textAlign: 'left',
};
