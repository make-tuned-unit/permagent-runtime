#!/usr/bin/env python3
"""Opt-in Chrome-only helper capture, with no new TCC request or device changes.
Capture and isolated Whisper transcription are separate explicit commands.
"""
import argparse
from array import array
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import threading
import time
import uuid
import wave
import isolated_daemon as isolated
import test_isolated_whisper_http as whisper

BUNDLE = 'com.google.Chrome'
SCOPE = 'PERMAGENT_AUDIOCAP_SCOPED_SOURCE_V1'
PERMISSION = 'PERMAGENT_AUDIOCAP_EXISTING_PERMISSION_V1'
CAPTURES = isolated.ROOT / 'capture-evidence'


def helper_check(path, expected):
    evidence = isolated.artifact(path, expected)
    path = Path(path)
    if not path.is_relative_to(isolated.TARGET) or not os.access(path, os.X_OK):
        raise RuntimeError('Only an executable helper in the owned review target is allowed')
    data = path.read_bytes()
    if any(marker.encode() not in data for marker in [SCOPE, PERMISSION]):
        raise RuntimeError('Helper lacks exact-scope/nonprompting-preflight capability; do not run old helper with unknown flags')
    subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(path)], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    return evidence


def chunk_info(path, directory):
    path = Path(path)
    if path.parent != directory or path.resolve() != path or not re.fullmatch(r'sysaudio_[0-9]+\.wav', path.name):
        raise RuntimeError('Unexpected/noncanonical helper chunk path')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as handle:
        if not stat.S_ISREG(os.fstat(handle.fileno()).st_mode):
            raise RuntimeError('Helper chunk must be a regular file')
        data = handle.read(10 * 1024 * 1024 + 1)
    if len(data) > 10 * 1024 * 1024:
        raise RuntimeError('Chunk exceeds10MiB safety bound')
    with wave.open(io.BytesIO(data), 'rb') as wav:
        if (wav.getnchannels(), wav.getframerate(), wav.getsampwidth()) != (1, 16000, 2):
            raise RuntimeError('Chunk is not16kHz mono PCM16 WAV')
        frames = wav.getnframes()
        values = array('h', wav.readframes(frames))
        if __import__('sys').byteorder != 'little': values.byteswap()
    return {'path': str(path), 'sha256': hashlib.sha256(data).hexdigest(), 'frames': frames,
            'durationSeconds': frames / 16000, 'peak': max(map(abs, values), default=0),
            'rms': math.sqrt(sum(value * value for value in values) / len(values)) if values else 0}


def classify_pcm_diagnostics(metrics):
    if not metrics or metrics.get('phase') != 'final':
        return 'incomplete_diagnostics'
    raw = metrics.get('rawChannels', [])
    if not raw or not all(channel.get('sampleCount', 0) > 0 for channel in raw):
        return 'no_input_samples'
    if not any(channel.get('peak', 0) > 0 for channel in raw):
        return 'incoming_pcm_silent'
    if metrics.get('downmixed', {}).get('peak', 0) == 0:
        return 'downmix_cancellation'
    if metrics.get('resampledFloat', {}).get('peak', 0) == 0:
        return 'resampling_loss'
    if metrics.get('quantizedInt16', {}).get('peak', 0) == 0:
        return 'quantization_loss'
    return 'nonzero_pcm_reached_wav'



def capture_protocol_errors(evidence):
    errors = []
    if any(entry['line'].startswith('ERROR ') for entry in evidence.get('stderr', [])):
        errors.append('Helper emitted an ERROR; partial output cannot prove complete capture')
    chunks = evidence.get('chunks', [])
    indices = []
    for chunk in chunks:
        match = re.fullmatch(r'sysaudio_([0-9]+)\.wav', Path(chunk['path']).name)
        indices.append(int(match.group(1)) if match else None)
    if indices != list(range(len(chunks))):
        errors.append('Helper chunk numbering is missing, duplicated, or out of order')
    # This runner always requests55..80s with45s chunks. A tail alone can look
    # valid after a failed first write because the helper increments its index
    # before attempting the write; require the full chunk as well.
    if not any(abs(chunk['durationSeconds'] - 45) <= 1 / 16000 for chunk in chunks):
        errors.append('Capture did not retain a complete45-second chunk')
    return errors


def capture_passed(evidence):
    return (not evidence.get('error') and not capture_protocol_errors(evidence) and evidence['ready'] and
        evidence['sourceVerified'] and evidence['readersJoined'] and evidence['exitCode'] == 0 and
        bool(evidence['chunks']) and any(c['peak'] > 0 for c in evidence['chunks']) and
        any(c['afterStopSignal'] and 0 < c['durationSeconds'] < 45 for c in evidence['chunks']))


def run(args):
    isolated.manifest()
    helper = helper_check(args.helper, args.sha256)
    if args.pcm_diagnostics and b'PCM_DIAGNOSTIC_V1' not in Path(args.helper).read_bytes():
        raise RuntimeError('Helper lacks optional PCM diagnostics; do not pass ignored flags to old helper')
    # This new mode exits before ScreenCaptureKit and never calls a request API.
    preflight = subprocess.run([args.helper, '--preflight-only'], capture_output=True, text=True, timeout=10)
    if preflight.returncode != 0 or f'PREFLIGHT {PERMISSION} granted' not in preflight.stderr:
        raise RuntimeError('Helper has no existing screen-capture permission. No capture or permission request attempted.')
    if CAPTURES.resolve() != CAPTURES:
        raise RuntimeError('Capture evidence directory must be canonical')
    CAPTURES.mkdir(mode=0o700, exist_ok=True)
    directory = CAPTURES / ('chrome-' + uuid.uuid4().hex)
    directory.mkdir(mode=0o700)
    chunks = directory / 'chunks'
    chunks.mkdir(mode=0o700)
    command = [args.helper, '--out', str(chunks), '--chunk-seconds', '45', '--parent-pid', str(os.getpid()),
               '--exclude-bundle-id', 'ai.permagent.record-review-20261007',
               '--include-bundle-id', BUNDLE, '--require-existing-permission']
    if args.pcm_diagnostics:
        command.append('--pcm-diagnostics')
    evidence = {'method': 'actual Chrome-scoped ScreenCaptureKit helper capture; NOT Record UI/Tauri IPC',
                'helper': helper, 'bundleId': BUNDLE, 'command': command, 'chunkSeconds': 45,
                'captureSecondsAfterReady': args.seconds, 'preflightGranted': True,
                'pcmDiagnosticsEnabled': args.pcm_diagnostics,
                'sourceVerified': False, 'ready': False, 'chunks': [], 'stderr': [], 'emitted': []}
    ready = threading.Event()
    lock = threading.Lock()
    stopped = False
    stop_time = None
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, start_new_session=True, env={'PATH': '/usr/bin:/bin', 'HOME': os.environ['HOME']})
    started = time.monotonic()
    def read_stderr():
        for raw in proc.stderr:
            line = raw.rstrip('\n')[:2000]
            with lock:
                evidence['stderr'].append({'elapsed': time.monotonic() - started, 'line': line})
                if line == f'SOURCE {SCOPE} application {BUNDLE}': evidence['sourceVerified'] = True
                if line.startswith('READY ') and evidence['sourceVerified']:
                    evidence['ready'] = True
                    ready.set()
    def read_stdout():
        for line in proc.stdout:
            with lock:
                evidence['emitted'].append({'path': line.rstrip('\n'), 'elapsed': time.monotonic() - started,
                                            'afterStopSignal': stop_time is not None})
    readers = [threading.Thread(target=read_stderr), threading.Thread(target=read_stdout)]
    for thread in readers: thread.start()
    def signal_stop(_signum, _frame):
        nonlocal stopped
        stopped = True
    signal.signal(signal.SIGINT, signal_stop)
    signal.signal(signal.SIGTERM, signal_stop)
    try:
        while not ready.wait(.2):
            if proc.poll() is not None or stopped or time.monotonic() - started > 30:
                raise RuntimeError('Scoped helper did not reach READY within30s; no permission retry/fallback attempted')
        print(json.dumps({'ready': True, 'bundleId': BUNDLE, 'runDirectory': str(directory),
                          'next': 'Parent CUA: start far55s WebRTC fixture within10s'}), flush=True)
        until = time.monotonic() + args.seconds
        while proc.poll() is None and not stopped and time.monotonic() < until:
            time.sleep(.2)
        if proc.poll() is not None:
            raise RuntimeError(f'Capture helper exited early with code{proc.returncode}')
    except Exception as error:
        evidence['error'] = str(error)
    finally:
        stop_time = time.monotonic()
        evidence['stopSignalElapsed'] = stop_time - started
        isolated.stop_owned_group(proc)
        for thread in readers: thread.join(timeout=5)
        evidence['readersJoined'] = all(not thread.is_alive() for thread in readers)
        evidence['exitCode'] = proc.returncode
        seen = set()
        try:
            for emitted in evidence['emitted']:
                if emitted['path'] in seen: raise RuntimeError('Helper emitted a duplicate chunk')
                seen.add(emitted['path'])
                evidence['chunks'].append({**chunk_info(emitted['path'], chunks), **emitted})
            observed = set(str(p) for p in chunks.glob('*.wav'))
            if observed != seen: raise RuntimeError('Chunk files do not match emitted protocol paths')
        except Exception as error:
            evidence['error'] = str(error)
        if args.pcm_diagnostics:
            for entry in evidence['stderr']:
                line = entry['line']
                try:
                    if line.startswith('PCM_FORMAT_V1 '):
                        evidence['pcmFormat'] = json.loads(line[len('PCM_FORMAT_V1 '):])
                    if line.startswith('PCM_DIAGNOSTIC_V1 '):
                        metrics = json.loads(line[len('PCM_DIAGNOSTIC_V1 '):])
                        if metrics.get('phase') == 'final': evidence['pcmDiagnostics'] = metrics
                except ValueError:
                    evidence['error'] = 'Malformed PCM diagnostic protocol JSON'
            evidence['pcmDiagnosis'] = classify_pcm_diagnostics(evidence.get('pcmDiagnostics'))
            if evidence['pcmDiagnosis'] == 'incomplete_diagnostics':
                evidence['error'] = 'PCM diagnostic helper did not emit its final amplitude receipt'
        evidence['protocolErrors'] = capture_protocol_errors(evidence)
        evidence['capturePassed'] = capture_passed(evidence)
        isolated.write_json(directory / 'capture.json', evidence)
    print(json.dumps({'capturePassed': evidence['capturePassed'], 'evidence': str(directory / 'capture.json')}))
    return evidence['capturePassed']


def transcribe(args):
    isolated.manifest()
    directory = Path(args.run_directory)
    if directory.resolve() != directory or directory.parent != CAPTURES or not directory.is_dir():
        raise RuntimeError('Only a canonical owned capture run can be transcribed')
    path = directory / 'capture.json'
    if path.resolve() != path: raise RuntimeError('Capture metadata symlink forbidden')
    evidence = json.loads(path.read_text())
    if not evidence.get('capturePassed') or evidence.get('bundleId') != BUNDLE:
        raise RuntimeError('Capture did not pass scoped protocol/stop-tail checks')
    helper_check(evidence['helper']['path'], evidence['helper']['sha256'])
    isolated.artifact(isolated.ROOT / 'models' / isolated.WHISPER_NAME, isolated.WHISPER_SHA)
    result = {'method': 'actual scoped helper WAV→isolated Whisper; NOT Record UI/Tauri IPC',
              'sourceCapture': str(path), 'url': isolated.URL, 'chunks': [], 'tokenLogged': False}
    output = directory / ('whisper-' + uuid.uuid4().hex + '.json')
    try:
        for chunk in evidence['chunks']:
            actual = chunk_info(chunk['path'], directory / 'chunks')
            if actual['sha256'] != chunk['sha256']: raise RuntimeError('Captured chunk changed before transcription')
            response = whisper.upload(Path(chunk['path']))
            result['chunks'].append({'sha256': actual['sha256'], 'path': chunk['path'],
                'afterStopSignal': chunk['afterStopSignal'], 'text': response.get('text', '')})
        text = '\n'.join(chunk['text'] for chunk in result['chunks'])
        tail_text = '\n'.join(chunk['text'] for chunk in result['chunks'] if chunk['afterStopSignal'])
        result['checks'] = {**whisper.evaluate('far_open', text), **whisper.evaluate('far_tail', tail_text)}
        result['passed'] = all(result['checks'].values())
    except Exception as error:
        result.update({'passed': False, 'error': str(error)[:500]})
    isolated.write_json(output, result)
    print(json.dumps({'passed': result['passed'], 'evidence': str(output)}))
    return result['passed']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    capture = sub.add_parser('capture')
    capture.add_argument('--execute', action='store_true')
    capture.add_argument('--helper', required=True)
    capture.add_argument('--sha256', required=True)
    capture.add_argument('--seconds', type=int, default=75)
    capture.add_argument('--pcm-diagnostics', action='store_true', help='Opt-in aggregate amplitude evidence; no sample values logged')
    trans = sub.add_parser('transcribe')
    trans.add_argument('--execute', action='store_true')
    trans.add_argument('--run-directory', required=True)
    args = parser.parse_args()
    if not args.execute:
        print(json.dumps({'executeRequired': True, 'bundleId': BUNDLE, 'permission': 'existing only; no prompt',
                          'method': 'helper protocol + separate Whisper; NOT full Record UI'}))
        return
    if args.action == 'capture':
        if not 55 <= args.seconds <= 80: parser.error('seconds must be55..80 to bound task-only capture')
        success = run(args)
    else: success = transcribe(args)
    raise SystemExit(0 if success else 1)

if __name__ == '__main__':
    try: main()
    except Exception as error:
        print(f'Scoped capture stopped: {error}', file=__import__('sys').stderr)
        raise SystemExit(1)
