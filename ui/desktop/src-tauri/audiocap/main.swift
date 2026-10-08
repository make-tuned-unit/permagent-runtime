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
// a call — which the microphone cannot hear. Exclude the parent application
// by PID/bundle identity, as excludesCurrentProcessAudio excludes this helper
// only. WebKit subprocess attribution must also be checked in a live TTS test.
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
import AppKit
import CoreMedia
import CoreGraphics
import Darwin
import Foundation
@preconcurrency import ScreenCaptureKit

let TARGET_RATE: Double = 16_000

// ── CLI ──────────────────────────────────────────────────────────────────────
var outDir = FileManager.default.temporaryDirectory.appendingPathComponent("permagent-audiocap").path
var chunkSeconds: Double = 45
var parentPID: pid_t = getppid()
var excludedBundleID: String?
var includedBundleID: String?
let scopedAudioProtocol = "PERMAGENT_AUDIOCAP_SCOPED_SOURCE_V1"
let existingPermissionProtocol = "PERMAGENT_AUDIOCAP_EXISTING_PERMISSION_V1"
var preflightOnly = false
var requireExistingPermission = false
var listSourceScope = false
var selfTest = false
var pcmDiagnostics = false

var it = CommandLine.arguments.dropFirst().makeIterator()
while let a = it.next() {
    switch a {
    case "--out": outDir = it.next() ?? outDir
    case "--chunk-seconds": chunkSeconds = Double(it.next() ?? "") ?? chunkSeconds
    case "--parent-pid": parentPID = pid_t(it.next() ?? "") ?? parentPID
    case "--exclude-bundle-id": excludedBundleID = it.next()
    case "--include-bundle-id": includedBundleID = it.next() ?? ""
    case "--self-test": selfTest = true
    case "--pcm-diagnostics": pcmDiagnostics = true
    case "--preflight-only": preflightOnly = true
    case "--require-existing-permission": requireExistingPermission = true
    case "--list-source-scope": listSourceScope = true
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

/// Exact application identity only: no prefix/glob matching, and never the parent.
func matchesIncludedApplication(bundleID: String, processID: pid_t,
                                requested: String, parent: pid_t, excludedBundle: String?) -> Bool {
    bundleID == requested && processID != parent && bundleID != excludedBundle
}

struct ScopedApplicationRoot {
    let processID: pid_t
    let bundlePath: String?
}

struct ScopedProcess {
    let parent: pid_t
    let executablePath: String
}

/// Resolve symlinks before comparing path components, so Chrome.app.other and
/// an executable symlink escaping Chrome.app are never treated as owned helpers.
func executableBelongsToApplication(_ executable: String, bundlePath: String) -> Bool {
    let bundle = URL(fileURLWithPath: bundlePath).resolvingSymlinksInPath().standardizedFileURL
    guard bundle.pathExtension == "app" else { return false }
    let contents = bundle.appendingPathComponent("Contents").pathComponents
    let path = URL(fileURLWithPath: executable).resolvingSymlinksInPath().standardizedFileURL.pathComponents
    return path.count > contents.count && Array(path.prefix(contents.count)) == contents
}

/// A helper needs BOTH a verified ancestry chain and executable ownership.
/// Bundle prefixes alone are never sufficient. Missing/exited processes fail
/// closed, and the selected app's root must still be present in SCK's inventory.
func selectedApplicationOwner(processID: pid_t, bundleID: String,
                              roots: [ScopedApplicationRoot], parent: pid_t,
                              excludedBundle: String?,
                              process: (pid_t) -> ScopedProcess?) -> pid_t? {
    guard processID != parent, bundleID != excludedBundle else { return nil }
    if let root = roots.first(where: { $0.processID == processID }) { return root.processID }
    for root in roots {
        guard let bundlePath = root.bundlePath else { continue }
        var current = processID
        var seen = Set<pid_t>()
        for _ in 0..<64 {
            guard current > 1, current != parent, seen.insert(current).inserted,
                  let info = process(current),
                  executableBelongsToApplication(info.executablePath, bundlePath: bundlePath)
            else { break }
            if info.parent == root.processID { return root.processID }
            current = info.parent
        }
    }
    return nil
}

func readScopedProcess(_ pid: pid_t) -> ScopedProcess? {
    var info = proc_bsdinfo()
    let infoSize = Int32(MemoryLayout<proc_bsdinfo>.size)
    guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, infoSize) == infoSize else { return nil }
    var path = [CChar](repeating: 0, count: 4096)
    let length = path.withUnsafeMutableBufferPointer { buffer in
        proc_pidpath(pid, buffer.baseAddress, UInt32(buffer.count))
    }
    guard length > 0 else { return nil }
    return ScopedProcess(parent: pid_t(info.pbi_ppid), executablePath: String(cString: path))
}

func scopedApplications(_ applications: [SCRunningApplication], requested: String) -> [SCRunningApplication] {
    let roots = applications.filter {
        matchesIncludedApplication(bundleID: $0.bundleIdentifier, processID: $0.processID,
            requested: requested, parent: parentPID, excludedBundle: excludedBundleID)
    }.map { app in
        ScopedApplicationRoot(processID: app.processID,
            bundlePath: NSRunningApplication(processIdentifier: app.processID)?.bundleURL?.path)
    }
    return applications.filter { app in
        let owner = selectedApplicationOwner(processID: app.processID, bundleID: app.bundleIdentifier,
            roots: roots, parent: parentPID, excludedBundle: excludedBundleID, process: readScopedProcess)
        if listSourceScope || owner != nil {
            // No window titles, executable paths, or audio contents in diagnostics.
            let row: [String: Any] = ["pid": app.processID, "bundleId": app.bundleIdentifier,
                "selected": owner != nil, "ownerPid": owner.map { Int($0) } ?? 0]
            if let data = try? JSONSerialization.data(withJSONObject: row, options: [.sortedKeys]),
               let json = String(data: data, encoding: .utf8) {
                FileErrorLog.write("SCOPE \(scopedAudioProtocol) \(json)")
            }
        }
        return owner != nil
    }
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

/// Decode either interleaved or planar Float32 PCM without assuming a one-buffer
/// AudioBufferList. Reject unsupported layouts instead of reinterpreting bytes.
func downmixFloatPCM(_ buffers: UnsafeMutableAudioBufferListPointer,
                     format: AudioStreamBasicDescription) -> [Float]? {
    guard format.mFormatID == kAudioFormatLinearPCM,
          format.mFormatFlags & kAudioFormatFlagIsFloat != 0,
          format.mBitsPerChannel == 32, format.mSampleRate >= TARGET_RATE,
          !buffers.isEmpty else { return nil }
    var frameCount: Int?
    var channels = 0
    for buffer in buffers {
        let count = Int(buffer.mNumberChannels)
        guard count > 0, buffer.mData != nil,
              Int(buffer.mDataByteSize) % (MemoryLayout<Float>.size * count) == 0 else { return nil }
        let frames = Int(buffer.mDataByteSize) / (MemoryLayout<Float>.size * count)
        if let expected = frameCount, frames != expected { return nil }
        frameCount = frames
        channels += count
    }
    guard let frames = frameCount, frames > 0, channels == Int(format.mChannelsPerFrame) else { return nil }
    var mono = [Float](repeating: 0, count: frames)
    for buffer in buffers {
        let samples = buffer.mData!.assumingMemoryBound(to: Float.self)
        let count = Int(buffer.mNumberChannels)
        for frame in 0..<frames {
            for channel in 0..<count {
                let value = samples[frame * count + channel]
                guard value.isFinite else { return nil }
                mono[frame] += value / Float(channels)
            }
        }
    }
    return mono
}

// ── Capture ──────────────────────────────────────────────────────────────────
// Optional diagnostics: aggregate amplitudes only, never sample values.
struct PCMAmplitudeStats {
    var count = 0
    var nonzero = 0
    var peak: Double = 0
    var squareSum: Double = 0
    mutating func add(_ value: Double) {
        count += 1
        if value != 0 { nonzero += 1 }
        peak = max(peak, abs(value))
        squareSum += value * value
    }
    var json: [String: Any] {
        ["sampleCount": count, "nonzeroCount": nonzero, "peak": peak,
         "rms": count == 0 ? 0 : sqrt(squareSum / Double(count))]
    }
}

final class Capture: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private var rawChannels: [PCMAmplitudeStats] = []
    private var monoStats = PCMAmplitudeStats()
    private var resampledStats = PCMAmplitudeStats()
    private var quantizedStats = PCMAmplitudeStats()
    private var callbackCount = 0
    private var nextDiagnosticTime = 0.0
    private var formatReported = false
    private func reportPCM(_ phase: String) {
        guard pcmDiagnostics else { return }
        let report: [String: Any] = ["phase": phase, "callbacks": callbackCount,
            "rawChannels": rawChannels.map { $0.json }, "downmixed": monoStats.json,
            "resampledFloat": resampledStats.json, "quantizedInt16": quantizedStats.json]
        if let data = try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]),
           let line = String(data: data, encoding: .utf8) {
            FileErrorLog.write("PCM_DIAGNOSTIC_V1 \(line)")
        }
    }

    private var pending: [Float] = []
    private var sourceRate: Double = 48_000
    private var chunkIndex = 0
    let sampleQueue = DispatchQueue(label: "ai.permagent.audiocap.samples")
    private var finished = false
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
        guard !finished else { return }
        sourceRate = asbd.mSampleRate

        var neededSize = 0
        var blockBuffer: CMBlockBuffer?
        let sizing = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sb, bufferListSizeNeededOut: &neededSize, bufferListOut: nil,
            bufferListSize: 0, blockBufferAllocator: nil, blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            blockBufferOut: nil)
        guard sizing == noErr, neededSize >= MemoryLayout<AudioBufferList>.size else {
            FileErrorLog.write("ERROR format unable to size audio buffer list (\(sizing))")
            return
        }
        let storage = UnsafeMutableRawPointer.allocate(byteCount: neededSize,
                                                        alignment: MemoryLayout<AudioBufferList>.alignment)
        defer { storage.deallocate() }
        let list = storage.bindMemory(to: AudioBufferList.self, capacity: 1)
        let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sb, bufferListSizeNeededOut: nil, bufferListOut: list,
            bufferListSize: neededSize, blockBufferAllocator: nil, blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            blockBufferOut: &blockBuffer)
        guard status == noErr,
              let mono = downmixFloatPCM(UnsafeMutableAudioBufferListPointer(list), format: asbd) else {
            FileErrorLog.write("ERROR format unsupported audio buffer layout (\(status))")
            return
        }
        if pcmDiagnostics {
            // Observe the unmodified buffer values independently of the downmix.
            let buffers = UnsafeMutableAudioBufferListPointer(list)
            if !formatReported {
                formatReported = true
                let report: [String: Any] = ["sampleRate": asbd.mSampleRate,
                    "formatID": asbd.mFormatID, "formatFlags": asbd.mFormatFlags,
                    "bitsPerChannel": asbd.mBitsPerChannel, "channels": asbd.mChannelsPerFrame,
                    "bytesPerFrame": asbd.mBytesPerFrame, "sampleBufferFrames": CMSampleBufferGetNumSamples(sb),
                    "bufferChannels": buffers.map { $0.mNumberChannels },
                    "bufferBytes": buffers.map { $0.mDataByteSize }]
                if let data = try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]),
                   let line = String(data: data, encoding: .utf8) {
                    FileErrorLog.write("PCM_FORMAT_V1 \(line)")
                }
            }
            if rawChannels.isEmpty {
                rawChannels = Array(repeating: PCMAmplitudeStats(), count: Int(asbd.mChannelsPerFrame))
            }
            guard rawChannels.count == Int(asbd.mChannelsPerFrame) else {
                FileErrorLog.write("ERROR diagnostic source channel count changed")
                return
            }
            var channelOffset = 0
            for buffer in buffers {
                let channels = Int(buffer.mNumberChannels)
                let frames = Int(buffer.mDataByteSize) / (MemoryLayout<Float>.size * channels)
                let values = buffer.mData!.assumingMemoryBound(to: Float.self)
                for frame in 0..<frames {
                    for channel in 0..<channels {
                        rawChannels[channelOffset + channel].add(Double(values[frame * channels + channel]))
                    }
                }
                channelOffset += channels
            }
            for value in mono { monoStats.add(Double(value)) }
            callbackCount += 1
            let now = ProcessInfo.processInfo.systemUptime
            if now >= nextDiagnosticTime {
                reportPCM("receiving")
                nextDiagnosticTime = now + 5
            }
        }
        // All sample/tail operations run on sampleQueue, including chunk numbering and writes.
        pending.append(contentsOf: mono)
        let needed = Int(sourceRate / TARGET_RATE * Double(chunkSamples))
        while pending.count >= needed {
            let take = Array(pending.prefix(needed))
            pending.removeFirst(needed)
            flush(take, rate: sourceRate)
        }
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
            let quantized = Int16(v < 0 ? v * 32768 : v * 32767)
            if pcmDiagnostics {
                resampledStats.add(Double(v))
                quantizedStats.add(Double(quantized))
            }
            out.append(quantized)
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
    func finish(completion: @escaping @Sendable () -> Void) {
        sampleQueue.async {
            if !self.finished {
                self.finished = true
                let rest = self.pending
                self.pending = []
                self.flush(rest, rate: self.sourceRate)
                self.reportPCM("final")
            }
            completion()
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        FileErrorLog.write("ERROR stopped \(error.localizedDescription)")
        finish { exit(1) }
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────
guard chunkSeconds.isFinite, (1...300).contains(chunkSeconds) else {
    fail("arguments", "chunk duration must be between 1 and 300 seconds")
}

// Fixture mode never requests permissions or starts ScreenCaptureKit.
if selfTest {
    var format = AudioStreamBasicDescription(mSampleRate: 48_000, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | kAudioFormatFlagIsNonInterleaved,
        mBytesPerPacket: 4, mFramesPerPacket: 1, mBytesPerFrame: 4,
        mChannelsPerFrame: 2, mBitsPerChannel: 32, mReserved: 0)
    var left: [Float] = [0.2, 0.4, -0.8]
    var right: [Float] = [0.4, 0.8, 0.2]
    let planarStorage = UnsafeMutableRawPointer.allocate(
        byteCount: MemoryLayout<AudioBufferList>.size + MemoryLayout<AudioBuffer>.stride,
        alignment: MemoryLayout<AudioBufferList>.alignment)
    defer { planarStorage.deallocate() }
    let planarList = planarStorage.bindMemory(to: AudioBufferList.self, capacity: 1)
    planarList.pointee.mNumberBuffers = 2
    let list = UnsafeMutableAudioBufferListPointer(planarList)
    left.withUnsafeMutableBytes { l in right.withUnsafeMutableBytes { r in
        list[0] = AudioBuffer(mNumberChannels: 1, mDataByteSize: UInt32(l.count), mData: l.baseAddress)
        list[1] = AudioBuffer(mNumberChannels: 1, mDataByteSize: UInt32(r.count), mData: r.baseAddress)
        let mono = downmixFloatPCM(list, format: format)!
        precondition(zip(mono, [Float(0.3), 0.6, -0.3]).allSatisfy { abs($0 - $1) < 0.00001 })
        // Build a real CoreMedia stereo sample. The old fixed one-buffer list
        // must fail ArrayTooSmall; the queried allocation must decode both sides.
        var description: CMAudioFormatDescription?
        precondition(CMAudioFormatDescriptionCreate(allocator: kCFAllocatorDefault,
            asbd: &format, layoutSize: 0, layout: nil, magicCookieSize: 0,
            magicCookie: nil, extensions: nil, formatDescriptionOut: &description) == noErr)
        var sample: CMSampleBuffer?
        var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 48_000), presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
        precondition(CMSampleBufferCreate(allocator: kCFAllocatorDefault, dataBuffer: nil,
            dataReady: false, makeDataReadyCallback: nil, refcon: nil,
            formatDescription: description, sampleCount: 3, sampleTimingEntryCount: 1,
            sampleTimingArray: &timing, sampleSizeEntryCount: 0, sampleSizeArray: nil,
            sampleBufferOut: &sample) == noErr)
        let setStatus = CMSampleBufferSetDataBufferFromAudioBufferList(sample!,
            blockBufferAllocator: kCFAllocatorDefault, blockBufferMemoryAllocator: kCFAllocatorDefault,
            flags: 0, bufferList: list.unsafePointer)
        precondition(setStatus == noErr, "set audio buffer status: \(setStatus)")
        var required = 0
        var fixedList = AudioBufferList()
        var retained: CMBlockBuffer?
        let undersized = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(sample!,
            bufferListSizeNeededOut: &required, bufferListOut: &fixedList,
            bufferListSize: MemoryLayout<AudioBufferList>.size, blockBufferAllocator: nil,
            blockBufferMemoryAllocator: nil, flags: 0, blockBufferOut: &retained)
        precondition(undersized == kCMSampleBufferError_ArrayTooSmall)
        precondition(required > MemoryLayout<AudioBufferList>.size)
        let dynamicStorage = UnsafeMutableRawPointer.allocate(byteCount: required,
            alignment: MemoryLayout<AudioBufferList>.alignment)
        defer { dynamicStorage.deallocate() }
        let dynamicList = dynamicStorage.bindMemory(to: AudioBufferList.self, capacity: 1)
        precondition(CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(sample!,
            bufferListSizeNeededOut: nil, bufferListOut: dynamicList,
            bufferListSize: required, blockBufferAllocator: nil,
            blockBufferMemoryAllocator: nil, flags: 0, blockBufferOut: &retained) == noErr)
        let decoded = downmixFloatPCM(UnsafeMutableAudioBufferListPointer(dynamicList), format: format)!
        precondition(zip(decoded, mono).allSatisfy { abs($0 - $1) < 0.00001 })
        list[1].mDataByteSize = 4
        precondition(downmixFloatPCM(list, format: format) == nil)
    } }
    var interleaved: [Float] = [0.2, 0.4, 0.4, 0.8, -0.8, 0.2]
    var interleavedList = AudioBufferList()
    interleavedList.mNumberBuffers = 1
    let stereo = UnsafeMutableAudioBufferListPointer(&interleavedList)
    format.mFormatFlags = kAudioFormatFlagIsFloat
    format.mBytesPerFrame = 8
    format.mBytesPerPacket = 8
    interleaved.withUnsafeMutableBytes { data in
        stereo[0] = AudioBuffer(mNumberChannels: 2, mDataByteSize: UInt32(data.count), mData: data.baseAddress)
        let mono = downmixFloatPCM(stereo, format: format)!
        precondition(zip(mono, [Float(0.3), 0.6, -0.3]).allSatisfy { abs($0 - $1) < 0.00001 })
    }
    let source = "ai.permagent.record-practice-20261007"
    precondition(matchesIncludedApplication(bundleID: source, processID: 12, requested: source, parent: 34, excludedBundle: "ai.permagent.desktop"))
    precondition(!matchesIncludedApplication(bundleID: source + ".other", processID: 12, requested: source, parent: 34, excludedBundle: nil))
    precondition(!matchesIncludedApplication(bundleID: source, processID: 34, requested: source, parent: 34, excludedBundle: nil))
    precondition(!matchesIncludedApplication(bundleID: source, processID: 12, requested: source, parent: 34, excludedBundle: source))
    let rootPath = "/Applications/Scope Test.app"
    let roots = [ScopedApplicationRoot(processID: 100, bundlePath: rootPath)]
    let helperPath = rootPath + "/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
    let processes: [pid_t: ScopedProcess] = [
        101: ScopedProcess(parent: 100, executablePath: helperPath),
        102: ScopedProcess(parent: 101, executablePath: helperPath),
        103: ScopedProcess(parent: 999, executablePath: helperPath),
        104: ScopedProcess(parent: 100, executablePath: "/Applications/Other.app/Contents/MacOS/Helper"),
        105: ScopedProcess(parent: 100, executablePath: rootPath + ".other/Contents/MacOS/Helper"),
        106: ScopedProcess(parent: 107, executablePath: helperPath),
        107: ScopedProcess(parent: 106, executablePath: helperPath),
        108: ScopedProcess(parent: 104, executablePath: helperPath),
    ]
    func owner(_ pid: pid_t, bundle: String = "test.app.helper", excluded: String? = nil) -> pid_t? {
        selectedApplicationOwner(processID: pid, bundleID: bundle, roots: roots,
            parent: 34, excludedBundle: excluded, process: { processes[$0] })
    }
    precondition(owner(100, bundle: "test.app") == 100)
    precondition(owner(101) == 100 && owner(102) == 100) // direct and nested owned helpers
    precondition(owner(103) == nil) // same executable, unrelated process ancestry
    precondition(owner(104) == nil && owner(105) == nil) // child outside app / prefix collision
    precondition(owner(106) == nil && owner(108) == nil) // cycle / outside intermediary
    precondition(owner(109) == nil && owner(34) == nil) // absent process / capturing parent
    precondition(owner(101, excluded: "test.app.helper") == nil)
    precondition(selectedApplicationOwner(processID: 101, bundleID: "test.app.helper", roots: [],
        parent: 34, excludedBundle: nil, process: { processes[$0] }) == nil)
    let testDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: testDir) }
    let contents = testDir.appendingPathComponent("Scope.app/Contents")
    try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
    let outside = testDir.appendingPathComponent("outside")
    try Data([0]).write(to: outside)
    let escape = contents.appendingPathComponent("escaped-helper")
    try FileManager.default.createSymbolicLink(at: escape, withDestinationURL: outside)
    precondition(!executableBelongsToApplication(escape.path, bundlePath: contents.deletingLastPathComponent().path))
    // Prove the diagnostics distinguish real silence, anti-phase cancellation,
    // and a nonzero input too quiet to survive Int16 quantization.
    var silence = PCMAmplitudeStats()
    [0.0, 0.0, 0.0].forEach { silence.add($0) }
    precondition(silence.count == 3 && silence.nonzero == 0 && silence.peak == 0)
    var signal = PCMAmplitudeStats()
    [0.5, -0.5, 0.0].forEach { signal.add($0) }
    precondition(signal.count == 3 && signal.nonzero == 2 && signal.peak == 0.5)
    precondition(abs(signal.squareSum - 0.5) < 0.000001)
    var antiPhase: [Float] = [0.5, -0.5, -0.5, 0.5]
    antiPhase.withUnsafeMutableBytes { data in
        stereo[0] = AudioBuffer(mNumberChannels: 2, mDataByteSize: UInt32(data.count), mData: data.baseAddress)
        let mono = downmixFloatPCM(stereo, format: format)!
        precondition(mono.allSatisfy { $0 == 0 })
        var raw = PCMAmplitudeStats()
        let samples = data.bindMemory(to: Float.self)
        for value in samples { raw.add(Double(value)) }
        precondition(raw.nonzero == 4 && raw.peak == 0.5)
    }
    let quiet: Float = 0.000001
    precondition(quiet != 0 && Int16(quiet * 32767) == 0)
    print("PASS planar stereo, interleaved stereo, mismatched channel rejection, CoreMedia undersized-list regression, exact application scope, owned descendant scope and escape rejection, PCM silence/phase/quantization diagnostics")
    exit(0)
}

// The bounded amplitude probe is only authorized for exact Chrome ownership;
// its permission preflight must run under the same helper identity as capture.
if pcmDiagnostics && (includedBundleID != "com.google.Chrome" || !requireExistingPermission) {
    fail("arguments", "--pcm-diagnostics requires exact Chrome scope and --require-existing-permission")
}

// Diagnostic/QA callers must never trigger a new permission request. This is
// the public non-prompting access check (CGWindow.h), not CGRequestScreenCaptureAccess.
// Keep it in the helper itself so the checked identity is the capturing process.
if listSourceScope && (includedBundleID == nil || includedBundleID?.isEmpty == true) {
    fail("arguments", "--list-source-scope requires --include-bundle-id; no capture was started")
}
if preflightOnly || requireExistingPermission || listSourceScope {
    guard CGPreflightScreenCaptureAccess() else {
        fail("permission", "\(existingPermissionProtocol) existing screen capture access absent; no permission was requested")
    }
    FileErrorLog.write("PREFLIGHT \(existingPermissionProtocol) granted")
    if preflightOnly { exit(0) }
}

let sem = DispatchSemaphore(value: 0)
let lifecycleQueue = DispatchQueue(label: "ai.permagent.audiocap.lifecycle")
// Accessed only on lifecycleQueue, including async startup completion.
var capture: Capture?
var liveStream: SCStream?
var stopping = false
func stopCapture() {
    guard !stopping else { return }
    stopping = true
    guard let s = liveStream, let c = capture else { sem.signal(); return }
    // Stop delivery first, then drain the sample queue and flush its tail exactly once.
    s.stopCapture { error in
        if let error { FileErrorLog.write("ERROR stop \(error.localizedDescription)") }
        c.finish { sem.signal() }
    }
}

// The main thread waits below. Signal handlers must use a separate queue; .main
// would never run while sem.wait() is blocking it. Install before starting capture.
var signalSources: [DispatchSourceSignal] = []
for sig in [SIGINT, SIGTERM] {
    signal(sig, SIG_IGN)
    let src = DispatchSource.makeSignalSource(signal: sig, queue: lifecycleQueue)
    src.setEventHandler { stopCapture() }
    src.resume()
    signalSources.append(src)
}
let parentMonitor = DispatchSource.makeTimerSource(queue: lifecycleQueue)
parentMonitor.schedule(deadline: .now() + 1, repeating: 1)
parentMonitor.setEventHandler {
    if getppid() != parentPID || (kill(parentPID, 0) != 0 && errno == ESRCH) { stopCapture() }
}
parentMonitor.resume()

Task {
    let content: SCShareableContent
    do { content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false) }
    catch { fail("permission", error.localizedDescription) }
    var selected: [SCRunningApplication]?
    if let requested = includedBundleID {
        guard !requested.isEmpty, requested != excludedBundleID else {
            fail("source", "A distinct call application must be selected")
        }
        let included = scopedApplications(content.applications, requested: requested)
        guard !included.isEmpty else {
            fail("source", "Selected call application is not available: \(requested). Open its call window and try again; no system audio was recorded.")
        }
        selected = included
        if listSourceScope {
            FileErrorLog.write("SCOPE_ONLY \(scopedAudioProtocol) selected=\(included.count); no stream created")
            exit(0)
        }
    }
    guard let display = content.displays.first else { fail("nodisplay", "no display to attach to") }
    let cfg = SCStreamConfiguration()
    cfg.capturesAudio = true
    cfg.excludesCurrentProcessAudio = true // helper itself, not the parent
    cfg.sampleRate = 48_000
    cfg.channelCount = 2
    cfg.width = 2
    cfg.height = 2
    cfg.minimumFrameInterval = CMTime(value: 1, timescale: 1)
    cfg.showsCursor = false
    let excluded = content.applications.filter {
        $0.processID == parentPID || (excludedBundleID != nil && $0.bundleIdentifier == excludedBundleID)
    }
    let filter: SCContentFilter
    if let requested = includedBundleID, let included = selected {
        filter = SCContentFilter(display: display, including: included, exceptingWindows: [])
        FileErrorLog.write("SOURCE \(scopedAudioProtocol) application \(requested)")
    } else {
        filter = SCContentFilter(display: display, excludingApplications: excluded, exceptingWindows: [])
    }
    do {
        let c = try Capture(outDir: outDir, chunkSeconds: chunkSeconds)
        let s = SCStream(filter: filter, configuration: cfg, delegate: c)
        try s.addStreamOutput(c, type: .audio, sampleHandlerQueue: c.sampleQueue)
        try await s.startCapture()
        lifecycleQueue.async {
            capture = c
            liveStream = s
            if stopping {
                s.stopCapture { _ in c.finish { sem.signal() } }
            } else {
                FileErrorLog.write("READY capturing system audio → \(outDir); excluded applications: \(excluded.count)")
            }
        }
    } catch { fail("start", error.localizedDescription) }
}
sem.wait()
