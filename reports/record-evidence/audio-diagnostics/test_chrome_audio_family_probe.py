"""Synthetic-only tests: no HAL/libproc loads, capture, permissions or compilation."""
import ctypes as C
from dataclasses import replace
import importlib.util
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('chrome_family_probe', Path(__file__).with_name('chrome_audio_family_probe.py'))
p = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = p
with patch.object(C, 'CDLL', side_effect=AssertionError('Tests must not load OS libraries')):
    SPEC.loader.exec_module(p)

CONTENTS = '/Applications/Google Chrome.app/Contents'
ROOT = p.Identity(100, 1, CONTENTS + '/MacOS/Google Chrome', 123, 456)
HELPER = CONTENTS + '/Frameworks/Helper.app/Contents/MacOS/Helper'


def identity(pid, parent, path=HELPER):
    return p.Identity(pid, parent, path, 123, 456)


class FakeHAL:
    def __init__(self):
        self.values = {(1, 'dOut'): 70, (1, 'id2p'): 10,
                       (10, 'ppid'): 100, (11, 'ppid'): 101, (12, 'ppid'): 999,
                       (11, 'pir?'): 1, (11, 'piro'): 1,
                       (70, 'nsrt'): 48000.0, (70, 'mute'): 0, (70, 'volm'): 0.3125}
        self.lists = {(1, 'prs#'): [10, 11, 12, 11], (10, 'pdv#'): [], (11, 'pdv#'): [70]}
        self.calls = []
        self.after_scalar = lambda obj, selector: None

    def scalar(self, obj, selector, kind=p.U32, scope='glob', qualifier=None):
        self.calls.append((obj, selector))
        key = (obj, selector)
        if key not in self.values:
            raise p.ProbeError('unsupported')
        value = self.values[key]
        self.after_scalar(obj, selector)
        return value

    def objects(self, obj, selector, scope='glob'):
        self.calls.append((obj, selector))
        if (obj, selector) not in self.lists:
            raise p.ProbeError('unsupported')
        return self.lists[obj, selector]


class FamilyTests(unittest.TestCase):
    def setUp(self):
        self.table = {100: ROOT, 101: identity(101, 100), 102: identity(102, 101),
                      999: identity(999, 1, '/Applications/Other.app/Contents/MacOS/Other')}
        self.hal = FakeHAL()
        self.no_load = patch.object(C, 'CDLL', side_effect=AssertionError('No OS libraries in tests'))
        self.no_load.start()
        self.addCleanup(self.no_load.stop)

    def chain(self, pid):
        return p.owned_chain(pid, ROOT, CONTENTS, self.table.get)

    def inventory(self):
        return p.inventory(self.hal, self.table.get, ROOT, CONTENTS)

    def test_public_abi_layout_and_fourcc(self):
        self.assertEqual(C.sizeof(p.BSDInfo), 136)
        self.assertEqual(p.BSDInfo.started_seconds.offset, 120)
        self.assertEqual(p.BSDInfo.started_micros.offset, 128)
        self.assertEqual(C.sizeof(p.Address), 12)
        self.assertEqual(p.fourcc('ppid'), 0x70706964)

    def test_exact_root_direct_and_nested_owned_chains(self):
        self.assertEqual(self.chain(100), [ROOT])
        self.assertEqual([x.pid for x in self.chain(102)], [102, 101, 100])

    def test_rejects_unrelated_prefix_collision_traversal_and_missing_process(self):
        for path in ['/Applications/Google Chrome.app.other/Contents/MacOS/Helper',
                     CONTENTS + '/../Other', '/Applications/Other.app/Contents/MacOS/Other']:
            self.table[101] = identity(101, 100, path)
            self.assertIsNone(self.chain(101))
        self.table[101] = identity(101, 999)
        self.assertIsNone(self.chain(101))
        self.assertIsNone(self.chain(1))
        self.assertIsNone(self.chain(103))

    def test_rejects_external_intermediary_cycles_lookup_mismatch_and_reuse(self):
        self.table[101] = identity(101, 100, '/other')
        self.assertIsNone(self.chain(102))
        self.table[101] = identity(101, 102)
        self.assertIsNone(self.chain(102))
        self.table[101] = identity(999, 100)
        self.assertIsNone(self.chain(101))
        self.table[101] = identity(101, 100)
        self.table[100] = replace(ROOT, started_micros=457)
        self.assertIsNone(self.chain(101))

    def test_only_owned_rows_with_real_activity_and_supported_device_values_emit(self):
        result = self.inventory()
        self.assertEqual([r['pid'] for r in result['processes']], [100, 101])
        row = result['processes'][1]
        self.assertEqual(row['isRunningOutput'], 1)
        self.assertEqual(row['outputDevices'], [{'objectId': 70, 'isDefaultOutput': True,
                         'sampleRate': 48000.0, 'masterMute': 0, 'masterVolume': 0.3125}])
        self.assertNotIn((12, 'piro'), self.hal.calls)
        self.assertNotIn('executable', repr(result))
        self.assertEqual(result['unownedProcessesEmitted'], 0)

    def test_root_without_hal_object_still_emits_and_unsupported_devices_are_explicit(self):
        self.hal.values[1, 'id2p'] = 0
        del self.hal.lists[11, 'pdv#']
        rows = self.inventory()['processes']
        self.assertFalse(rows[0]['hasHALObject'])
        self.assertNotIn('isRunningOutput', rows[0])
        self.assertTrue(rows[1]['outputDeviceQueryUnavailable'])

    def test_child_reused_during_metadata_read_is_dropped(self):
        def change(obj, selector):
            if (obj, selector) == (11, 'piro'):
                self.table[101] = replace(self.table[101], started_seconds=999)
        self.hal.after_scalar = change
        self.assertEqual([r['pid'] for r in self.inventory()['processes']], [100])

    def test_hal_object_pid_changed_during_metadata_read_is_dropped(self):
        def change(obj, selector):
            if (obj, selector) == (11, 'piro'):
                self.hal.values[11, 'ppid'] = 999
        self.hal.after_scalar = change
        self.assertEqual([r['pid'] for r in self.inventory()['processes']], [100])

    def test_root_reused_during_inventory_rejects_entire_result(self):
        def change(obj, selector):
            if (obj, selector) == (11, 'piro'):
                self.table[100] = replace(ROOT, started_seconds=999)
        self.hal.after_scalar = change
        with self.assertRaises(p.ProbeError):
            self.inventory()

    def test_child_reparented_after_row_inspection_is_dropped_before_output(self):
        def change(obj, selector):
            if (obj, selector) == (12, 'ppid'):
                self.table[101] = replace(self.table[101], parent=999)
        self.hal.after_scalar = change
        self.assertEqual([r['pid'] for r in self.inventory()['processes']], [100])

    def test_exact_bundle_and_main_executable_required(self):
        with tempfile.TemporaryDirectory() as temp:
            contents = Path(temp) / 'Google Chrome.app' / 'Contents'
            (contents / 'MacOS').mkdir(parents=True)
            exe = contents / 'MacOS' / 'Google Chrome'
            exe.touch()
            root = replace(ROOT, executable=str(exe.resolve()))
            info = {'CFBundleIdentifier': 'com.google.Chrome', 'CFBundleExecutable': 'Google Chrome'}
            target = contents / 'Info.plist'
            target.write_bytes(plistlib.dumps(info))
            self.assertEqual(p.verified_contents(root), str(contents.resolve()))
            for field, value in [('CFBundleIdentifier', 'com.google.Chrome.lookalike'),
                                 ('CFBundleExecutable', '../MacOS/Google Chrome')]:
                target.write_bytes(plistlib.dumps({**info, field: value}))
                with self.assertRaises(p.ProbeError):
                    p.verified_contents(root)

    def test_unowned_rows_never_trigger_device_queries(self):
        self.table[101] = identity(101, 1)
        rows = self.inventory()['processes']
        self.assertEqual(len(rows), 1)
        self.assertNotIn((11, 'pdv#'), self.hal.calls)


class HALBufferTests(unittest.TestCase):
    def fake(self, size, returned=None, status=0):
        hal = object.__new__(p.HAL)
        def sized(obj, addr, qs, qp, dest):
            C.cast(dest, C.POINTER(p.U32))[0] = size
            return 0
        def data(obj, addr, qs, qp, actual, values):
            C.cast(actual, C.POINTER(p.U32))[0] = size if returned is None else returned
            return status
        hal.size_fn, hal.data_fn = sized, data
        return hal

    def test_excessive_misaligned_growing_and_failed_hal_lists_reject(self):
        for hal in [self.fake(4097), self.fake(3), self.fake(4, 8),
                    self.fake(8, 3), self.fake(4, status=-1)]:
            with self.assertRaises(p.ProbeError):
                hal.objects(1, 'prs#')
        self.assertEqual(self.fake(0).objects(1, 'prs#'), [])
        self.assertEqual(self.fake(8, 4).objects(1, 'prs#'), [0])


if __name__ == '__main__':
    unittest.main()
