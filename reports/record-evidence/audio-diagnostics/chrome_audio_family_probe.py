#!/usr/bin/env python3
"""Read-only HAL metadata for an explicitly selected Chrome process family.

Stdlib only; no compiler, taps, capture, writes, permission APIs or argv reads.
Public ABI constants/layouts: macOS SDK AudioHardware{,Base}.h, sys/proc_info.h,
libproc.h. OS libraries are loaded only from main(), never on import. Root is
verified by its canonical main executable and the enclosing app's Info.plist.
Unlike the Swift variant, descendant roles are not inferred from arguments.
"""
import argparse
import ctypes as C
from dataclasses import dataclass
import json
import math
from pathlib import Path
import plistlib
import sys


class ProbeError(Exception):
    pass


U32 = C.c_uint32
I32 = C.c_int32


def fourcc(value):
    return int.from_bytes(value.encode('ascii'), 'big')


class Address(C.Structure):
    _fields_ = [('selector', U32), ('scope', U32), ('element', U32)]


class BSDInfo(C.Structure):
    _fields_ = [(name, U32) for name in (
        'flags', 'status', 'xstatus', 'pid', 'ppid', 'uid', 'gid', 'ruid',
        'rgid', 'svuid', 'svgid', 'reserved')]
    _fields_ += [('comm', C.c_char * 16), ('name', C.c_char * 32)]
    _fields_ += [(name, U32) for name in ('nfiles', 'pgid', 'pjobc', 'tdev', 'tpgid')]
    _fields_ += [('nice', I32), ('started_seconds', C.c_uint64), ('started_micros', C.c_uint64)]


@dataclass(frozen=True)
class Identity:
    pid: int
    parent: int
    executable: str
    started_seconds: int
    started_micros: int


class ProcessReader:
    def __init__(self):
        if C.sizeof(BSDInfo) != 136 or BSDInfo.started_seconds.offset != 120:
            raise ProbeError('Unsupported process ABI')
        self.lib = C.CDLL('/usr/lib/libproc.dylib')
        self.lib.proc_pidinfo.argtypes = [C.c_int, C.c_int, C.c_uint64, C.c_void_p, C.c_int]
        self.lib.proc_pidinfo.restype = C.c_int
        self.lib.proc_pidpath.argtypes = [C.c_int, C.c_void_p, U32]
        self.lib.proc_pidpath.restype = C.c_int

    def __call__(self, pid):
        if not 1 < pid <= 2**31 - 1:
            return None
        info = BSDInfo()
        if self.lib.proc_pidinfo(pid, 3, 0, C.byref(info), C.sizeof(info)) != C.sizeof(info):
            return None
        path = C.create_string_buffer(4096)
        count = self.lib.proc_pidpath(pid, path, len(path))
        if count <= 0 or count >= len(path) or info.pid != pid:
            return None
        try:
            executable = str(Path(path.value.decode('utf-8')).resolve(strict=True))
        except (OSError, UnicodeError, RuntimeError):
            return None
        return Identity(pid, info.ppid, executable, info.started_seconds, info.started_micros)


class HAL:
    """Only the two public read-property entrypoints are bound."""
    def __init__(self):
        self.lib = C.CDLL('/System/Library/Frameworks/CoreAudio.framework/CoreAudio')
        self.size_fn = self.lib.AudioObjectGetPropertyDataSize
        self.size_fn.argtypes = [U32, C.POINTER(Address), U32, C.c_void_p, C.POINTER(U32)]
        self.size_fn.restype = I32
        self.data_fn = self.lib.AudioObjectGetPropertyData
        self.data_fn.argtypes = [U32, C.POINTER(Address), U32, C.c_void_p, C.POINTER(U32), C.c_void_p]
        self.data_fn.restype = I32

    def scalar(self, obj, selector, kind=U32, scope='glob', qualifier=None):
        result = kind()
        size = U32(C.sizeof(result))
        addr = Address(fourcc(selector), fourcc(scope), 0)
        qsize = C.sizeof(qualifier) if qualifier is not None else 0
        qptr = C.byref(qualifier) if qualifier is not None else None
        status = self.data_fn(obj, C.byref(addr), qsize, qptr, C.byref(size), C.byref(result))
        if status or size.value != C.sizeof(result):
            raise ProbeError('HAL scalar unavailable')
        if isinstance(result.value, float) and not math.isfinite(result.value):
            raise ProbeError('HAL scalar invalid')
        return result.value

    def objects(self, obj, selector, scope='glob'):
        addr = Address(fourcc(selector), fourcc(scope), 0)
        size = U32()
        if self.size_fn(obj, C.byref(addr), 0, None, C.byref(size)):
            raise ProbeError('HAL list unavailable')
        capacity = size.value
        if capacity > 4096 or capacity % 4:
            raise ProbeError('HAL list size invalid or excessive')
        if not capacity:
            return []
        values = (U32 * (capacity // 4))()
        status = self.data_fn(obj, C.byref(addr), 0, None, C.byref(size), values)
        if status or size.value > capacity or size.value % 4:
            raise ProbeError('HAL list changed or could not be read')
        return list(values[:size.value // 4])


def verified_contents(root):
    """Require the app's actual main executable, not an arbitrary Chrome helper."""
    executable = Path(root.executable)
    if executable.parent.name != 'MacOS' or executable.parent.parent.name != 'Contents':
        raise ProbeError('Selected PID is not an app main executable')
    contents = executable.parent.parent
    if contents.parent.suffix != '.app':
        raise ProbeError('Selected PID is not an app main executable')
    try:
        info_path = contents / 'Info.plist'
        with info_path.open('rb') as source:
            raw = source.read(1_048_577)
        if len(raw) > 1_048_576:
            raise ProbeError('App metadata exceeds bound')
        info = plistlib.loads(raw)
        name = info.get('CFBundleExecutable')
        if (info.get('CFBundleIdentifier') != 'com.google.Chrome' or
                not isinstance(name, str) or Path(name).name != name or
                str((contents / 'MacOS' / name).resolve(strict=True)) != root.executable):
            raise ProbeError('Selected PID is not the exact Chrome root')
    except (OSError, ValueError, TypeError, AttributeError, plistlib.InvalidFileException):
        raise ProbeError('Chrome app metadata unavailable or invalid') from None
    return str(contents)


def contained(executable, contents):
    # Live ProcessReader canonicalizes paths. Reject lexical traversal in fakes too.
    path, base = Path(executable), Path(contents)
    return (path.is_absolute() and '..' not in path.parts and
            len(path.parts) > len(base.parts) and path.parts[:len(base.parts)] == base.parts)


def owned_chain(pid, root, contents, lookup):
    if not contained(root.executable, contents):
        return None
    chain, seen = [], set()
    for _ in range(64):
        if pid <= 1 or pid in seen:
            return None
        seen.add(pid)
        item = lookup(pid)
        if item is None or item.pid != pid or not contained(item.executable, contents):
            return None
        chain.append(item)
        if pid == root.pid:
            return chain if item == root else None
        pid = item.parent
    return None


def optional_scalar(hal, row, key, obj, selector, kind=U32, scope='glob'):
    try:
        row[key] = hal.scalar(obj, selector, kind, scope)
    except ProbeError:
        pass  # Missing device controls are unsupported, never false/zero.


def inspect_owned(hal, lookup, obj, pid, root, contents, default_output):
    chain = owned_chain(pid, root, contents, lookup)
    if chain is None:
        return None
    row = {'pid': pid, 'role': 'chrome_root' if pid == root.pid else 'owned_descendant',
           'hasHALObject': obj != 0}
    if obj:
        try:
            if hal.scalar(obj, 'ppid', I32) != pid:
                return None
        except ProbeError:
            return None
        row['processObjectId'] = obj
        optional_scalar(hal, row, 'isRunning', obj, 'pir?')
        optional_scalar(hal, row, 'isRunningOutput', obj, 'piro')
        try:
            devices = hal.objects(obj, 'pdv#', 'outp')
            row['outputDevices'] = []
            for device in devices:
                entry = {'objectId': device, 'isDefaultOutput': device == default_output}
                for key, selector, kind, scope in (
                    ('isAlive', 'livn', U32, 'glob'),
                    ('isRunningSomewhere', 'gone', U32, 'glob'),
                    ('sampleRate', 'nsrt', C.c_double, 'glob'),
                    ('masterMute', 'mute', U32, 'outp'),
                    ('masterVolume', 'volm', C.c_float, 'outp')):
                    optional_scalar(hal, entry, key, device, selector, kind, scope)
                row['outputDevices'].append(entry)
        except ProbeError:
            row['outputDeviceQueryUnavailable'] = True
    if not revalidate(hal, lookup, obj, pid, root, contents, chain):
        return None
    return row, chain


def revalidate(hal, lookup, obj, pid, root, contents, chain):
    try:
        if obj and hal.scalar(obj, 'ppid', I32) != pid:
            return False
    except ProbeError:
        return False
    return (all(lookup(item.pid) == item for item in chain) and
            owned_chain(pid, root, contents, lookup) == chain)


def inventory(hal, lookup, root, contents):
    default_output = hal.scalar(1, 'dOut')
    root_obj = hal.scalar(1, 'id2p', qualifier=I32(root.pid))
    selected = inspect_owned(hal, lookup, root_obj, root.pid, root, contents, default_output)
    if selected is None:
        raise ProbeError('Chrome root changed during inspection')
    gathered = [(root_obj, selected)]
    included = {root_obj}
    # Global IDs/PIDs are used internally only; unrelated identities never emit.
    for obj in hal.objects(1, 'prs#'):
        if not obj or obj in included:
            continue
        # Do not revisit an object whose identity changed during this snapshot.
        included.add(obj)
        try:
            pid = hal.scalar(obj, 'ppid', I32)
        except ProbeError:
            continue
        if pid == root.pid:
            continue
        selected = inspect_owned(hal, lookup, obj, pid, root, contents, default_output)
        if selected is not None:
            gathered.append((obj, selected))
    rows = []
    for obj, (row, chain) in gathered:
        if revalidate(hal, lookup, obj, row['pid'], root, contents, chain):
            rows.append(row)
        elif row['pid'] == root.pid:
            raise ProbeError('Chrome root changed during inventory')
    if lookup(root.pid) != root:
        raise ProbeError('Chrome root changed during inventory')
    return {'method': 'read-only HAL metadata for exact Chrome root and verified owned descendants; no capture',
            'chromePID': root.pid, 'processes': rows,
            'roleClassification': 'ancestry only; process arguments not read',
            'tapsCreated': 0, 'devicesCreated': 0, 'deviceWrites': 0,
            'permissionRequests': 0, 'unownedProcessesEmitted': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--chrome-pid', type=int, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin' or not 1 < args.chrome_pid <= 2**31 - 1:
        raise ProbeError('Requires macOS and an explicit Chrome root PID')
    lookup = ProcessReader()
    root = lookup(args.chrome_pid)
    if root is None:
        raise ProbeError('Selected Chrome process absent or inaccessible')
    contents = verified_contents(root)
    if lookup(root.pid) != root:
        raise ProbeError('Selected Chrome identity changed')
    result = inventory(HAL(), lookup, root, contents)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))


if __name__ == '__main__':
    try:
        main()
    except (ProbeError, OSError):
        # No raw path, process arguments, unrelated IDs, or library error leaks.
        print('ERROR: scoped read-only inventory unavailable or identity changed', file=sys.stderr)
        sys.exit(1)
