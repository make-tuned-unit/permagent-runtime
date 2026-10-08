"""Pure fake-HAL checks; imports must not load an OS library."""
import ctypes as C
import unittest
from unittest.mock import patch

with patch.object(C, 'CDLL', side_effect=AssertionError('No live OS libraries in tests')):
    import default_output_probe as p


class FakeHAL:
    def __init__(self):
        self.default = 70
        self.values = {'livn': 1, 'gone': 1, 'nsrt': 48000.0, 'mute': 0, 'volm': 0.3125}
        self.calls = []

    def scalar(self, obj, selector, kind=p.U32, scope='glob'):
        self.calls.append((obj, selector, scope))
        if obj == 1 and selector == 'dOut':
            return self.default
        if obj != self.default:
            raise AssertionError('Only selected default device may be queried')
        if selector not in self.values:
            raise p.ProbeError('unsupported')
        return self.values[selector]


class DefaultOutputTests(unittest.TestCase):
    def setUp(self):
        self.hal = FakeHAL()
        no_load = patch.object(C, 'CDLL', side_effect=AssertionError('No live OS libraries in tests'))
        no_load.start()
        self.addCleanup(no_load.stop)

    def test_only_allowed_selected_device_metadata_and_scopes(self):
        result = p.inspect_default_output(self.hal)
        self.assertEqual(result['device'], {'objectId': 70, 'isAlive': 1,
            'isRunningSomewhere': 1, 'sampleRate': 48000.0,
            'masterMute': 0, 'masterVolume': 0.3125, 'unavailableProperties': []})
        self.assertEqual(self.hal.calls, [(1, 'dOut', 'glob'), (70, 'livn', 'glob'),
            (70, 'gone', 'glob'), (70, 'nsrt', 'glob'), (70, 'mute', 'outp'),
            (70, 'volm', 'outp'), (1, 'dOut', 'glob')])
        self.assertEqual(result['processesQueried'], 0)
        self.assertIn('does not prove Chrome uses this device', result['limitations'])

    def test_unsupported_controls_not_reported_as_zero_or_unmuted(self):
        del self.hal.values['mute']
        del self.hal.values['volm']
        row = p.inspect_default_output(self.hal)['device']
        self.assertNotIn('masterMute', row)
        self.assertNotIn('masterVolume', row)
        self.assertEqual(row['unavailableProperties'], ['masterMute', 'masterVolume'])

    def test_no_default_device_does_not_query_any_device(self):
        self.hal.default = 0
        result = p.inspect_default_output(self.hal)
        self.assertFalse(result['hasDefaultOutput'])
        self.assertNotIn('device', result)
        self.assertEqual(self.hal.calls, [(1, 'dOut', 'glob')] * 2)

    def test_changed_default_rejects_mixed_snapshot(self):
        scalar = self.hal.scalar
        def changed(obj, selector, kind=p.U32, scope='glob'):
            value = scalar(obj, selector, kind, scope)
            if selector == 'volm':
                self.hal.default = 80
            return value
        self.hal.scalar = changed
        with self.assertRaises(p.ProbeError):
            p.inspect_default_output(self.hal)


if __name__ == '__main__':
    unittest.main()
