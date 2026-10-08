#!/usr/bin/env python3
"""Run the real captured ASR text through isolated note/triage/Brain/Kanban APIs.

Reuses the existing retry/provenance/card acceptance checks. This is not a
native Record UI or Tauri IPC acceptance run. Creates synthetic projects only.
"""
import argparse
import json
from pathlib import Path
import time

import isolated_daemon as isolated
import scoped_capture
import test_isolated_meeting_http as meeting
import test_isolated_whisper_http as whisper


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--transcription', required=True)
    args = parser.parse_args()
    if not args.execute:
        print('Requires --execute; creates two synthetic projects only on the owned3002 runtime.')
        return
    path = Path(args.transcription)
    if (path.resolve() != path or not path.is_file() or
            path.parent.parent != scoped_capture.CAPTURES or not path.name.startswith('whisper-')):
        raise RuntimeError('Expected a canonical private transcription receipt under the owned capture root')
    asr = json.loads(path.read_text())
    capture_path = path.parent / 'capture.json'
    capture = json.loads(capture_path.read_text())
    if not asr.get('passed') or asr.get('sourceCapture') != str(capture_path) or not capture.get('capturePassed'):
        raise RuntimeError('Capture and transcription must have passed independently')
    if len(asr['chunks']) != len(capture['chunks']):
        raise RuntimeError('Transcription chunk count changed')
    for observed, recorded in zip(asr['chunks'], capture['chunks']):
        actual = scoped_capture.chunk_info(recorded['path'], path.parent / 'chunks')
        if (observed['path'] != recorded['path'] or observed['sha256'] != actual['sha256'] or
                recorded['sha256'] != actual['sha256'] or observed['afterStopSignal'] != recorded['afterStopSignal']):
            raise RuntimeError('Captured WAV/transcription provenance mismatch')
    transcript = '\n'.join(row['text'] for row in asr['chunks'])
    tail = '\n'.join(row['text'] for row in asr['chunks'] if row['afterStopSignal'])
    if not all({**whisper.evaluate('far_open', transcript), **whisper.evaluate('far_tail', tail)}.values()):
        raise RuntimeError('Observed ASR lost required semantics')
    name = 'captured-audio'
    # These are literal observed sentences, not substituted fixture narration.
    negations = ['The old idea to buy equipment is cancelled.', 'Do not create a purchase task.']
    if not all(sentence in transcript for sentence in negations):
        raise RuntimeError('This receipt does not contain the independently observed negation sentences')
    meeting.SCENARIOS[name] = {'transcript': transcript, 'tail': tail, 'negations': negations}
    evidence = {
        'method': 'actual captured WAV→retained ASR→isolated HTTP note/triage/Brain/Kanban; NOT native UI/IPC',
        'startedUtc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'sourceTranscription': isolated.artifact(path), 'sourceCapture': isolated.artifact(capture_path),
        'checks': [], 'tokenLogged': False,
    }
    output = meeting.evidence_path(name)
    try:
        meeting.execute(evidence, 300, name)
    except Exception as error:
        evidence['error'] = meeting.failure_details(error)
    evidence['passed'] = ('error' not in evidence and bool(evidence['checks']) and
                          all(check['passed'] for check in evidence['checks']))
    meeting.save_evidence(output, evidence)
    print(json.dumps({'passed': evidence['passed'], 'evidence': str(output), 'checks': len(evidence['checks'])}))
    raise SystemExit(0 if evidence['passed'] else 1)


if __name__ == '__main__':
    main()
