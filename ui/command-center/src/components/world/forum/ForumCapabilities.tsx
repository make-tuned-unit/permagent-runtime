import { useEffect, useState } from 'react';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { subscribeWorldEvents } from '../shared/worldEvents';
import { loadForumRuntime, type ForumRuntimeSnapshot } from './forumRuntime';
export function ForumCapabilities({ onDevelop, onSkills, onManageAgent }: {
  onDevelop: () => void; onSkills?: () => void; onManageAgent?: (id:string)=>void;
}) {
  const { colors }=useTheme();
  const [data,setData]=useState<ForumRuntimeSnapshot|null>(null);
  useEffect(()=>{
    let cancelled=false, latest=0;
    const refresh=async()=>{const request=++latest;const next=await loadForumRuntime();if(!cancelled&&request===latest)setData(next);};
    void refresh();const timer=setInterval(()=>void refresh(),30000);
    const unsubscribe=subscribeWorldEvents(event=>{if(event.type==='skill_proposed'||event.type==='skill_saved'||event.type==='task_completed')void refresh();});
    return ()=>{cancelled=true;clearInterval(timer);unsubscribe();};
  },[]);
  return <section className="forum-work" aria-label="Live capabilities">
    <div className="eyebrow">CAPABILITIES</div>
    {!data?<p>Connecting to your agent registry and skills.</p>:<>
      {data.unavailable.length>0&&<p role="status">Could not load {data.unavailable.join(', ')}. Connect to your daemon to see current capabilities.</p>}
      {data.agents&&<><p>{data.agents.length} registered workers</p>{data.agents.map(agent=><div className="forum-job" key={agent.id}><b>{agent.name}</b><span>{agent.role}</span>{onManageAgent&&<Button colors={colors} flashSuccess={false} onClick={()=>onManageAgent(agent.id)}>Manage {agent.name}</Button>}</div>)}</>}
      {data.skills&&<><p>{data.skills.length} saved skills</p>{data.skills.slice(0,6).map(skill=><div className="forum-job" key={skill.id}><b>{skill.name}</b><span>{skill.status||'Saved'}</span></div>)}</>}
      {data.proposals&&<p>{data.proposals.length} skill proposals from observed work</p>}
    </>}
    <Button colors={colors} flashSuccess={false} onClick={onDevelop}>Develop a capability</Button>
    {onSkills&&<Button colors={colors} flashSuccess={false} onClick={onSkills}>Open Skills</Button>}
  </section>;
}
