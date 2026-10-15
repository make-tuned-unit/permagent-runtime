import { useEffect, useState } from 'react';
import { useCommandCenter } from '../../../lib/store';
import { useHabitatTravel } from './habitatMotion';
import { stripVoiceDeliveryTags } from '../../../lib/voiceDeliveryTags';

export function OrchestratorConversation({ name, onLeave }: { name: string; onLeave: () => void }) {
  const engine = useCommandCenter(s => s.voiceEngine);
  const conversation = useCommandCenter(s => s.voiceConversation);
  const messages = useCommandCenter(s => s.chatMessages);
  const travel = useHabitatTravel('henry');
  const [transcript, setTranscript] = useState(false);
  const [startError, setStartError] = useState('');
  const active = !!engine && (engine.handsFree || !['idle', 'error'].includes(engine.state));
  const status = engine?.state === 'playing' ? 'Speaking' : engine?.state === 'recording' ? 'Listening to you' : engine?.state === 'processing' ? 'Thinking' : engine?.state === 'connecting' ? 'Connecting voice…' : active ? conversation?.wakeHint || 'Ready to listen' : 'A moment, face to face';
  const end = () => { if (engine) { void engine.setHandsFree(false); engine.deactivate(); } };
  useEffect(() => { const key=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();end();onLeave();}};window.addEventListener('keydown',key);return()=>window.removeEventListener('keydown',key); }, [engine,onLeave]);
  return <section className="orchestrator-conversation" aria-label={`One on one with ${name}`}>
    <div className="orchestrator-conversation-heading"><span>AT THE COMMONS TABLE</span><button aria-label={active?'Leave table and end voice chat':'Leave table'} onClick={() => { end(); onLeave(); }}>Leave table <span aria-hidden="true">↗</span></button></div>
    <h2>{name}<span>Your orchestrator</span></h2>
    <p className="orchestrator-voice-state" role="status"><i data-speaking={engine?.state === 'playing'} />{travel?.moving ? `${name} is joining you…` : travel?.label.startsWith('No clear') || travel?.label.startsWith('Route occupied') ? travel.label : status}</p>
    <div className="orchestrator-conversation-actions">
      {active ? <><button onClick={end}>End voice chat</button>{(engine?.state === 'playing' || engine?.state === 'processing') && <button onClick={() => engine.interrupt()}>Let me speak</button>}</> : <button className="habitat-primary" disabled={!engine} onClick={() => { setStartError(''); void Promise.resolve(engine?.setHandsFree(true)).catch(e => setStartError(e instanceof Error ? e.message : String(e))); }}>Start voice chat</button>}
      <button aria-expanded={transcript} onClick={() => setTranscript(v => !v)}>{transcript ? 'Hide conversation' : 'Read conversation'}</button>
    </div>
    {!engine && <small>Voice is available in the Permagent app.</small>}
    {(engine?.error || startError) && <p role="alert">{engine?.error || startError}</p>}
    {transcript && <div className="orchestrator-transcript" aria-live="polite">{messages.length ? messages.slice(-8).map(m => <div key={m.id}><b>{m.role === 'user' ? 'You' : name}</b><p>{stripVoiceDeliveryTags(m.content)}</p></div>) : <p>Your conversation will appear here.</p>}</div>}
  </section>;
}
