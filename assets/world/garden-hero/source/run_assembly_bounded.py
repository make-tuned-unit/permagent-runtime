import subprocess,time,shutil,sys
from pathlib import Path
base=Path(__file__).resolve().parent.parent
assert shutil.disk_usage(base).free>=3*1024**3
with (base/'assembly-v3b.log').open('w') as log:
 p=subprocess.Popen([sys.executable,str(base/'source'/'assemble_kit_v3b.py')],stdout=log,stderr=subprocess.STDOUT);start=time.monotonic();peak=0;reason=None
 while p.poll() is None:
  rss=subprocess.run(['ps','-o','rss=','-p',str(p.pid)],capture_output=True,text=True).stdout.strip();kb=int(rss) if rss else 0;peak=max(peak,kb)
  if time.monotonic()-start>120:reason='120s cap'
  elif kb>512*1024:reason='RSS above512MiB'
  elif shutil.disk_usage(base).free<3*1024**3:reason='disk below3GiB'
  if reason:
   p.terminate()
   try:p.wait(timeout=3)
   except subprocess.TimeoutExpired:p.kill();p.wait()
   break
  time.sleep(.05)
 print({'exit':p.returncode,'seconds':round(time.monotonic()-start,3),'peak_rss_kib_sampled':peak,'stop_reason':reason,'free_gib_end':round(shutil.disk_usage(base).free/1024**3,2)})
