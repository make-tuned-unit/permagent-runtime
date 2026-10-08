#!/usr/bin/env python3
"""Read only the current system default-output device's public HAL metadata.

No device names/UIDs, process lists, capture, taps, writes or permission APIs.
This does not establish which output device Chrome uses or prove audibility.
"""
import argparse
import ctypes as C
from datetime import datetime, timezone
import json
import sys

from chrome_audio_family_probe import HAL, ProbeError, U32


def inspect_default_output(hal):
    device = hal.scalar(1, 'dOut')
    result = {
        'method': 'read-only public HAL metadata for system default output',
        'hasDefaultOutput': device != 0,
        'limitations': 'System default only; does not prove Chrome uses this device or that sound is audible.',
        'tapsCreated': 0, 'devicesCreated': 0, 'deviceWrites': 0,
        'permissionRequests': 0, 'processesQueried': 0,
    }
    if device:
        row = {'objectId': device}
        unavailable = []
        for key, selector, kind, scope in (
            ('isAlive', 'livn', U32, 'glob'),
            ('isRunningSomewhere', 'gone', U32, 'glob'),
            ('sampleRate', 'nsrt', C.c_double, 'glob'),
            ('masterMute', 'mute', U32, 'outp'),
            ('masterVolume', 'volm', C.c_float, 'outp')):
            try:
                row[key] = hal.scalar(device, selector, kind, scope)
            except ProbeError:
                unavailable.append(key)
        row['unavailableProperties'] = unavailable
        result['device'] = row
    if hal.scalar(1, 'dOut') != device:
        raise ProbeError('Default output changed during inspection')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--read-default-output', action='store_true', required=True)
    parser.parse_args()
    if sys.platform != 'darwin':
        raise ProbeError('Requires macOS')
    result = inspect_default_output(HAL())
    result['observedAt'] = datetime.now(timezone.utc).isoformat()
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))


if __name__ == '__main__':
    try:
        main()
    except (ProbeError, OSError):
        print('ERROR: default-output metadata unavailable or device changed', file=sys.stderr)
        sys.exit(1)
