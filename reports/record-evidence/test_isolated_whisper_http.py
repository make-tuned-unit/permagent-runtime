#!/usr/bin/env python3
"""Direct synthetic WAV→Whisper API checks on isolated 3002. No audio playback/capture."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.request
import uuid
import wave
import isolated_daemon as isolated

HERE = Path(__file__).resolve().parent
# Preserve semantic cues without requiring punctuation/capitalization or identical ASR prose.
RULES = {
    'mic_open': {'commitment': r'checklist.*friday', 'launch_negation': r'(?:not|never|isn.t)\s+approved',
                 'no_email': r'(?:do not|don.t|never|no)\s+(?:send\s+)?(?:any\s+)?e-?mail'},
    'far_open': {'commitment': r'accessibility\s+checklist', 'cancelled_purchase': r'cancel[le]+d',
                 'no_purchase_task': r'(?:do not|don.t|never)\s+create\s+(?:a\s+)?purchase\s+task'},
    'far_tail': {'final_commitment': r'final\s+commitment', 'venue_tail': r'venue\s+step[ -]?free\s+route'},
}


def evaluate(cue_id, text):
    normalized = text.lower().replace('\u2019', "'")
    return {label: bool(re.search(pattern, normalized, re.S)) for label, pattern in RULES[cue_id].items()}


def upload(path):
    boundary = 'recordqa' + uuid.uuid4().hex
    body = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="synthetic.wav"\r\n'
            'Content-Type: audio/wav\r\n\r\n').encode() + path.read_bytes() + f'\r\n--{boundary}--\r\n'.encode()
    req = urllib.request.Request(isolated.URL + '/api/dictation/transcribe', data=body,
        headers={'Authorization': 'Bearer ' + isolated.token(), 'Content-Type': 'multipart/form-data; boundary=' + boundary})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(req, timeout=240) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise RuntimeError(f'Isolated Whisper returned HTTP {error.code}') from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true', help='Required to resample fixtures and call isolated Whisper')
    args = parser.parse_args()
    if not args.execute:
        print(json.dumps({'executeRequired': True, 'url': isolated.URL, 'cues': list(RULES),
                          'method': 'direct synthetic WAV upload; NOT native capture'}))
        return
    isolated.manifest()
    isolated.artifact(isolated.ROOT / 'models' / isolated.WHISPER_NAME, isolated.WHISPER_SHA)
    manifest = json.loads((HERE / 'manifest.json').read_text())
    cues = {cue['id']: cue for cue in manifest['cues']}
    stamp = time.strftime('%Y%m%d-%H%M%S', time.gmtime())
    output = HERE / ('isolated-whisper-' + stamp)
    output.mkdir(mode=0o700)
    evidence = {'method': 'direct synthetic 16k mono WAV upload, NOT native capture', 'url': isolated.URL,
                'modelSha256': isolated.WHISPER_SHA, 'tokenLogged': False, 'results': []}
    for cue_id in RULES:
        cue = cues[cue_id]
        row = {'cue': cue_id, 'expected': cue['script'], 'sourceSha256': cue['sha256']}
        evidence['results'].append(row)
        try:
            source = (HERE / cue['url']).resolve()
            if not source.is_relative_to((HERE / 'assets').resolve()):
                raise RuntimeError('Fixture escapes owned assets')
            isolated.artifact(source, cue['sha256'])
            target = output / (cue_id + '.wav')
            subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(source), '-ac', '1', '-ar', '16000',
                            '-c:a', 'pcm_s16le', str(target)], check=True, timeout=30, stdout=subprocess.DEVNULL,
                           stderr=subprocess.PIPE)
            with wave.open(str(target), 'rb') as wav:
                if (wav.getnchannels(), wav.getframerate(), wav.getsampwidth()) != (1, 16000, 2):
                    raise RuntimeError('Resampled WAV format mismatch')
                row['durationSeconds'] = wav.getnframes() / wav.getframerate()
            row['uploadedSha256'] = isolated.digest(target)
            started = time.monotonic()
            result = upload(target)
            row['elapsedSeconds'] = round(time.monotonic() - started, 3)
            row['observed'] = result.get('text', '')
            row['checks'] = evaluate(cue_id, row['observed'])
            row['passed'] = all(row['checks'].values())
        except Exception as error:
            row.update({'passed': False, 'error': str(error)})
    evidence['passed'] = all(row['passed'] for row in evidence['results'])
    path = output / 'evidence.json'
    isolated.write_json(path, evidence)
    print(json.dumps({'passed': evidence['passed'], 'evidence': str(path)}))
    raise SystemExit(0 if evidence['passed'] else 1)

if __name__ == '__main__': main()
