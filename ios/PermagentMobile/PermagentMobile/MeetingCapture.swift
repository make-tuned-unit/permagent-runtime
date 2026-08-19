// Meeting capture — phone flat on the table, screen locked, for hours.
//
// This is the microphone half of the meeting recorder. The durable half is
// `MeetingQueue.swift`; this file's only job is to keep producing finished
// audio files and hand each one to the store the instant it closes.
//
// ── THE FORMAT, AND WHY IT IS NOT COMPRESSED ────────────────────────────────
//
// 16 kHz mono 16-bit LinearPCM in a WAV container: ~115 MB per hour. That is
// not the frugal choice, and it was not the first one. It is the only one the
// hub can actually read.
//
// The hub decodes every clip through one function — `decode_audio_simple` in
// crates/goose/src/dictation/whisper.rs — using symphonia with
// `features = ["all"]`. Files produced by AVFoundation with each candidate
// setting were run through a faithful copy of that exact function on
// 2026-08-19. What it accepts is much narrower than what symphonia supports:
//
//   LinearPCM 16-bit mono in WAV .... OK, 120.0 s decoded
//   AAC-LC in M4A ................... FAIL "No channel information in audio track"
//   AAC-LC in CAF ................... FAIL "aac: aac too complex"
//   IMA4 ADPCM in CAF ............... FAIL "adpcm: maximum frames per packet is required"
//   FLAC in CAF ..................... FAIL "flac: minimum block length is 16 samples"
//   ALAC in M4A ..................... FAIL "No channel information in audio track"
//   mu-law in WAV ................... FAIL "wav: malformed fmt_mulaw chunk"
//   FLAC in .flac ................... OK
//   ALAC in CAF ..................... OK
//
// Opus was ruled out before any of this: no Opus decoder appears in the hub's
// resolved dependency set at all, so an Opus file is rejected at the probe.
//
// The AAC failure is worth naming precisely, because it is a hub-side bug and
// not a property of AAC. `symphonia-format-isomp4` fills a track's codec type
// and its AudioSpecificConfig from the `esds` atom but never sets `channels`;
// `decode_audio_simple` reads `codec_params.channels` and bails when it is
// absent — BEFORE the decoder, which knows the answer, is ever consulted.
// Removing that one guard and taking the channel count from the first decoded
// buffer makes the same AAC file decode to all 120.2 seconds. That fix is
// worth about a 10x reduction here (24 kbps AAC is ~10.8 MB/hour) and is the
// obvious follow-up; until the hub carries it, sending AAC would mean every
// segment failing on arrival, and this feature exists because a meeting was
// lost, not to be clever about bitrate.
//
// The two formats that DID pass, FLAC and ALAC-in-CAF, are lossless and would
// save perhaps 40%. Not enough to change the category, and both would be an
// unverified `AVAudioRecorder` path; WAV is what the existing dictation client
// already sends, so it is the one path known to work end to end today.
//
// The size is survivable because it is transient: a segment is deleted the
// moment the hub confirms it, so with a reachable hub the phone holds about
// two minutes of audio at a time. 115 MB/hour is the WORST case — a whole
// meeting with the Mac asleep — and the screen says so before you start.
//
// ── SEGMENTS ────────────────────────────────────────────────────────────────
//
// The recording is cut into fixed-length pieces so each can be uploaded as it
// completes. A dropped connection or a flat battery costs the piece in flight,
// not the meeting. Two minutes of the format above is 3.84 MB — well under the
// hub's 25 MB request ceiling (`MAX_AUDIO_SIZE`,
// crates/goose-server/src/routes/dictation.rs), which is the limit that forced
// the old dictation recorder to stop at ten minutes. Chunking is what removes
// that ceiling: it is per-request, and there is no longer one request per
// meeting. Segments roll over by ARMING the next recorder at the exact
// boundary before the current one ends, rather than stopping and starting in
// sequence, so the seam is as small as the audio session can make it.
//
// ── SURVIVING THE LOCK SCREEN ───────────────────────────────────────────────
//
// `UIBackgroundModes: [audio]` is declared; what makes it work is a `.record`
// session that stays active, plus handling every way iOS takes the microphone
// away — a phone call, another app, a media-services reset, a route change —
// by resuming into a NEW segment rather than dying quietly. A watchdog checks
// the recorder is actually running and restarts it if it is not.

import Foundation
import AVFoundation
import UIKit

@MainActor
final class MeetingCapture: NSObject, ObservableObject {

    // ── Tuning ───────────────────────────────────────────────────────────────

    /// Seconds of audio per uploadable segment.
    ///
    /// Two minutes is 3.84 MB in this format — comfortably under the hub's
    /// 25 MB request ceiling, so the limit that broke the lost meeting cannot
    /// be reached however long the meeting runs. It also bounds what a kill
    /// costs: the only audio at risk is whatever is in the segment currently
    /// being written, never more than two minutes — and even that is usually
    /// recovered, because launch recovery repairs the unfinished file's header
    /// and queues it (`WavInspector`).
    static let segmentSeconds: TimeInterval = 120

    /// 16 kHz mono 16-bit LinearPCM. Whisper's native rate, so the hub neither
    /// resamples nor downmixes, and the one container/codec pair measured to
    /// decode on the hub — see the note at the top of this file.
    static let audioSettings: [String: Any] = [
        AVFormatIDKey: Int(kAudioFormatLinearPCM),
        AVSampleRateKey: 16_000.0,
        AVNumberOfChannelsKey: 1,
        AVLinearPCMBitDepthKey: 16,
        AVLinearPCMIsFloatKey: false,
        AVLinearPCMIsBigEndianKey: false,
    ]

    static let fileExtension = "wav"

    /// Megabytes an hour of recording occupies while it is waiting to send.
    /// 16000 samples/s x 2 bytes x 3600 s.
    static let megabytesPerHour: Double = 110

    // ── Published state ──────────────────────────────────────────────────────

    @Published private(set) var isRecording = false
    /// Seconds captured. Excludes time lost to an interruption, so it is the
    /// length of the audio, not of the wall clock.
    @Published private(set) var elapsed: TimeInterval = 0
    @Published private(set) var levels: [CGFloat] = Array(repeating: 0, count: 28)
    @Published private(set) var segmentsCaptured = 0
    /// Set while another app or a phone call holds the microphone. The UI says
    /// so out loud — a recording that has stopped must never look live.
    @Published private(set) var interrupted = false
    @Published private(set) var lastProblem: String?

    private(set) var recordingId: UUID?

    /// Called on the main actor whenever a segment lands in the store, so the
    /// uploader can drain without polling.
    var onSegmentReady: (() -> Void)?

    // ── Internals ────────────────────────────────────────────────────────────

    private let store: RecordingStore
    private var active: AVAudioRecorder?
    /// Armed for the next boundary, not yet recording.
    private var armed: AVAudioRecorder?
    private var indexFor: [ObjectIdentifier: Int] = [:]
    private var nextIndex = 0
    private var nextBoundary: TimeInterval = 0
    private var completedDuration: TimeInterval = 0
    private var meterTimer: Timer?
    private var watchdog: Timer?
    private var observers: [NSObjectProtocol] = []

    /// One recorder for the app, not one per view.
    ///
    /// A meeting must not stop because the user navigated away from the
    /// screen, switched tabs, or opened their notes to check something. If the
    /// recorder were owned by the view, SwiftUI tearing that view down would
    /// silently end the recording — the same class of failure as the ten-minute
    /// stop: the meeting is over and nothing says so.
    static let shared = MeetingCapture()

    init(store: RecordingStore = .shared) {
        self.store = store
        super.init()
    }

    enum MicPermission { case granted, denied }

    func requestPermission() async -> MicPermission {
        await AVAudioApplication.requestRecordPermission() ? .granted : .denied
    }

    /// Free space on the volume, in megabytes — used to warn before a long
    /// meeting rather than after it.
    static func freeMegabytes() -> Double? {
        let url = RecordingStore.defaultRoot()
        guard let values = try? url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey]),
              let bytes = values.volumeAvailableCapacityForImportantUsage
        else { return nil }
        return Double(bytes) / 1_048_576
    }

    // ── Start / stop ─────────────────────────────────────────────────────────

    func start(projectId: String?, projectName: String?) throws {
        guard !isRecording else { return }
        let recording = store.begin(
            kind: .meeting, startedAt: Date(),
            projectId: projectId, projectName: projectName
        )
        recordingId = recording.id
        nextIndex = 0
        completedDuration = 0
        elapsed = 0
        segmentsCaptured = 0
        interrupted = false
        lastProblem = nil
        levels = Array(repeating: 0, count: levels.count)

        try activateSession()
        subscribeToSessionEvents()
        try beginChain()
        isRecording = true
        startTimers()
    }

    /// Stop, finalise the tail segment, and close the recording so the
    /// uploader knows nothing more is coming.
    func stop() {
        guard isRecording, let id = recordingId else { return }
        isRecording = false
        stopTimers()
        unsubscribe()

        // The armed recorder never started; it must not leave a stub behind.
        discardArmed()

        // Stopping the live recorder finalises its file — this is the "tail"
        // the desktop recorder flushes on stop, and it is delivered through
        // the same delegate path as every other segment.
        finaliseActive()

        store.close(id)
        deactivateSession()
        onSegmentReady?()
    }

    // ── Session ──────────────────────────────────────────────────────────────

    private func activateSession() throws {
        let session = AVAudioSession.sharedInstance()
        // `.record`, not `.playAndRecord`: nothing is played back, and the
        // narrower category is the one iOS is least likely to take away.
        //
        // Mode `.default`, NOT `.voiceChat` or `.measurement`. `.voiceChat`
        // applies echo cancellation and aggressive processing tuned for a
        // phone held to the head, which is the wrong shape for a device flat
        // on a table hearing a room. The system's ordinary gain control is
        // what makes a distant speaker audible.
        //
        // No `.allowBluetoothHFP`: a paired headset would silently become the
        // microphone at 8 kHz, which is worse than the built-in array for a
        // room, and the user put the PHONE on the table.
        try session.setCategory(.record, mode: .default, options: [])
        try session.setActive(true)
    }

    private func deactivateSession() {
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func subscribeToSessionEvents() {
        let centre = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()

        // `Notification` is not Sendable, so each closure reads the plain
        // values it needs on the main queue and hands only those across.
        observers.append(centre.addObserver(
            forName: AVAudioSession.interruptionNotification, object: session, queue: .main
        ) { [weak self] note in
            let type = (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt)
                .flatMap(AVAudioSession.InterruptionType.init(rawValue:))
            let options = (note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt)
                .map(AVAudioSession.InterruptionOptions.init(rawValue:)) ?? []
            Task { @MainActor in self?.handleInterruption(type: type, options: options) }
        })

        // Media services can be reset out from under a long recording. Every
        // audio object is invalid afterwards, so the session and the recorder
        // chain are both rebuilt.
        observers.append(centre.addObserver(
            forName: AVAudioSession.mediaServicesWereResetNotification, object: session, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.rebuildAfterLoss(reason: "the phone\u{2019}s audio system restarted") }
        })

        observers.append(centre.addObserver(
            forName: AVAudioSession.routeChangeNotification, object: session, queue: .main
        ) { [weak self] note in
            let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt)
                .flatMap(AVAudioSession.RouteChangeReason.init(rawValue:))
            Task { @MainActor in self?.handleRouteChange(reason: reason) }
        })

        // Coming back to the foreground is the natural moment to check the
        // recording is still alive after whatever happened while we were away.
        observers.append(centre.addObserver(
            forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.resumeIfStalled() }
        })
    }

    private func unsubscribe() {
        observers.forEach { NotificationCenter.default.removeObserver($0) }
        observers.removeAll()
    }

    private func handleInterruption(
        type: AVAudioSession.InterruptionType?,
        options: AVAudioSession.InterruptionOptions
    ) {
        guard isRecording, let type else { return }

        switch type {
        case .began:
            // iOS has taken the microphone. Close the current segment cleanly
            // so what was captured is finalised and uploadable, rather than
            // hoping a paused recorder resumes into the same file.
            interrupted = true
            lastProblem = "Paused \u{2014} a call or another app took the microphone. Recording resumes by itself."
            finaliseActive()
            discardArmed()
        case .ended:
            // `.shouldResume` is a hint, not a permission. We try either way:
            // the alternative is a recording that is silently over while the
            // screen still says "live".
            _ = options
            rebuildAfterLoss(reason: nil)
        @unknown default:
            break
        }
    }

    private func handleRouteChange(reason: AVAudioSession.RouteChangeReason?) {
        guard isRecording, let reason else { return }
        switch reason {
        case .oldDeviceUnavailable, .newDeviceAvailable, .override, .categoryChange:
            // A route change can stop the recorder without an interruption
            // notification. Only rebuild if it actually died.
            resumeIfStalled()
        default:
            break
        }
    }

    /// Rebuild the session and start a fresh segment. Used after an
    /// interruption ends and after a media-services reset.
    private func rebuildAfterLoss(reason: String?) {
        guard isRecording else { return }
        do {
            try activateSession()
            try beginChain()
            interrupted = false
            lastProblem = reason.map { "Recording resumed after \($0)." }
        } catch {
            interrupted = true
            lastProblem = "Couldn't restart the microphone yet — everything recorded so far is safe. Retrying."
        }
    }

    /// The watchdog's question: are we supposed to be recording, and are we?
    private func resumeIfStalled() {
        guard isRecording else { return }
        if active?.isRecording == true { return }
        // A boundary may have just passed: the armed recorder has taken over
        // and the delegate simply has not reported it yet. Adopt it, rather
        // than starting a third recorder on top of a healthy one.
        if let armed, armed.isRecording {
            active = armed
            self.armed = nil
            armNext()
            return
        }
        rebuildAfterLoss(reason: nil)
    }

    // ── The segment chain ────────────────────────────────────────────────────

    private func stagingURL(for index: Int) -> URL {
        guard let id = recordingId else { return RecordingStore.defaultRoot() }
        return store.directory(for: id)
            .appendingPathComponent(
                String(format: "%@%04d.%@", RecordingStore.inProgressPrefix, index, Self.fileExtension)
            )
    }

    private func makeRecorder(index: Int) throws -> AVAudioRecorder {
        let recorder = try AVAudioRecorder(url: stagingURL(for: index), settings: Self.audioSettings)
        recorder.delegate = self
        recorder.isMeteringEnabled = true
        recorder.prepareToRecord()
        indexFor[ObjectIdentifier(recorder)] = index
        return recorder
    }

    /// Start recording now, and arm the following segment at the boundary.
    private func beginChain() throws {
        let first = try makeRecorder(index: nextIndex)
        nextIndex += 1
        let startAt = first.deviceCurrentTime + 0.12
        if !first.record(atTime: startAt, forDuration: Self.segmentSeconds) {
            // Scheduling refused — record immediately instead. A slightly
            // ragged boundary is not worth failing a meeting over.
            guard first.record(forDuration: Self.segmentSeconds) else {
                throw CocoaError(.fileWriteUnknown)
            }
            nextBoundary = first.deviceCurrentTime + Self.segmentSeconds
        } else {
            nextBoundary = startAt + Self.segmentSeconds
        }
        active = first
        armNext()
    }

    /// Arm the next recorder to begin at exactly the boundary the current one
    /// ends at. Both objects exist at once; only one is ever recording.
    private func armNext() {
        guard isRecording || active != nil else { return }
        guard armed == nil else { return }
        do {
            let next = try makeRecorder(index: nextIndex)
            nextIndex += 1
            let when = max(nextBoundary, next.deviceCurrentTime + 0.05)
            if next.record(atTime: when, forDuration: Self.segmentSeconds) {
                nextBoundary = when + Self.segmentSeconds
                armed = next
            } else {
                // Could not schedule; the delegate will start the next one the
                // moment the current segment finishes.
                indexFor[ObjectIdentifier(next)] = nil
                next.deleteRecording()
            }
        } catch {
            lastProblem = "Couldn't prepare the next segment — retrying."
        }
    }

    /// A recorder finished (boundary reached, stop(), or interruption). Adopt
    /// its file into the durable store and keep the chain moving.
    fileprivate func segmentFinished(_ recorder: AVAudioRecorder, success: Bool) {
        let key = ObjectIdentifier(recorder)
        let index = indexFor[key]
        indexFor[key] = nil
        let url = recorder.url
        // Measured from the finished FILE, not from `recorder.currentTime` —
        // which reads 0 once a recorder has stopped, so the tail segment (the
        // partial one at the end of every meeting) would have been recorded
        // as a full-length segment and the elapsed readout would have lied.
        let duration = Self.fileDuration(at: url) ?? Self.segmentSeconds

        if recorder === active { active = nil }

        // Promote the armed recorder — it is already running at this point.
        if let armed, armed.isRecording {
            active = armed
            self.armed = nil
            armNext()
        } else if isRecording, !interrupted, active == nil {
            // Nothing was armed (or it failed to start): begin a fresh chain
            // straight away rather than leaving the meeting unrecorded.
            self.armed = nil
            try? beginChain()
        }

        guard let index, let id = recordingId else { return }
        adopt(url: url, index: index, duration: duration, success: success, recordingId: id)
    }

    private func adopt(url: URL, index: Int, duration: TimeInterval, success: Bool, recordingId id: UUID) {
        let attributes = try? FileManager.default.attributesOfItem(atPath: url.path)
        let size = (attributes?[.size] as? NSNumber)?.intValue ?? 0
        // A file this small is a stub from a recorder that was armed and then
        // cancelled — it carries no audio. Nothing else is ever discarded:
        // "the recording failed" is NOT grounds for deleting audio, which is
        // the mistake this whole feature exists to undo, so an unsuccessful
        // recorder's file is still adopted and still uploaded.
        guard size > 1_024 else {
            try? FileManager.default.removeItem(at: url)
            return
        }
        // A recorder that ended abnormally can leave the RIFF lengths saying
        // "nothing written yet", which the hub rejects outright. Cheap to
        // check, and the difference between keeping this piece and losing it.
        WavInspector.repairHeaderIfNeeded(at: url)
        do {
            _ = try store.adopt(fileAt: url, into: id, index: index, duration: duration)
            completedDuration += duration
            segmentsCaptured += 1
            if !success {
                lastProblem = "One segment ended early — it was kept and will still be transcribed."
            }
            onSegmentReady?()
        } catch {
            // The file could not be moved into place. Leave it exactly where
            // it is: an un-adopted file is recoverable by hand, a deleted one
            // is not.
            lastProblem = "A segment couldn't be filed away — the audio is still on this phone."
        }
    }

    /// Length of a finished audio file, read from the file itself.
    static func fileDuration(at url: URL) -> TimeInterval? {
        guard let file = try? AVAudioFile(forReading: url) else { return nil }
        let rate = file.processingFormat.sampleRate
        guard rate > 0, file.length > 0 else { return nil }
        return Double(file.length) / rate
    }

    /// Stop the live recorder so its file is finalised and uploadable. The
    /// delegate adopts it; nothing here touches the audio.
    private func finaliseActive() {
        active?.stop()
        active = nil
    }

    /// Throw away a recorder that was armed for a boundary that will never
    /// arrive. It has recorded nothing, so there is no audio to lose.
    private func discardArmed() {
        guard let armed else { return }
        armed.stop()
        armed.deleteRecording()
        indexFor[ObjectIdentifier(armed)] = nil
        self.armed = nil
    }

    // ── Timers ───────────────────────────────────────────────────────────────

    private func startTimers() {
        meterTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
        // Cheap, and the only thing standing between "iOS quietly stopped the
        // recorder" and a meeting that looks recorded but is not.
        watchdog = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.resumeIfStalled() }
        }
    }

    private func stopTimers() {
        meterTimer?.invalidate(); meterTimer = nil
        watchdog?.invalidate(); watchdog = nil
    }

    private func tick() {
        guard let rec = active, rec.isRecording else {
            elapsed = completedDuration
            return
        }
        elapsed = completedDuration + rec.currentTime
        rec.updateMeters()
        let db = rec.averagePower(forChannel: 0)
        let level = min(1, CGFloat(pow(10, db / 20)) * 3.2)
        levels.removeFirst()
        levels.append(level)
    }
}

extension MeetingCapture: AVAudioRecorderDelegate {
    nonisolated func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        Task { @MainActor in self.segmentFinished(recorder, success: flag) }
    }

    nonisolated func audioRecorderEncodeErrorDidOccur(_ recorder: AVAudioRecorder, error: Error?) {
        Task { @MainActor in
            self.lastProblem = "The recorder hit an encoding error — everything already captured is safe."
            self.segmentFinished(recorder, success: false)
        }
    }
}
