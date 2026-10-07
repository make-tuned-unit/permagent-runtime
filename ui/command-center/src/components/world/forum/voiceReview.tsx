/** Manual Vite-only audio/renderer integration fixture. This entry is not in
 * the production build. It never opens a microphone or dispatches a message. */
import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ForumView } from './ForumView';
import { useCommandCenter } from '../../../lib/store';
import { space } from '../../../styles/tokens';
import '../../../index.css';

function VoiceReview() {
  const [context, setContext] = useState<AudioContext | null>(null);
  const [status, setStatus] = useState('Silent audio fixture · no microphone or agent request');
  useEffect(() => () => { void context?.close(); }, [context]);
  useEffect(() => () => useCommandCenter.getState().setVoiceEngine(null), []);
  async function play() {
    if(context) await context.close();
    const audio = new AudioContext(), analyser = audio.createAnalyser(), envelope = audio.createGain(), mute = audio.createGain();
    analyser.fftSize = 64; analyser.smoothingTimeConstant = .75; mute.gain.value = 0;
    envelope.connect(analyser); analyser.connect(mute); mute.connect(audio.destination);
    for(const frequency of [180,420,860]){const source=audio.createOscillator();source.type='sawtooth';source.frequency.value=frequency;source.connect(envelope);source.start();source.stop(audio.currentTime+18);}
    const now=audio.currentTime;envelope.gain.setValueAtTime(0,now);
    for(let i=0;i<6;i++){envelope.gain.setValueAtTime(0,now+i*3);envelope.gain.linearRampToValueAtTime(.4,now+i*3+.15);envelope.gain.setValueAtTime(.4,now+i*3+1.6);envelope.gain.linearRampToValueAtTime(0,now+i*3+1.8);}
    setContext(audio);
    const stop=()=>{void audio.close();useCommandCenter.getState().setVoiceEngine(null);setStatus('Stopped · mouth should rest');};
    useCommandCenter.getState().setVoiceEngine({state:'playing',handsFree:true,error:null,activate:()=>{},deactivate:stop,startRecording:()=>{},stopRecording:()=>{},interrupt:stop,getAnalyser:()=>analyser,getMicAnalyser:()=>null,setHandsFree:()=>{},wakeWord:{enabled:false,setEnabled:()=>{},active:false,phrase:null,gated:false}});
    setStatus('TEST AUDIO · mouth opens for 1.8s, rests for 1.2s · repeats six times');
    await audio.resume();
  }
  return <><ForumView/><div style={{position:'fixed',top:space.xl,right:space.xl,zIndex:9999,background:'#342340',color:'white',padding:space.xl,borderRadius:8,fontSize:12}}><b>DEVELOPMENT CHECK</b><p>{status}</p><button onClick={()=>void play()}>Play silent audio fixture</button><button onClick={()=>{void context?.close();useCommandCenter.getState().setVoiceEngine(null);setStatus('Stopped · mouth should rest');}}>Stop fixture</button></div></>;
}
if(import.meta.env.DEV)createRoot(document.getElementById('root')!).render(<VoiceReview/>);
