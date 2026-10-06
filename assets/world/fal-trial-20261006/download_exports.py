"""Download existing completed FAL GLBs; never submits a generation.

Use the normal resolver first. If unavailable, obtain a fresh public DNS answer
via verified HTTPS and give curl a request-local --resolve mapping. Certificate
and hostname checks remain enabled. No host, DNS, browser or security settings
are modified. Existing valid files are verified and reused.
"""
from pathlib import Path
import argparse, hashlib, ipaddress, json, shutil, socket, struct, subprocess, urllib.parse, urllib.request
from datetime import datetime, timezone

HERE = Path(__file__).resolve().parent
SOURCES = {
    'planter-hunyuan': 'planterHunyuanResult.json',
    'instrument-hunyuan': 'instrumentHunyuanResult.json',
    'bench-hunyuan': 'benchHunyuanResult.json',
}

def verify(path, expected):
    size = path.stat().st_size
    with path.open('rb') as handle:
        header = handle.read(12)
        if len(header) != 12 or struct.unpack('<4sII', header) != (b'glTF', 2, size):
            raise ValueError(f'Invalid GLB header or declared length: {path.name}')
        handle.seek(0)
        digest = hashlib.file_digest(handle, 'sha256').hexdigest()
    if expected is not None and size != expected:
        raise ValueError(f'Unexpected file size: {size} != {expected}')
    return {'bytes': size, 'sha256': digest, 'glb_version': 2}

def download(name):
    receipt = json.loads((HERE / SOURCES[name]).read_text())
    if receipt['status'] != 'completed':
        raise ValueError('Requires a completed-job receipt')
    media = receipt['result']['model_glb']
    url = urllib.parse.urlsplit(media['url'])
    if url.scheme != 'https' or not url.hostname or not url.hostname.endswith('.fal.media') or url.username or url.password or url.port not in (None, 443):
        raise ValueError('Expected a public HTTPS FAL media URL')
    target = HERE / 'raw' / f'{name}.glb'
    target.parent.mkdir(exist_ok=True)
    result = {'asset': name, 'request_id': receipt['request_id'], 'url': media['url']}
    if target.exists():
        return {**result, **verify(target, media.get('file_size')), 'acquisition': 'reused verified local file'}
    maximum_bytes = media.get('file_size') or 128 * 1024**2
    if shutil.disk_usage(HERE).free < 10 * 1024**3 + maximum_bytes:
        raise RuntimeError('Download would cross the 10 GiB free-disk floor')
    resolve = []
    try:
        socket.getaddrinfo(url.hostname, 443)
        acquisition = 'system DNS; verified HTTPS'
    except socket.gaierror:
        query = urllib.parse.urlencode({'name': url.hostname, 'type': 'A'})
        request = urllib.request.Request('https://1.1.1.1/dns-query?' + query, headers={'accept': 'application/dns-json'})
        with urllib.request.urlopen(request, timeout=15) as response:
            dns = json.load(response)
        answers = [row['data'] for row in dns.get('Answer', []) if row.get('type') == 1]
        if dns.get('Status') != 0 or not answers:
            raise ValueError('Public DNS lookup did not return an A record')
        address = str(ipaddress.IPv4Address(answers[0]))
        resolve = ['--resolve', f'{url.hostname}:443:{address}']
        acquisition = 'fresh HTTPS public DNS answer; request-local resolution; verified HTTPS'
    partial = target.with_suffix('.glb.partial')
    subprocess.run(['/usr/bin/curl', '--fail', '--silent', '--show-error', '--proto', '=https',
                    '--connect-timeout', '15', '--max-time', '180', '--max-filesize', str(maximum_bytes), *resolve,
                    '--output', str(partial), media['url']], check=True)
    checked = verify(partial, media.get('file_size'))
    partial.replace(target)
    return {**result, **checked, 'acquisition': acquisition}

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('assets', nargs='*', choices=list(SOURCES), default=list(SOURCES))
    args = parser.parse_args()
    ledger_path = HERE / 'download-receipts.json'
    ledger = json.loads(ledger_path.read_text()) if ledger_path.exists() else {}
    for name in args.assets:
        result = download(name)
        if name in ledger and result['acquisition']=='reused verified local file':
            if ledger[name]['sha256']!=result['sha256'] or ledger[name]['request_id']!=result['request_id']:
                raise ValueError('Existing local file does not match its acquisition ledger')
            result={**ledger[name], 'revalidated_at':datetime.now(timezone.utc).isoformat()}
        else:
            result['downloaded_at']=datetime.now(timezone.utc).isoformat()
        ledger[name] = result
        ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
        print(json.dumps(result), flush=True)
