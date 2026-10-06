"""Extract only reviewed scene data from the existing world ZIP; never execute it."""
from pathlib import Path, PurePosixPath
import hashlib, json, shutil, stat, zipfile
HERE=Path(__file__).resolve().parent
TARGET=HERE/'unpacked'
with zipfile.ZipFile(HERE/'world.zip') as archive:
    infos=archive.infolist()
    if len(infos)>2000 or sum(i.file_size for i in infos)>2*1024**3:
        raise ValueError('Archive exceeds bounded review budget')
    if shutil.disk_usage(HERE).free<10*1024**3+sum(i.file_size for i in infos):
        raise RuntimeError('Extraction would cross the 10 GiB free-disk floor')
    selected=[]
    for info in infos:
        path=PurePosixPath(info.filename)
        if path.is_absolute() or '..' in path.parts or '\\' in info.filename or stat.S_ISLNK(info.external_attr>>16):
            raise ValueError('Unsafe archive member path/type')
        if not info.is_dir() and path.suffix.lower() in {'.ply','.glb','.png','.jpg','.jpeg','.obj','.mtl'}:
            selected.append(info)
    if not selected:raise ValueError('No supported scene data in archive')
    report=[]
    for info in selected:
        target=TARGET/info.filename
        if not target.resolve().is_relative_to(TARGET.resolve()):raise ValueError('Destination escapes review folder')
        target.parent.mkdir(parents=True,exist_ok=True)
        if not target.exists():
            with archive.open(info) as source,target.open('xb') as out:shutil.copyfileobj(source,out)
        if target.stat().st_size!=info.file_size:raise ValueError('Extracted size mismatch')
        with target.open('rb') as f:sha=hashlib.file_digest(f,'sha256').hexdigest()
        row={'path':str(target.relative_to(HERE)),'bytes':info.file_size,'sha256':sha}
        if target.suffix.lower()=='.ply':
            header=[]
            with target.open('rb') as f:
                for _ in range(100):
                    line=f.readline(4096).decode('ascii').strip();header.append(line)
                    if line=='end_header':break
                else:raise ValueError('No bounded PLY header')
            row['ply_header']=header
        report.append(row)
(HERE/'archive-inspection.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
