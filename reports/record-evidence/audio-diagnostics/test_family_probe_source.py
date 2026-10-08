"""Source safety contracts only: no Swift compiler, HAL access, or capture."""
import hashlib
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent


class FamilyProbeSourceTests(unittest.TestCase):
    def test_previous_exact_service_probe_is_preserved(self):
        self.assertEqual(hashlib.sha256((ROOT / 'chrome_audio_process_probe.swift').read_bytes()).hexdigest(),
                         '73cd50103a94a1134384886a67575f9e0d71ce1b40554802211919182c008457')

    def test_no_capture_permission_or_device_write_api(self):
        source = (ROOT / 'chrome_audio_family_probe.swift').read_text()
        for forbidden in ['AudioHardwareCreateProcessTap(', 'AudioHardwareCreateAggregateDevice(',
                          'AudioObjectSetPropertyData(', 'AudioDeviceStart(', 'CGRequestScreenCaptureAccess(',
                          'CGPreflightScreenCaptureAccess(', 'SCShareableContent', 'AVCaptureDevice']:
            self.assertNotIn(forbidden, source)
        self.assertIn('app.bundleIdentifier == "com.google.Chrome"', source)
        self.assertIn('kAudioHardwarePropertyProcessObjectList', source)
        self.assertIn('"unownedProcessesEmitted": 0', source)

    def test_only_validated_rows_are_emitted_and_pure_tests_precede_os_queries(self):
        source = (ROOT / 'chrome_audio_family_probe.swift').read_text()
        self.assertIn('chain.allSatisfy({ (try? identity($0.pid)) == $0 })', source)
        self.assertIn('object == kAudioObjectUnknown || (try? scalar(object, kAudioProcessPropertyPID, pid_t(0))) == pid', source)
        self.assertIn('let row = inspectOwned(object: object, pid: pid, root: root', source)
        self.assertLess(source.index('let row = inspectOwned(object: object, pid: pid, root: root'),
                        source.index('rows.append(row)'))
        start = source.index('func main() throws')
        self.assertLess(source.index('args[1] == "--self-test"', start),
                        source.index('NSRunningApplication(processIdentifier: rootPID)', start))
        self.assertIn('table[100] = process(100, 1, root.executable, start: 2)', source)
        self.assertIn('chain(100) == nil && chain(101) == nil', source)


if __name__ == '__main__': unittest.main()
