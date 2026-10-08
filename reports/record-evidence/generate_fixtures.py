#!/usr/bin/env python3
"""Generate synthetic speech files only. Never plays sound or captures a mic."""
from pathlib import Path
import hashlib,json,shutil,subprocess,tempfile
ROOT=Path(__file__).resolve().parent
ASSETS=ROOT/'assets'
CUES=[
 ('mic_open','near',2,'Synthetic microphone test, amber seven. For Record QA, I will draft the launch checklist by Friday. The launch is not approved. Do not send any email.'),
 ('far_open','far',3,'Synthetic remote participant, cobalt nine. I will review the accessibility checklist. The old idea to buy equipment is cancelled. Do not create a purchase task.'),
 ('far_tail','far',49,'Cobalt nine final commitment: confirm the venue step free route.'),
 ('duplex_near_open','near',3,'Synthetic practice call, amber seven. I will draft the launch checklist by Friday. This is only a recording test.'),
 ('duplex_far_open','far',17,'Cobalt nine here. I will review the accessibility checklist. We have not approved the launch.'),
 ('duplex_near_mid','near',33,'We decided the practice demo is on Tuesday. The example budget is forty two dollars. Do not send email.'),
 ('duplex_far_mid','far',44,'The purchase idea is cancelled. We still need to ask which room has a step free entrance.'),
 ('duplex_overlap','near',48,'Amber seven agrees: no purchase and no launch approval.'),
 ('duplex_far_late','far',67,'Record QA is the only project for these practice notes. This is not a request to execute the tasks.'),
 ('duplex_near_late','near',82,'My typed note says accessibility and venue route matter. Keep that note verbatim.'),
 ('duplex_tail','far',96,'Cobalt nine: confirm the venue route.'),
]
SCENARIOS=[
 {'id':'mic-smoke','label':'Microphone smoke · 20 seconds','duration':20,'cue_ids':['mic_open'],'instructions':'Turn OFF Also record the other participants in Permagent. This fixture plays a synthetic near voice through the receiver speaker. A physical microphone can capture it acoustically. This is not isolated duplex capture.','expected':['amber seven','draft the launch checklist by Friday','launch not approved','no email']},
 {'id':'far','label':'Far-side practice call · 55 seconds','duration':55,'cue_ids':['far_open','far_tail'],'instructions':'Turn ON Also record the other participants. Keep the physical microphone quiet. Synthetic remote audio traverses WebRTC before playback. Check the final commitment at 49 seconds survives Stop & save.','expected':['cobalt nine','review accessibility checklist','purchase cancelled','confirm venue step-free route after 49 seconds']},
 {'id':'duplex','label':'Two-participant practice · 100 seconds','duration':100,'cue_ids':[c[0] for c in CUES if c[0].startswith('duplex_')],'instructions':'Two synthetic participants traverse WebRTC. Both audible outputs share this Mac system mix. Speaker playback may also enter the microphone: this is an echo/attribution stress test, not proof of isolated near/far capture. Use established separate audio routing for clean duplex acceptance.','expected':['draft launch checklist','review accessibility checklist','practice demo Tuesday','42 dollars fictional budget','no purchase / no email / no launch approval','confirm venue route at 96 seconds','typed note preserved']},
]

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
 for tool in ['say','ffmpeg','ffprobe']:
  if not shutil.which(tool):raise SystemExit(f'{tool} unavailable')
 voices=subprocess.check_output(['say','-v','?'],text=True)
 for voice in ['Samantha','Daniel']:
  if not any(line.startswith(voice+' ') for line in voices.splitlines()):raise SystemExit(f'Required installed voice unavailable: {voice}')
 ASSETS.mkdir(exist_ok=True)
 result=[]
 with tempfile.TemporaryDirectory(prefix='record-qa-speech-') as tmp:
  for name,side,start,script in CUES:
   voice='Samantha' if side=='near' else 'Daniel'
   aiff=Path(tmp)/(name+'.aiff'); wav=ASSETS/(name+'.wav')
   subprocess.run(['say','-v',voice,'-r','190','-o',str(aiff),script],check=True)
   subprocess.run(['ffmpeg','-nostdin','-hide_banner','-loglevel','error','-y','-i',str(aiff),'-ar','24000','-ac','1','-c:a','pcm_s16le',str(wav)],check=True)
   duration=float(subprocess.check_output(['ffprobe','-v','error','-show_entries','format=duration','-of','default=noprint_wrappers=1:nokey=1',str(wav)],text=True).strip())
   result.append({'id':name,'side':side,'at_seconds':start,'duration_seconds':duration,'end_seconds':round(start+duration,6),'voice':voice,'rate_wpm':190,'script':script,'url':'assets/'+wav.name,'sha256':sha(wav),'bytes':wav.stat().st_size,'sample_rate':24000,'channels':1,'encoding':'PCM signed 16-bit little-endian WAV'})
 for scenario in SCENARIOS:
  cues=[c for c in result if c['id'] in scenario['cue_ids']]
  if any(c['end_seconds']>scenario['duration'] for c in cues):raise RuntimeError(f'Speech overruns {scenario["id"]}: {cues}')
 manifest={'schema':1,'title':'Permagent synthetic Record regression fixture','generated_with':{'say':'macOS installed voices, file output only','ffmpeg':subprocess.check_output(['ffmpeg','-version'],text=True).splitlines()[0]},'privacy':'No microphone, camera, external server, account, STUN or TURN is used. Playback only starts after the operator clicks Start. Captions are expected fixture text, never transcription results.','scenarios':SCENARIOS,'cues':result,'typed_note':'accessibility + venue route matter; no launch approval, no purchase, no email','expected_card_policy':'Only catalogue and add stated commitments to the disposable Record QA project. Do not execute tasks, send emails or buy anything.'}
 (ROOT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 print(json.dumps({'cues':len(result),'audio_bytes':sum(c['bytes'] for c in result),'manifest_sha256':sha(ROOT/'manifest.json'),'playback':False},indent=2))
if __name__=='__main__':main()
