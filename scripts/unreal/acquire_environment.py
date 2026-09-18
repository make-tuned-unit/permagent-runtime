"""Acquire CC0 Poly Haven sources for the native Unreal art pass, hash verified."""
import concurrent.futures, hashlib, json, subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'assets/world/forum/third-party'
OUT.mkdir(parents=True,exist_ok=True)
def api(asset):
    return json.loads(subprocess.check_output(['curl','-fLsS','--retry','2','https://api.polyhaven.com/files/'+asset]))
def download(item):
    path,record=item
    assert record['url'].startswith('https://dl.polyhaven.org/')
    path.parent.mkdir(parents=True,exist_ok=True)
    if not path.exists() or hashlib.md5(path.read_bytes()).hexdigest()!=record['md5']:
        subprocess.run(['curl','-fLsS','--retry','2',record['url'],'-o',str(path)],check=True)
    assert hashlib.md5(path.read_bytes()).hexdigest()==record['md5'],path
    return {'path':str(path.relative_to(ROOT)),**{k:record[k] for k in ('url','md5','size')}}
jobs=[]
for asset in ['white_sandstone_blocks_02','mossy_rock']:
    data=api(asset)
    for kind in ['Diffuse','nor_dx','Rough']:
        key=kind if kind in data else {'Diffuse':'diff','Rough':'rough'}[kind]
        record=data[key]['2k']['jpg']; jobs.append((OUT/asset/Path(record['url']).name,record))
for asset in ['jacaranda_tree','fern_02','rock_09']:
    data=api(asset); record=data['fbx']['2k']['fbx']
    assert record['size']<250_000_000
    jobs.append((OUT/asset/Path(record['url']).name,record))
    for relative,entry in record.get('include',{}).items():jobs.append((OUT/asset/relative,entry))
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:files=list(pool.map(download,jobs))
manifest={'license':'CC0-1.0','licenseUrl':'https://polyhaven.com/license','provider':'Poly Haven','files':files}
(ROOT/'docs/design/solar-forum/environment-sources.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('Verified',len(files),'environment source files;',sum(x['size'] for x in files),'bytes')
