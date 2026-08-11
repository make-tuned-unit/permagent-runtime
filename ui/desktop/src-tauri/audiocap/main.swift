// permagent-audiocap — system-audio capture for meeting transcripts.
//
// WHY A SEPARATE BINARY. The app is Rust/Tauri, and ScreenCaptureKit's capture
// path is delegate-based (`SCStreamOutput`). Declaring an Objective-C class
// from Rust via objc2 to satisfy that protocol is possible but intricate and
// hard to verify; in Swift it is a conformance. A sidecar also fails safely —
// if capture dies, the meeting recorder keeps its microphone track instead of
// taking the app down with it.
//
// WHAT IT CAPTURES. The system's audio output — i.e. the OTHER participants on
// a call — which the microphone cannot hear. `excludesCurrentProcessAudio`
// keeps Permagent's own TTS out of the recording, so the agent never
// transcribes itself.
//
// OUTPUT. 16 kHz mono 16-bit WAV chunks (what the hub's local Whisper wants),
// one file per `--chunk-seconds`, written to `--out`. Each completed chunk's
// absolute path is printed to stdout on its own line, so the caller can stream
// them to `/api/dictation/transcribe` while the meeting is still running.
// Line-buffered and flushed per chunk: a consumer reading stdout gets each
// path as it lands, not at exit.
//
// PERMISSION. Screen Recording (TCC). macOS shows the prompt on first capture;
// audio-only still counts as screen capture and there is no audio-only grant.
// Absent permission, SCShareableContent throws — reported on stderr as
// `ERROR permission` so the caller can tell "user said no" from "it crashed".

import AVFoundation
import CoreMedia
import Foundation
import ScreenCaptureKit

let TARGET_RATE: Double = 16_000

// ── CLI ──────────────────────────────────────────────────────────────────────
var outDir = FileManager.default.temporaryDirectory.appendingPathComponent("permagent-audiocap").path
var chunkSeconds: Double = 45

var it = CommandLine.arguments.dropFirst().makeIterator()
while let a = it.next() {
    switch a {
    case "--out": outDir = it.next() ?? outDir
    case "--chunk-seconds": chunkSeconds = Double(it.next() ?? "") ?? chunkSeconds
    default: break
    }
}

func fail(_ kind: String, _ message: String) -> Never {
    FileErrorLog.write("ERROR \(kind) \(message)")
    exit(kind == "permission" ? 2 : 1)
}

enum FileErrorLog {
    static func write(_ s: String) {
        FileHandle.standardError.write((s + "\n").data(using: .utf8)!)
    }
}

/// Emit a completed chunk path. Flushed immediately — the consumer transcribes
/// while the meeting continues, so a buffered write would defeat the point.
func emit(_ path: String) {
    FileHandle.standardOutput.write((path + "\n").data(using: .utf8)!)
}

// ── WAV writer (16-bit PCM mono) ─────────────────────────────────────────────
// Hand-rolled rather than AVAudioFile so the header matches the hub's decoder
// byte for byte: `useDictation.ts:encodeWav` produces exactly this layout, and
// the daemon has only ever been fed that shape.
func writeWav(samples: [Int16], rate: Double, to url: URL) throws {
    var d = Data()
    func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
    func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
    let bytes = UInt32(samples.count * 2)
    d.append("RIFF".data(using: .ascii)!); u32(36 + bytes)
    d.append("WAVE".data(using: .ascii)!)
    d.append("fmt ".data(using: .ascii)!); u32(16); u16(1); u16(1)
    u32(UInt32(rate)); u32(UInt32(rate) * 2); u16(2); u16(16)
    d.append("data".data(using: .ascii)!); u32(bytes)
    samples.forEach { s in withUnsafeBytes(of: s.littleEndian) { d.append(contentsOf: $0) } }
    try d.write(to: url)
}

// ── Capture ──────────────────────────────────────────────────────────────────
final class Capture: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private var pending: [Float] = []
    private var sourceRate: Double = 48_000
    private var chunkIndex = 0
    private let lock = NSLock()
    private let outURL: URL
    private let chunkSamples: Int

    init(outDir: String, chunkSeconds: Double) throws {
        outURL = URL(fileURLWithPath: outDir, isDirectory: true)
        try FileManager.default.createDirectory(at: outURL, withIntermediateDirectories: true)
        chunkSamples = Int(TARGET_RATE * chunkSeconds)
        super.init()
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, CMSampleBufferDataIsReady(sb) else { return }
        guard let fmt = CMSampleBufferGetFormatDescription(sb),
              let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(fmt)?.pointee
        else { return }
        sourceRate = asbd.mSampleRate

        // SCKit delivers non-interleaved 32-bit float; each channel is its own
        // buffer in the list. Downmix to mono by averaging channels — a call
        // has the same speech in both, and Whisper takes mono.
        var blockBuffer: CMBlockBuffer?
        var abl = AudioBufferList()
        let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sb,
            bufferListSizeNeededOut: nil,
            bufferListOut: &abl,
            bufferListSize: MemoryLayout<AudioBufferList>.size,
            blockBufferAllocator: nil,
            blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            blockBufferOut: &blockBuffer
        )
        guard status == noErr else { return }

        let buffers = UnsafeMutableAudioBufferListPointer(&abl)
        guard let first = buffers.first, let base = first.mData else { return }
        let frames = Int(first.mDataByteSize) / MemoryLayout<Float>.size
        guard frames > 0 else { return }

        var mono = [Float](repeating: 0, count: frames)
        let chans = buffers.count
        for b in 0..<chans {
            guard let p = buffers[b].mData else { continue }
            let f = p.assumingMemoryBound(to: Float.self)
            for i in 0..<frames { mono[i] += f[i] }
        }
        if chans > 1 { for i in 0..<frames { mono[i] /= Float(chans) } }
        _ = base

        lock.lock()
        pending.append(contentsOf: mono)
        let needed = Int(sourceRate / TARGET_RATE * Double(chunkSamples))
        let ready = pending.count >= needed
        var take: [Float] = []
        if ready { take = Array(pending.prefix(needed)); pending.removeFirst(needed) }
        lock.unlock()

        if ready { flush(take, rate: sourceRate) }
    }

    /// Decimating resample to 16 kHz with box averaging. Not a polyphase
    /// filter — for speech into Whisper the aliasing this leaves is inaudible
    /// in the transcript, and a real FIR here would add a dependency and
    /// latency for no measurable word-error gain.
    private func flush(_ input: [Float], rate: Double) {
        guard !input.isEmpty else { return }
        let ratio = rate / TARGET_RATE
        let outCount = Int(Double(input.count) / ratio)
        var out = [Int16](); out.reserveCapacity(outCount)
        for i in 0..<outCount {
            let start = Int(Double(i) * ratio)
            let end = min(input.count, Int(Double(i + 1) * ratio))
            guard start < end else { continue }
            var acc: Float = 0
            for j in start..<end { acc += input[j] }
            let v = max(-1, min(1, acc / Float(end - start)))
            out.append(Int16(v < 0 ? v * 32768 : v * 32767))
        }
        guard !out.isEmpty else { return }
        let url = outURL.appendingPathComponent(String(format: "sysaudio_%04d.wav", chunkIndex))
        chunkIndex += 1
        do { try writeWav(samples: out, rate: TARGET_RATE, to: url); emit(url.path) }
        catch { FileErrorLog.write("ERROR write \(error.localizedDescription)") }
    }

    /// Write whatever is buffered — called on shutdown so the last, partial
    /// chunk is not silently dropped. A meeting that ends mid-chunk must still
    /// transcribe its final seconds.
    func finish() {
        lock.lock(); let rest = pending; pending = []; lock.unlock()
        flush(rest, rate: sourceRate)
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        FileErrorLog.write("ERROR stopped \(error.localizedDescription)")
        finish()
        exit(1)
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────
let sem = DispatchSemaphore(value: 0)
var capture: Capture?
var liveStream: SCStream?

Task {
    let content: SCShareableContent
    do {
        content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
    } catch {
        // The overwhelmingly common cause is a missing Screen Recording grant.
        fail("permission", error.localizedDescription)
    }
    guard let display = content.displays.first else { fail("nodisplay", "no display to attach to") }

    let cfg = SCStreamConfiguration()
    cfg.capturesAudio = true
    // Never record ourselves: the agent's own TTS would otherwise land in the
    // transcript and the model would summarise its own replies back to itself.
    cfg.excludesCurrentProcessAudio = true
    cfg.sampleRate = 48_000
    cfg.channelCount = 2
    // Audio-only still requires a video plane; make it as close to free as the
    // API permits (2x2 at 1fps) rather than capturing a real screen.
    cfg.width = 2
    cfg.height = 2
    cfg.minimumFrameInterval = CMTime(value: 1, timescale: 1)
    cfg.showsCursor = false

    let filter = SCContentFilter(display: display, excludingWindows: [])
    do {
        let c = try Capture(outDir: outDir, chunkSeconds: chunkSeconds)
        capture = c
        let s = SCStream(filter: filter, configuration: cfg, delegate: c)
        try s.addStreamOutput(c, type: .audio, sampleHandlerQueue: DispatchQueue(label: "ai.permagent.audiocap"))
        try await s.startCapture()
        liveStream = s
        FileErrorLog.write("READY capturing system audio → \(outDir)")
    } catch {
        fail("start", error.localizedDescription)
    }
}

// Flush-on-signal: the parent stops us with SIGTERM/SIGINT when the user hits
// Stop. Default disposition would kill us before `finish()`, losing the tail of
// the meeting, so both are trapped and handled.
var signalSources: [DispatchSourceSignal] = []
for sig in [SIGINT, SIGTERM] {
    signal(sig, SIG_IGN)
    let src = DispatchSource.makeSignalSource(signal: sig, queue: .main)
    src.setEventHandler {
        capture?.finish()
        if let s = liveStream { s.stopCapture { _ in sem.signal() } } else { sem.signal() }
    }
    src.resume()
    signalSources.append(src)
}

sem.wait()
