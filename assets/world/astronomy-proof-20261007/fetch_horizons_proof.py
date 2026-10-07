"""Bounded read-only four-body proof: exact JPL states, no runtime integration."""
from pathlib import Path
import json, urllib.parse, urllib.request, hashlib, math

dest=Path(__file__).resolve().parent
dest.mkdir(exist_ok=True)
params={'format':'json','OBJ_DATA':'YES','MAKE_EPHEM':'YES','EPHEM_TYPE':'VECTORS',
 'CENTER':'500@0','REF_SYSTEM':'ICRF','REF_PLANE':'FRAME','TIME_TYPE':'TDB',
 'START_TIME':'2026-10-07 00:00','STOP_TIME':'2026-10-08 00:00','STEP_SIZE':'30 min',
 'OUT_UNITS':'KM-S','VEC_TABLE':'2','VEC_CORR':'NONE','CSV_FORMAT':'YES','VEC_LABELS':'NO'}
records={}
for name,body in [('sun','10'),('earth','399'),('moon','301'),('jupiter','599')]:
    path=dest/(name+'-horizons.json')
    p={**params,'COMMAND':body}
    url='https://ssd.jpl.nasa.gov/api/horizons.api?'+urllib.parse.urlencode({k:v if k=='format' else "'"+v+"'" for k,v in p.items()})
    if path.exists(): raw=path.read_bytes()
    else:
        with urllib.request.urlopen(url,timeout=45) as response: raw=response.read(1000000)
    response=json.loads(raw)
    if response.get('error'): raise ValueError(response['error'])
    value=response['result']
    if '$$SOE' not in value or '$$EOE' not in value or 'ICRF' not in value: raise ValueError('Missing vectors/ICRF')
    path.write_bytes(raw)
    rows=[]
    for line in value.split('$$SOE')[1].split('$$EOE')[0].strip().splitlines():
        cells=[s.strip() for s in line.split(',')]
        jd=float(cells[0]); nums=[float(s) for s in cells[2:8]]
        if len(nums)!=6 or not all(math.isfinite(n) for n in nums): raise ValueError('Bad state')
        rows.append([jd,*nums])
    if len(rows)!=49: raise ValueError(f'Unexpected rows {len(rows)}')
    # Hourly samples are interpolation knots; half-hour direct JPL states are independent checks.
    errors=[]
    for i in range(1,len(rows)-1,2):
        a,b,c=rows[i-1],rows[i],rows[i+1]
        dt=(c[0]-a[0])*86400
        interpolated=[.5*a[j]+.125*dt*a[j+3]+.5*c[j]-.125*dt*c[j+3] for j in range(1,4)]
        errors.append(math.sqrt(sum((interpolated[j]-b[j+1])**2 for j in range(3))))
    records[name]={'body_id':body,'query':p,'source_url':url,'sha256':hashlib.sha256(raw).hexdigest(),
      'rows':rows,'max_hourly_hermite_midpoint_error_km':max(errors),
      'source_header':value.split('$$SOE')[0]}
    print(json.dumps({'body':name,'rows':len(rows),'max_midpoint_error_km':max(errors)}),flush=True)
(dest/'proof.json').write_text(json.dumps({'retrieved':'2026-10-07','origin':'Solar System Barycenter (0)',
 'frame':'ICRF Cartesian; equatorial frame plane','time_scale':'TDB','distance':'km','velocity':'km/s',
 'corrections':'NONE: geometric, no light-time/aberration','window':'2026-10-07 through 2026-10-08 TDB',
 'coverage':'Four-body acquisition proof only, not complete catalog or runtime; error is interpolation vs JPL samples, not ephemeris uncertainty',
 'bodies':records},indent=2)+'\n')
