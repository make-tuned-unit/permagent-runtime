"""Read-only contract check against the existing local daemon. Never submits work."""
import json, time, urllib.request, urllib.error
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
token=json.loads((Path.home()/'.permagent/secrets/daemon_token.json').read_text())['token']
contracts={
 '/api/agent/identity':lambda d:isinstance(d.get('first_name'),str),
 '/api/henry/status':lambda d:isinstance(d.get('current_state'),str),
 '/api/agents':lambda d:isinstance(d.get('agents'),list),
 '/api/goals/active':lambda d:isinstance(d.get('goals'),list),
 '/permagent/skills':lambda d:isinstance(d,list),
 '/permagent/skills/proposals':lambda d:isinstance(d,list),
}
results=[]
for path,validate in contracts.items():
 try:
  started=time.monotonic()
  request=urllib.request.Request('http://127.0.0.1:3001'+path,headers={'Authorization':'Bearer '+token})
  with urllib.request.urlopen(request,timeout=20) as response:
   data=json.load(response);results.append({'path':path,'status':response.status,'schemaMatches':validate(data),'elapsedSeconds':round(time.monotonic()-started,3)})
 except urllib.error.HTTPError as error:results.append({'path':path,'status':error.code,'schemaMatches':False})
 except Exception as error:results.append({'path':path,'error':type(error).__name__,'schemaMatches':False})
report={'scope':'Authenticated read-only endpoint contracts; no task execution or native telemetry bridge claimed','complete':all(r['schemaMatches'] for r in results),'endpoints':results}
(ROOT/'docs/design/solar-forum/runtime-contract.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
