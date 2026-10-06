"""Fetch CC0 surface maps from official Poly Haven metadata, with verified HTTPS."""
from pathlib import Path
import hashlib, ipaddress, json, shutil, socket, subprocess, urllib.parse, urllib.request

HERE=Path(__file__).resolve().parent
def fetch(url, target, expected=None):
    host=urllib.parse.urlsplit(url).hostname
    if urllib.parse.urlsplit(url).scheme!='https' or host not in ['api.polyhaven.com','dl.polyhaven.org']:
        raise ValueError('Expected official HTTPS asset host')
    if shutil.disk_usage(HERE).free<10*1024**3+32*1024**2:raise RuntimeError('10 GiB disk guard')
    resolution=[]
    try:socket.getaddrinfo(host,443)
    except socket.gaierror:
        req=urllib.request.Request('https://1.1.1.1/dns-query?'+urllib.parse.urlencode({'name':host,'type':'A'}),headers={'accept':'application/dns-json'})
        with urllib.request.urlopen(req,timeout=15) as r:dns=json.load(r)
        ip=str(ipaddress.IPv4Address(next(x['data'] for x in dns['Answer'] if x['type']==1)))
        resolution=['--resolve',host+':443:'+ip]
    if not target.exists():
        partial=target.with_suffix(target.suffix+'.partial')
        subprocess.run(['/usr/bin/curl','--fail','--silent','--show-error','--proto','=https','--max-time','90','--max-filesize',str(expected['size'] if expected else 1024**2),*resolution,'--output',str(partial),url],check=True)
        partial.replace(target)
    content=target.read_bytes()
    if expected and (len(content)!=expected['size'] or hashlib.md5(content).hexdigest()!=expected['md5']):raise ValueError('Provider checksum/size mismatch')
    return {'url':url,'file':str(target.relative_to(HERE)),'bytes':len(content),'sha256':hashlib.sha256(content).hexdigest()}

if __name__=='__main__':
    ledger=[]
    for name,author,metres in [('monastery_stone_floor','Amal Kumar',1.8),('wood_floor','Dimitrios Savva',1.7)]:
        metadata=HERE/(name+'-files.json')
        fetch('https://api.polyhaven.com/files/'+name,metadata)
        data=json.loads(metadata.read_text())
        for channel in ['Diffuse','nor_gl','Rough']:
            entry=data[channel]['2k']['jpg'];target=HERE/'raw'/Path(urllib.parse.urlsplit(entry['url']).path).name
            ledger.append({**fetch(entry['url'],target,entry),'asset':name,'channel':channel,'author':author,'metres':metres,'license':'CC0','source':'https://polyhaven.com/a/'+name,'license_url':'https://polyhaven.com/license'})
    (HERE/'surface-provenance.json').write_text(json.dumps(ledger,indent=2)+'\n')
    print('Verified',len(ledger),'CC0 maps')
