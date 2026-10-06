"""Acquire completed panorama/world outputs through verified HTTPS only."""
from pathlib import Path
import argparse,hashlib,ipaddress,json,socket,struct,subprocess,urllib.parse,urllib.request,zipfile,shutil
HERE=Path(__file__).resolve().parent

def fetch(kind):
    receipt_path=HERE/(kind+'-result.json')
    private=HERE/('.'+kind+'-result-private.json')
    if private.exists():receipt_path=private
    receipt=json.loads(receipt_path.read_text())
    if receipt['status']!='completed':raise ValueError('Job must be completed')
    media=receipt['result']['image' if kind=='panorama' else 'world_file']
    url=urllib.parse.urlsplit(media['url'])
    fal_host=url.hostname and url.hostname.endswith('.fal.media')
    fal_bucket=url.hostname=='storage.googleapis.com' and url.path.startswith('/fal_file_storage/')
    if url.scheme!='https' or not (fal_host or fal_bucket) or url.username or url.password or url.port not in (None,443):raise ValueError('Expected FAL output storage URL')
    suffix='.png' if kind=='panorama' else Path(media.get('file_name') or url.path).suffix
    if suffix not in ['.png','.zip','.glb','.ply']:raise ValueError('Inspect unfamiliar export type before downloading')
    target=HERE/(kind+suffix);partial=target.with_suffix(target.suffix+'.partial')
    if not target.exists():
        maximum=media.get('file_size') or 512*1024**2
        if maximum>512*1024**2:raise ValueError('Inspect export larger than 512 MiB before acquiring')
        if shutil.disk_usage(HERE).free<10*1024**3+maximum:raise RuntimeError('Preserve the 10 GiB free-disk floor')
        resolve=[]
        try:socket.getaddrinfo(url.hostname,443)
        except socket.gaierror:
            request=urllib.request.Request('https://1.1.1.1/dns-query?'+urllib.parse.urlencode({'name':url.hostname,'type':'A'}),headers={'accept':'application/dns-json'})
            with urllib.request.urlopen(request,timeout=15) as f:dns=json.load(f)
            ip=str(ipaddress.IPv4Address(next(a['data'] for a in dns['Answer'] if a['type']==1)))
            resolve=['--resolve',url.hostname+':443:'+ip]
        subprocess.run(['/usr/bin/curl','--fail','--silent','--show-error','--proto','=https','--connect-timeout','15','--max-time','240','--max-filesize',str(maximum),*resolve,'--output',str(partial),'--config','-'],input='url = '+json.dumps(media['url'])+'\n',text=True,check=True)
        if media.get('file_size') is not None and partial.stat().st_size!=media['file_size']:raise ValueError('Size mismatch')
        with partial.open('rb') as f:magic=f.read(8)
        if suffix=='.png' and magic!=b'\x89PNG\r\n\x1a\n':raise ValueError('Invalid PNG')
        if suffix=='.zip' and not zipfile.is_zipfile(partial):raise ValueError('Invalid ZIP')
        partial.replace(target)
    if media.get('file_size') is not None and target.stat().st_size!=media['file_size']:raise ValueError('Local size mismatch')
    with target.open('rb') as f:header=f.read(12)
    if suffix=='.png' and header[:8]!=b'\x89PNG\r\n\x1a\n':raise ValueError('Invalid PNG')
    if suffix=='.zip' and not zipfile.is_zipfile(target):raise ValueError('Invalid ZIP')
    if suffix=='.glb' and struct.unpack('<4sII',header)!=(b'glTF',2,target.stat().st_size):raise ValueError('Invalid GLB')
    if suffix=='.ply' and not header.startswith(b'ply\n'):raise ValueError('Invalid PLY')
    with target.open('rb') as f:sha=hashlib.file_digest(f,'sha256').hexdigest()
    ledger=HERE/(kind+'-download.json')
    if ledger.exists() and json.loads(ledger.read_text())['sha256']!=sha:raise ValueError('Existing acquisition checksum mismatch')
    report={'path':target.name,'request_id':receipt['request_id'],'url':urllib.parse.urlunsplit(url._replace(query='')),'bytes':target.stat().st_size,'sha256':sha,'method':'Verified HTTPS; public DNS answer provided to curl per request when system DNS fails; signed queries omitted from ledger'}
    if suffix=='.zip':
        with zipfile.ZipFile(target) as z:report['archive_members']=[{'path':i.filename,'bytes':i.file_size} for i in z.infolist()]
    (HERE/(kind+'-download.json')).write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('kind',choices=['panorama','world']);fetch(p.parse_args().kind)
