"""Pure protocol/path fixtures; never runs helper, capture, permissions or Whisper."""
from array import array
from pathlib import Path
import tempfile
import unittest
import wave
from unittest.mock import patch
import scoped_capture as capture

class ScopedCaptureTests(unittest.TestCase):
    def wav(self, path, rate=16000):
        with wave.open(str(path), 'wb') as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(rate)
            wav.writeframes(array('h', [0, 100, -100, 50] * 100).tobytes())

    def test_canonical_pcm_chunk_has_signal_hash_and_partial_duration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / 'sysaudio_0000.wav'
            self.wav(path)
            info = capture.chunk_info(path, root)
            self.assertEqual(info['frames'], 400)
            self.assertEqual(info['peak'], 100)
            self.assertGreater(info['rms'], 0)
            self.assertLess(info['durationSeconds'], 45)
            self.assertEqual(len(info['sha256']), 64)

    def test_chunk_rejects_symlink_outside_name_and_wrong_format(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / 'sysaudio_0000.wav'
            self.wav(path, 24000)
            with self.assertRaisesRegex(RuntimeError, 'PCM16'): capture.chunk_info(path, root)
            link = root / 'sysaudio_0001.wav'
            link.symlink_to(path)
            with self.assertRaisesRegex(RuntimeError, 'noncanonical'): capture.chunk_info(link, root)
            with self.assertRaisesRegex(RuntimeError, 'noncanonical'): capture.chunk_info(root / 'private.wav', root)
            with self.assertRaisesRegex(RuntimeError, 'noncanonical'): capture.chunk_info(path, root / 'other')

    def test_old_helper_rejected_before_signature_or_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            helper = root / 'old-helper'
            helper.write_bytes(capture.SCOPE.encode())
            helper.chmod(0o700)
            with patch.object(capture.isolated, 'TARGET', root), patch.object(capture.subprocess, 'run') as run:
                with self.assertRaisesRegex(RuntimeError, 'nonprompting-preflight'):
                    capture.helper_check(str(helper), capture.isolated.digest(helper))
                run.assert_not_called()

    def test_permission_preflight_source_precedes_screen_capture_and_never_requests(self):
        source = (capture.isolated.LANE / 'ui/desktop/src-tauri/audiocap/main.swift').read_text()
        preflight = source.index('if preflightOnly || requireExistingPermission')
        shareable = source.index('content = try await SCShareableContent')
        self.assertLess(preflight, shareable)
        self.assertIn('guard CGPreflightScreenCaptureAccess() else', source)
        self.assertNotIn('CGRequestScreenCaptureAccess(', source)
        self.assertIn('if preflightOnly { exit(0) }', source)
        self.assertIn('case "--require-existing-permission"', source)

    def test_pcm_classification_separates_upstream_silence_and_conversion_loss(self):
        metrics = {'phase': 'final', 'rawChannels': [{'sampleCount': 10, 'peak': 0}],
                   'downmixed': {'peak': 0}, 'resampledFloat': {'peak': 0}, 'quantizedInt16': {'peak': 0}}
        self.assertEqual(capture.classify_pcm_diagnostics(None), 'incomplete_diagnostics')
        self.assertEqual(capture.classify_pcm_diagnostics(metrics), 'incoming_pcm_silent')
        metrics['rawChannels'][0]['peak'] = .25
        self.assertEqual(capture.classify_pcm_diagnostics(metrics), 'downmix_cancellation')
        metrics['downmixed']['peak'] = .25
        self.assertEqual(capture.classify_pcm_diagnostics(metrics), 'resampling_loss')
        metrics['resampledFloat']['peak'] = .000001
        self.assertEqual(capture.classify_pcm_diagnostics(metrics), 'quantization_loss')
        metrics['quantizedInt16']['peak'] = 500
        self.assertEqual(capture.classify_pcm_diagnostics(metrics), 'nonzero_pcm_reached_wav')

    def test_pcm_probe_is_default_off_exact_chrome_and_existing_permission_only(self):
        source = (capture.isolated.LANE / 'ui/desktop/src-tauri/audiocap/main.swift').read_text()
        self.assertIn('var pcmDiagnostics = false', source)
        self.assertIn('case "--pcm-diagnostics": pcmDiagnostics = true', source)
        self.assertIn('if pcmDiagnostics && (includedBundleID != "com.google.Chrome" || !requireExistingPermission)', source)
        self.assertIn('guard pcmDiagnostics else { return }', source)
        self.assertLess(source.index('if pcmDiagnostics &&'), source.index('content = try await SCShareableContent'))
        self.assertIn('self.reportPCM("final")', source)

    def test_hal_probe_has_no_capture_or_mutation_apis(self):
        source = (capture.isolated.LANE / 'reports/record-evidence/audio-diagnostics/chrome_audio_process_probe.swift').read_text()
        for forbidden in ['AudioHardwareCreateProcessTap(', 'AudioHardwareCreateAggregateDevice(',
                          'AudioObjectSetPropertyData(', 'AudioDeviceStart(', 'CGRequestScreenCaptureAccess(']:
            self.assertNotIn(forbidden, source)
        self.assertIn('app.bundleIdentifier == "com.google.Chrome"', source)
        self.assertIn('--utility-sub-type=audio.mojom.AudioService', source)
        self.assertIn('kAudioHardwarePropertyTranslatePIDToProcessObject', source)
        self.assertIn('kAudioProcessPropertyIsRunningOutput', source)
        self.assertIn('Selected process identity changed during HAL inspection', source)

if __name__ == '__main__': unittest.main()
