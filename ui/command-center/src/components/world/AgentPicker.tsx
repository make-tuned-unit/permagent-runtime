import { useState, useEffect } from 'react';
import { COLORS } from './constants';
import { api } from '../../lib/api';

interface Agent {
  id: string;
  name: string;
  role: string;
  source: string;
}

interface AgentPickerProps {
  selectedAgentId: string | null;
  onSelectAgent: (id: string) => void;
}

export function AgentPicker({ selectedAgentId, onSelectAgent }: AgentPickerProps) {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    api.getAgents()
      .then(res => setAgents(res.agents))
      .catch((err: unknown) => console.warn('[AgentPicker] Failed to fetch agents:', err));
  }, []);

  if (agents.length === 0) return null;

  return (
    <div style={containerStyle}>
      <button
        onClick={() => setOpen(!open)}
        style={triggerStyle}
      >
        <span style={{ fontSize: 11, color: COLORS.primaryMarble }}>
          {selectedAgentId
            ? agents.find(a => a.id === selectedAgentId)?.name ?? 'Select agent'
            : 'Select agent'}
        </span>
        <span style={{ fontSize: 9, color: '#6B7280', marginLeft: 6 }}>
          {open ? '▲' : '▼'}
        </span>
      </button>

      {open && (
        <div style={dropdownStyle}>
          {agents.map(agent => (
            <button
              key={agent.id}
              onClick={() => {
                onSelectAgent(agent.id);
                setOpen(false);
              }}
              style={{
                ...itemStyle,
                background: agent.id === selectedAgentId
                  ? 'rgba(0,213,255,0.12)'
                  : 'transparent',
              }}
            >
              <span style={{
                fontWeight: agent.id === selectedAgentId ? 600 : 400,
                color: agent.id === selectedAgentId ? COLORS.neonCyan : COLORS.primaryMarble,
              }}>
                {agent.name}
              </span>
              <span style={{ fontSize: 9, color: '#6B7280', marginLeft: 8 }}>
                {agent.source === 'persona' ? 'primary' : 'extension'}
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
  minWidth: 180,
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
