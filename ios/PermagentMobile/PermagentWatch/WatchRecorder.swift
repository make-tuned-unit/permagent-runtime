import AVFoundation
import Foundation

@MainActor
final class WatchRecorder: NSObject, ObservableObject, AVAudioRecorderDelegate {
    @Published var isRecording = false
    @Published var elapsed: TimeInterval = 0
    /// 0…1 mic level for the orb. Derived from dBFS, not a raw RMS.
    @Published var level: Float = 0

    var onFinish: ((URL?) -> Void)?
    var endpoint: WatchEndpoint = .chat
    private(set) var heardSpeech = false

    private var recorder: AVAudioRecorder?
    private var fileURL: URL?
    private var meterTask: Task<Void, Never>?
    private var spokenFor: TimeInterval = 0
    private var silentFor: TimeInterval = 0
    private var lastTick: TimeInterval = 0

    func requestPermission() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }

    func start() throws {
        if recorder != nil {
            recorder?.delegate = nil
            recorder?.stop()
            cancelMeter()
            recorder = nil
            isRecording = false
        }
        cancelMeter()
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.record, mode: .default)
        try session.setActive(true)

        let url = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        let file = url.appendingPathComponent("watch-\(UUID().uuidString).wav")
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 16_000.0,
            AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
        ]
        let rec = try AVAudioRecorder(url: file, settings: settings)
        rec.delegate = self
        rec.isMeteringEnabled = true
        rec.record(forDuration: endpoint.maxDuration)
        recorder = rec
        fileURL = file
        elapsed = 0
        level = 0
        heardSpeech = false
        spokenFor = 0
        silentFor = 0
        lastTick = 0
        isRecording = true
        meterTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self, self.isRecording else { break }
                self.tickMeters()
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
    }

    func stop() { recorder?.stop() }

    func cancel() {
        onFinish = nil
        recorder?.stop()
        recorder?.deleteRecording()
        teardown()
    }

    nonisolated func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        Task { @MainActor in
            let url = flag ? self.fileURL : nil
            let finish = self.onFinish
            self.teardown()
            finish?(url)
        }
    }

    private func tickMeters() {
        guard let rec = recorder else { return }
        rec.updateMeters()
        let db = rec.averagePower(forChannel: 0)
        // dBFS: roughly −50 silence, −12 loud speech.
        let normalized = min(1, max(0, (db + 50) / 38))
        level = normalized.isFinite ? normalized : 0
        elapsed = rec.currentTime

        let now = elapsed
        let dt = lastTick > 0 ? max(0, now - lastTick) : 0.1
        lastTick = now
        let voiced = db > -38
        if voiced {
            heardSpeech = true
            spokenFor += dt
            silentFor = 0
        } else {
            silentFor += dt
        }
        if endpoint.shouldStop(heardSpeech: heardSpeech, spokenFor: spokenFor,
                               silentFor: silentFor, elapsed: elapsed) {
            rec.stop()
        }
    }

    private func cancelMeter() {
        meterTask?.cancel()
        meterTask = nil
    }

    private func teardown() {
        cancelMeter()
        recorder = nil
        isRecording = false
        level = 0
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}
