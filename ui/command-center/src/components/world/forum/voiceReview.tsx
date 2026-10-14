/** Manual Vite-only audio/renderer integration fixture. This entry is not in
 * the production build. It never opens a microphone or dispatches a message. */
import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ForumView } from './ForumView';
import { useCommandCenter } from '../../../lib/store';
import { space } from '../../../styles/tokens';
import '../../../index.css';
import { beginPlayback, clearPlayback, makeSegment } from '../../../lib/voicePlaybackClock';
import { FIXTURE_RATE, FIXTURE_TEXT, speechPcm } from './speechFixture';

function VoiceReview() {
  const [status, setStatus] = useState('Speech-timing fixture · no microphone, audio or agent request');
  useEffect(() => () => useCommandCenter.getState().setVoiceEngine(null), []);
  async function play() {
    // Real speech timing: a speech-shaped PCM fixture with the proportional
    // word timings the daemon sends, run through the same playback clock the
    // live mouth reads. Not an on/off tone. Nothing is audible.
    const sentences = [FIXTURE_TEXT, 'Maybe the board is big.', 'Yes, I can do that now.'];
    const stop = () => { clearPlayback(); useCommandCenter.getState().setVoiceEngine(null); setStatus('Stopped · mouth should rest'); };
    const engine = (state: 'playing' | 'ready') => ({state,handsFree:true,error:null,activate:()=>{},deactivate:stop,startRecording:()=>{},stopRecording:()=>{},interrupt:stop,getAnalyser:()=>null,getMicAnalyser:()=>null,setHandsFree:()=>{},wakeWord:{enabled:false,setEnabled:()=>{},active:false,phrase:null,gated:false}});
    setStatus('TEST SPEECH · three sentences with real word timing, then rest');
    let at = performance.now() + 300;
    for (const [i, text] of sentences.entries()) {
      const { samples, words, durationMs } = speechPcm(text);
      const seg = makeSegment({ segmentId: i, startedAtMs: at, text, words, samples, sampleRate: FIXTURE_RATE, durationMs });
      window.setTimeout(() => { useCommandCenter.getState().setVoiceEngine(engine('playing')); beginPlayback(seg); }, Math.max(0, at - performance.now()));
      at += durationMs + 600;
    }
    window.setTimeout(stop, Math.max(0, at - performance.now()));
  }
  return <><ForumView/><div style={{position:'fixed',top:space.xl,right:space.xl,zIndex:9999,background:'#342340',color:'white',padding:space.xl,borderRadius:8,fontSize:12}}><b>DEVELOPMENT CHECK</b><p>{status}</p><button onClick={()=>void play()}>Play speech fixture</button><button onClick={()=>{clearPlayback();useCommandCenter.getState().setVoiceEngine(null);setStatus('Stopped · mouth should rest');}}>Stop fixture</button></div></>;
}
if(import.meta.env.DEV)createRoot(document.getElementById('root')!).render(<VoiceReview/>);
