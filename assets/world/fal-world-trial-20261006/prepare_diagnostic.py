"""Keep original triangles intersecting two camera frusta; no geometry simplification.
Run with Blender's bundled Python (NumPy), without launching Blender itself.
Output is a diagnostic view subset, not a complete replacement world.
"""
from pathlib import Path
import json,hashlib,shutil
import numpy as np
HERE=Path(__file__).resolve().parent
DEST=HERE/'diagnostic-meshes';DEST.mkdir(exist_ok=True)
if shutil.disk_usage(HERE).free<11*1024**3:raise RuntimeError('Preserve disk budget')
# Upstream viewer uses Rx(-pi/2)Rz(-pi/2) and an origin camera facing -Z.
# In the original Z-up PLY coordinates that is origin facing -X, with right +Y.
views=[{'name':'origin','position':[0,0,0]},{'name':'offset','position':[0,1,0]}]
half_width=np.tan(np.deg2rad(75)/2);half_height=half_width*9/16
rows=[]
for source in sorted((HERE/'unpacked').glob('mesh_layer*.ply')):
    fields=[];vertex_count=face_count=None;section=''
    with source.open('rb') as f:
        for _ in range(100):
            line=f.readline().decode('ascii').strip();words=line.split()
            if words[:2]==['element','vertex']:vertex_count=int(words[2]);section='vertex'
            elif words[:2]==['element','face']:face_count=int(words[2]);section='face'
            elif words and words[0]=='property' and section=='vertex':fields.append((words[2],{'double':'<f8','uchar':'u1'}[words[1]]))
            elif line=='end_header':break
        offset=f.tell()
    dtype=np.dtype(fields);vertices=np.memmap(source,mode='r',dtype=dtype,offset=offset,shape=(vertex_count,))
    fdtype=np.dtype([('count','u1'),('indices','<u4',(3,))]);faces=np.memmap(source,mode='r',dtype=fdtype,offset=offset+vertex_count*dtype.itemsize,shape=(face_count,))
    selected=[];lo=np.full(3,np.inf);hi=np.full(3,-np.inf)
    for start in range(0,vertex_count,100000):
        v=vertices[start:start+100000];xyz=np.column_stack([v[x] for x in 'xyz'])
        if not np.isfinite(xyz).all():raise ValueError('Nonfinite coordinates')
        lo=np.minimum(lo,xyz.min(axis=0));hi=np.maximum(hi,xyz.max(axis=0))
    for start in range(0,face_count,50000):
        f=faces[start:start+50000]
        if not np.all(f['count']==3):raise ValueError('Only triangular PLY data supported')
        idx=f['indices'];v=vertices[idx];points=np.stack([v[x] for x in 'xyz'],axis=-1)
        visible=np.zeros(len(f),dtype=bool)
        for view in views:
            p=points-np.array(view['position']);depth=-p[:,:,0];right=p[:,:,1];up=p[:,:,2]
            outside=np.all(depth<.03,axis=1)|np.all(right>depth*half_width,axis=1)|np.all(right<-depth*half_width,axis=1)|np.all(up>depth*half_height,axis=1)|np.all(up<-depth*half_height,axis=1)
            visible|=~outside
        selected.append(np.array(idx[visible],copy=True))
    kept=np.concatenate(selected);del selected
    if len(kept)>2500000:raise ValueError('Diagnostic triangle budget exceeded')
    indices,inverse=np.unique(kept,return_inverse=True);del kept
    outdtype=np.dtype([(x,'<f4') for x in 'xyz']+[(x,'u1') for x in ['red','green','blue']]);out=np.empty(len(indices),dtype=outdtype)
    for field in outdtype.names:out[field]=vertices[field][indices]
    outfaces=np.empty(inverse.size//3,dtype=fdtype);outfaces['count']=3;outfaces['indices']=inverse.reshape(-1,3)
    target=DEST/source.name
    header=('ply\nformat binary_little_endian 1.0\nelement vertex '+str(len(out))+'\n'+''.join('property float '+x+'\n' for x in 'xyz')+''.join('property uchar '+x+'\n' for x in ['red','green','blue'])+'element face '+str(len(outfaces))+'\nproperty list uchar uint vertex_indices\nend_header\n').encode()
    with target.open('wb') as f:f.write(header);out.tofile(f);outfaces.tofile(f)
    rows.append({'source':source.name,'vertices':vertex_count,'triangles':face_count,'bounds':[lo.tolist(),hi.tolist()],'diagnostic_vertices':len(out),'diagnostic_triangles':len(outfaces),'diagnostic_bytes':target.stat().st_size})
    print(json.dumps(rows[-1]),flush=True)
    del vertices,faces,out,outfaces,indices,inverse
report={'layers':rows,'total_triangles':sum(r['triangles'] for r in rows),'views':views,'horizontal_fov_degrees':75,'method':'Conservative original-triangle frustum selection for two fixed views; positions reduced from float64 to float32 for rendering only, RGB retained. Not a full scene export or geometry simplification. Scale is provider units, not calibrated metres.','orientation_source':'https://github.com/Tencent-Hunyuan/HunyuanWorld-1.0/blob/main/modelviewer.html'}
(HERE/'mesh-inspection.json').write_text(json.dumps(report,indent=2)+'\n')
