import AVFoundation
import Foundation

@MainActor
final class WatchRecorder: NSObject, ObservableObject, AVAudioRecorderDelegate {
    @Published var isRecording = false
    @Published var elapsed: TimeInterval = 0

    static let maxDuration: TimeInterval = 120

    var onFinish: ((URL?) -> Void)?

    private var recorder: AVAudioRecorder?
    private var fileURL: URL?
    private var tick: Timer?

    func requestPermission() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }

    func start() throws {
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
        rec.record(forDuration: Self.maxDuration)
        recorder = rec
        fileURL = file
        elapsed = 0
        isRecording = true
        tick = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self, let rec = self.recorder else { return }
                self.elapsed = rec.currentTime
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
            self.onFinish = nil
            self.teardown()
            finish?(url)
        }
    }

    private func teardown() {
        tick?.invalidate()
        tick = nil
        recorder = nil
        isRecording = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}
