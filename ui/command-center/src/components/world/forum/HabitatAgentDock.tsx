import { useEffect, useState } from 'react';
import { ROSTER } from '../agents';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import { subscribeWorldEvents } from '../shared/worldEvents';
import { FORUM_AGENT_PROFILES } from './agentProfiles';
import { HABITAT_PLACES, stationFor } from './habitatLayout';
import { commandHabitatAgent, stopHabitatAgent, useHabitatTravel } from './habitatMotion';
import { useCommandCenter } from '../../../lib/store';

export function HabitatAgentDock({selected,name,directing,following,onSelect,onDirect,onFollow,onInspect,onBrief,onConversation}:{selected:string;name:string;directing:boolean;following:boolean;onSelect:(id:string)=>void;onDirect:()=>void;onFollow:()=>void;onInspect:()=>void;onBrief:()=>void;onConversation:()=>void}){
  const states=useAgentRuntimeStates(),state=states.find(s=>s.id===selected),travel=useHabitatTravel(selected),station=stationFor(selected),profile=FORUM_AGENT_PROFILES[selected],authError=useCommandCenter(s=>s.daemonAuthError);
  const [expanded,setExpanded]=useState(false),[watching,setWatching]=useState(false),[events,setEvents]=useState<{id:string;line:string;time:string}[]>([]);
  useEffect(()=>subscribeWorldEvents(e=>{
    if(e.replayed||e.type!=='agent_state_changed')return;
    const id=typeof e.payload.agent_id==='string'?e.payload.agent_id:typeof e.payload.agent==='string'?e.payload.agent:null;
    if(!id||typeof e.payload.state!=='string')return;
    setEvents(old=>[{id,line:e.payload.state as string,time:e.timestamp?new Date(e.timestamp).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'}):'Just now'},...old].slice(0,24));
  }),[]);
  const live=!authError&&state?.source==='daemon',working=live&&state.hudState==='working';
  return <section className="habitat-agent-dock" aria-label="Agent movement and work" style={{'--identity':station.color} as React.CSSProperties}>
    <div className="habitat-agent-heading"><span className="habitat-agent-emblem" aria-hidden="true">◉</span><div><label htmlFor="habitat-agent">YOUR COLLABORATORS</label><select id="habitat-agent" value={selected} onChange={e=>onSelect(e.target.value)}>{ROSTER.map(a=><option key={a.id} value={a.id}>{a.isHenry?name:a.name}</option>)}</select></div><button className="habitat-subtle" aria-expanded={expanded} onClick={()=>setExpanded(v=>!v)}>{expanded?'Less':'Personality'}</button></div>
    <div className="habitat-agent-status"><span className={working?'is-working':''}/><b>{working?'Working':live?state.hudState:'No live work reported'}</b><span className="habitat-station">{station.name}</span></div>
    {expanded&&<div className="habitat-personality"><b>{profile.archetype}</b><p>{profile.manner}</p><small>{profile.voice}</small><button onClick={onInspect}>Meet & inspect capabilities</button></div>}
    <div className="habitat-agent-actions">{selected==='henry'&&<button className="habitat-primary habitat-sit" onClick={onConversation}>Sit with {name==='Your orchestrator'?'Hank':name} <span aria-hidden="true">↗</span></button>}<button aria-pressed={directing} onClick={onDirect}>{directing?'Choose a destination…':'Move agent'}</button><button onClick={()=>commandHabitatAgent(selected,[...station.position],'Walking to workstation',true)}>To workstation</button><button aria-pressed={following} onClick={onFollow}>{following?'Stop following':'Follow'}</button>{travel?.moving&&<button onClick={()=>stopHabitatAgent(selected)}>Stop</button>}<button className="habitat-primary" onClick={onBrief}>Give a brief</button></div>
    {directing&&<div className="habitat-destinations"><p>Click a clear path in the garden, or choose a place.</p>{HABITAT_PLACES.map(p=><button key={p.id} onClick={()=>{commandHabitatAgent(selected,p.id==='commons'?[.5,0,4.8]:[p.position[0],p.position[1],p.position[2]+1.5],`Walking to ${p.name.toLowerCase()}`);onDirect();}}>{p.name}</button>)}</div>}
    <div className="habitat-travel" role="status">{travel?.label??'At workstation'}<button onClick={()=>{setWatching(v=>!v);if(!following)onFollow();}} aria-expanded={watching}>{watching?'Hide work feed':'Watch work'}</button></div>
    {watching&&<div className="habitat-work-feed"><b>{station.detail}</b><p>{authError?'Connect to your agent to see live work here.':working?'The daemon reports this agent working. Their workstation reflects that state.':'No active work is reported for this agent. Give a brief to request a task.'}</p>{events.filter(e=>e.id===selected).slice(0,4).map((e,i)=><div key={`${e.time}-${i}`}><time>{e.time}</time> {e.line.replace(/_/g,' ')}</div>)}<small>Movement controls the avatar. Task requests and approvals use the agent desk.</small></div>}
  </section>;
}
