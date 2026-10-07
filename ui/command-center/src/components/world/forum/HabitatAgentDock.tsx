import { useEffect, useState, type CSSProperties } from 'react';
import { ROSTER } from '../agents';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import { subscribeWorldEvents } from '../shared/worldEvents';
import { HABITAT_PLACES, stationFor } from './habitatLayout';
import { commandHabitatAgent, stopHabitatAgent, useHabitatTravel } from './habitatMotion';
import { useCommandCenter } from '../../../lib/store';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';

type Props = {
  selected: string; name: string; directing: boolean; following: boolean;
  onSelect: (id: string) => void; onDirect: () => void; onFollow: () => void;
  onInspect: () => void; onBrief: () => void; onConversation: () => void;
};

/** Inline vars beat Button.variantVars; CSS custom-property rules cannot. */
const DOCK_PAD = { '--pa-btn-pad': '5px 8px' } as CSSProperties;

export function HabitatAgentDock({ selected, name, directing, following, onSelect, onDirect, onFollow, onInspect, onBrief, onConversation }: Props) {
  const { colors } = useTheme();
  const states = useAgentRuntimeStates(), state = states.find(s => s.id === selected);
  const travel = useHabitatTravel(selected), station = stationFor(selected);
  const authError = useCommandCenter(s => s.daemonAuthError);
  const [movement, setMovement] = useState(false), [watching, setWatching] = useState(false);
  const [events, setEvents] = useState<{ id: string; line: string; time: string }[]>([]);
  useEffect(() => subscribeWorldEvents(e => {
    if (e.replayed || e.type !== 'agent_state_changed') return;
    const id = typeof e.payload.agent_id === 'string' ? e.payload.agent_id : typeof e.payload.agent === 'string' ? e.payload.agent : null;
    if (!id || typeof e.payload.state !== 'string') return;
    setEvents(old => [{ id, line: e.payload.state as string, time: e.timestamp ? new Date(e.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : 'Just now' }, ...old].slice(0, 24));
  }), []);
  const live = !authError && state?.source === 'daemon', working = live && state.hudState === 'working';
  const movementOpen = movement || directing;
  const button = { colors, flashSuccess: false, style: DOCK_PAD };
  const sitStyle = {
    ...DOCK_PAD,
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-border': colors.borderHi,
    '--pa-btn-bg-hover': colors.cyanSoft,
  } as CSSProperties;
  return <section className="habitat-agent-dock" aria-label="Agent movement and work" style={{ '--identity': station.color } as CSSProperties}>
    <div className="habitat-agent-heading">
      <span className="habitat-agent-emblem" aria-hidden="true">◉</span>
      <div><label htmlFor="habitat-agent">Collaborators</label><select id="habitat-agent" value={selected} onChange={e => { setMovement(false); onSelect(e.target.value); }}>{ROSTER.map(a => <option key={a.id} value={a.id}>{a.isHenry ? name : a.name}</option>)}</select></div>
      <Button {...button} variant="bare" className="habitat-subtle" style={{ ...DOCK_PAD, '--pa-btn-pad': '4px', '--pa-btn-fg': colors.textMuted } as CSSProperties} onClick={onInspect}>Details <span aria-hidden="true">↗</span></Button>
    </div>
    <div className="habitat-agent-status"><span className={working ? 'is-working' : ''} /><b>{working ? 'Working' : live ? state.hudState : 'No live work reported'}</b><span className="habitat-station">{station.name}</span></div>
    <div className="habitat-agent-actions">
      {selected === 'henry' && <Button {...button} variant="ghost" className="habitat-sit" style={sitStyle} onClick={onConversation}>Sit with {name} <span aria-hidden="true">↗</span></Button>}
      <Button {...button} variant="ghost" onClick={onBrief}>Give a brief</Button>
    </div>
    <div className="habitat-agent-utilities">
      <Button {...button} variant={movementOpen ? 'ghostOn' : 'bare'} style={{ ...DOCK_PAD, '--pa-btn-fg': movementOpen ? colors.cyan : colors.textMuted } as CSSProperties} aria-expanded={movementOpen} aria-controls="habitat-movement" onClick={() => { if (directing) onDirect(); setMovement(!movementOpen); }}>Movement <span aria-hidden="true">{movementOpen ? '−' : '+'}</span></Button>
      <Button {...button} variant={watching ? 'ghostOn' : 'bare'} style={{ ...DOCK_PAD, '--pa-btn-fg': watching ? colors.cyan : colors.textMuted } as CSSProperties} aria-expanded={watching} aria-controls="habitat-work-feed" onClick={() => setWatching(v => !v)}>Work feed <span aria-hidden="true">{watching ? '−' : '+'}</span></Button>
    </div>
    {movementOpen && <div id="habitat-movement" className="habitat-movement">
      <div className="habitat-movement-actions">
        <Button {...button} variant={directing ? 'ghostOn' : 'ghost'} aria-pressed={directing} onClick={onDirect}>{directing ? 'Cancel destination' : 'Move agent'}</Button>
        <Button {...button} variant={following ? 'ghostOn' : 'ghost'} aria-pressed={following} onClick={onFollow}>{following ? 'Stop following' : 'Follow'}</Button>
        <Button {...button} onClick={() => commandHabitatAgent(selected, [...station.position], 'Walking to workstation', true, 'user')}>To workstation</Button>
      </div>
      {directing && <div className="habitat-destinations"><p>Click a clear path, or choose a place.</p>{HABITAT_PLACES.map(p => <Button {...button} key={p.id} onClick={() => { if (commandHabitatAgent(selected, p.id === 'commons' ? [.5, 0, 4.8] : [p.position[0], p.position[1], p.position[2] + 1.5], `Walking to ${p.name.toLowerCase()}`)) onDirect(); }}>{p.name}</Button>)}</div>}
    </div>}
    {(movementOpen || travel?.moving || (travel && travel.label !== 'At workstation')) && <div className="habitat-travel" role="status"><span>{travel?.label ?? 'At workstation'}</span>{travel?.moving && <Button {...button} onClick={() => stopHabitatAgent(selected)}>Stop</Button>}</div>}
    {watching && <div id="habitat-work-feed" className="habitat-work-feed"><b>{station.detail}</b><p>{authError ? 'Connect to your agent to see live work here.' : working ? 'Your agent reports active work. Their workstation reflects that state.' : 'No active work is reported. Give a brief to request a task.'}</p>{events.filter(e => e.id === selected).slice(0, 4).map((e, i) => <div key={`${e.time}-${i}`}><time>{e.time}</time> {e.line.replace(/_/g, ' ')}</div>)}</div>}
  </section>;
}
