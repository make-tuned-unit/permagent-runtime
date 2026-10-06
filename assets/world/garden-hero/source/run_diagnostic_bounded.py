import subprocess,time,shutil,sys,os
from pathlib import Path
base=Path(__file__).resolve().parent.parent
if shutil.disk_usage(base).free<3*1024**3:raise SystemExit('NOT_LAUNCHED: less than 3GiB free')
with (base/'robot-diagnostic.log').open('w') as log:
 script=sys.argv[1] if len(sys.argv)>1 else 'robot_diagnostic.py'
 p=subprocess.Popen(['/Applications/Blender.app/Contents/MacOS/Blender','--background','--threads','1','--python-exit-code','1','--python',str(base/'source'/script)],stdout=log,stderr=subprocess.STDOUT)
 start=time.monotonic();peak=0;reason=None
 while p.poll() is None:
  rss=subprocess.run(['ps','-o','rss=','-p',str(p.pid)],capture_output=True,text=True).stdout.strip()
  kb=int(rss) if rss else 0;peak=max(peak,kb)
  limit=int(os.environ.get('WORLD_JOB_LIMIT_SECONDS','90'))
  if time.monotonic()-start>limit:reason=str(limit)+'s cap'
  elif shutil.disk_usage(base).free<3*1024**3:reason='free disk below3GiB'
  elif kb>1024**2:reason='RSS above1GiB'
  if reason:
   p.terminate()
   try:p.wait(timeout=5)
   except subprocess.TimeoutExpired:p.kill();p.wait()
   break
  time.sleep(.5)
 print({'pid':p.pid,'exit':p.returncode,'stop_reason':reason,'seconds':round(time.monotonic()-start,2),'peak_rss_kib':peak,'free_gib':round(shutil.disk_usage(base).free/1024**3,2),'preview':str(base/'previews'/'robot-export-diagnostic-512.png')})
