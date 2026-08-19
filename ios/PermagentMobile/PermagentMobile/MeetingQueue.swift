// The durable capture queue — recorded audio is the SOURCE OF TRUTH, and the
// upload is something that happens to it later.
//
// Why this file exists (2026-08-18): a real meeting was lost. The Notes
// recorder wrote its clip into `FileManager.temporaryDirectory` and then
// deleted it from a `defer` that ran on the FAILURE path exactly as it ran on
// the success path — six lines above the error the user actually saw. The app
// said "couldn't reach your hub" and destroyed the only copy of the audio in
// the same breath. There was no queue, no retry, and no retention.
//
// The rule that replaces it, and the invariant every function here upholds:
//
//   Audio is written into Application Support, it is deleted ONLY after the
//   hub has answered with a transcript for that exact segment, and nothing
//   else in the app is allowed to delete it.
//
// Everything unsent therefore survives app termination and device restart:
// the manifest and the audio are both on disk, and the in-memory state is a
// cache of them rather than the other way round.
//
// Foundation only, deliberately. The test target compiles this file directly
// (see `project.yml`), so the state machine is exercised without an app
// bundle, a simulator host, a microphone or a hub.

import Foundation
// `ObservableObject` and `@Published` are Combine's, not Foundation's. The
// newer SDK re-exports them so `import Foundation` alone compiles there; the
// Swift 6.0 toolchain CI runs need not, and this file is the only one in the
// test target that uses them, so there is no precedent proving the re-export.
// Naming the dependency costs nothing and cannot regress.
import Combine

// ── Vocabulary ───────────────────────────────────────────────────────────────

/// What kind of capture a recording is. The two jobs wear the same microphone
/// today; they are not the same job, and the note they produce differs.
enum RecordingKind: String, Codable {
    /// Phone on the table, runs locked, chunked, filed as a meeting note
    /// (`kind: "meeting"`, which is what makes the hub extract follow-ups).
    case meeting
    /// A quick spoken note. Single clip, reviewed before it is filed.
    case note
}

/// Where one segment of audio has got to. `pending` and `uploading` both mean
/// "the audio file is still on disk"; only `sent` authorises deletion.
enum SegmentState: String, Codable {
    /// Captured, on disk, not yet accepted by the hub.
    case pending
    /// An upload is in flight. Reverted to `pending` at launch — a process
    /// that died mid-upload must not leave a segment nobody will retry.
    case uploading
    /// The hub returned a transcript for this exact segment. The transcript is
    /// persisted BEFORE the audio is deleted, never after.
    case sent
    /// The user explicitly gave up on this one so the rest of the meeting
    /// could be filed. The audio is KEPT — skipping is not deleting.
    case skipped
}

enum CaptureText {
    /// Left in the transcript where a segment never transcribed. Deliberately
    /// the same string the desktop recorder uses
    /// (`ui/command-center/src/hooks/useMeetingDictation.ts`) so a phone note
    /// and a desktop note read identically.
    static let gapMarker = "[… a segment could not be transcribed …]"

    /// e.g. "Meeting — 18 Aug 2026, 14:05". Pure, so the title a recovered
    /// recording gets is the title it would have got live.
    static func meetingTitle(startedAt: Date, calendar: Calendar = .current) -> String {
        let f = DateFormatter()
        f.calendar = calendar
        f.locale = .current
        f.setLocalizedDateFormatFromTemplate("d MMM yyyy, HH:mm")
        return "Meeting — \(f.string(from: startedAt))"
    }

    /// The title a stranded quick note gets when it is finally filed.
    static func noteTitle(startedAt: Date, calendar: Calendar = .current) -> String {
        let f = DateFormatter()
        f.calendar = calendar
        f.locale = .current
        f.setLocalizedDateFormatFromTemplate("d MMM yyyy, HH:mm")
        return "Dictated note — \(f.string(from: startedAt))"
    }

    /// h:mm:ss (or m:ss under an hour) for the elapsed readout.
    static func elapsed(_ seconds: TimeInterval) -> String {
        let s = max(0, Int(seconds))
        let h = s / 3600, m = (s % 3600) / 60, sec = s % 60
        return h > 0
            ? String(format: "%d:%02d:%02d", h, m, sec)
            : String(format: "%d:%02d", m, sec)
    }
}

// ── WAV, and the two length fields that decide whether a segment survives ────

/// Reading and repairing the RIFF header of a recorded segment.
///
/// A WAV writer stamps the RIFF size and the `data` chunk size when it CLOSES
/// the file. Kill the app mid-segment and those two fields still say "nothing
/// written yet" — and the hub's decoder rejects such a file outright rather
/// than reading what is plainly there. Measured against the daemon's own
/// `decode_audio_simple` on 2026-08-19: a 120-second recording with a stale
/// header fails with "wav: missing data chunk" and yields zero samples; the
/// same bytes with the two fields rewritten from the real file length decode
/// to all 120 seconds.
///
/// So this is not tidying. It is the difference between losing the piece of
/// the meeting that was being written when the phone died and keeping it.
///
/// Foundation only, and pure enough to test: it takes a file and fixes it.
enum WavInspector {

    /// Byte offsets of what matters in a RIFF/WAVE file.
    struct Layout: Equatable {
        /// Offset of the `data` chunk header (the four ASCII bytes).
        let dataChunkOffset: Int
        /// The size the header CLAIMS the audio payload is.
        let declaredDataSize: Int
        /// Bytes of audio per second, from the `fmt ` chunk.
        let byteRate: Int
        /// First byte of audio.
        var payloadOffset: Int { dataChunkOffset + 8 }
    }

    /// Parse enough of the header to locate the `data` chunk. `header` need
    /// only be the first few kilobytes — writers put `fmt `/`JUNK` first and
    /// `data` last.
    static func layout(of header: Data) -> Layout? {
        guard header.count >= 44 else { return nil }
        func ascii(_ offset: Int) -> String? {
            guard offset + 4 <= header.count else { return nil }
            return String(bytes: header[header.startIndex + offset ..< header.startIndex + offset + 4], encoding: .ascii)
        }
        func u32(_ offset: Int) -> Int? {
            guard offset + 4 <= header.count else { return nil }
            var value: UInt32 = 0
            for i in (0..<4).reversed() {
                value = (value << 8) | UInt32(header[header.startIndex + offset + i])
            }
            return Int(value)
        }
        guard ascii(0) == "RIFF", ascii(8) == "WAVE" else { return nil }

        var offset = 12
        var byteRate = 0
        // Bounded: a header with more chunks than this before `data` is not a
        // file this app wrote, and walking it forever is not a recovery.
        for _ in 0..<32 {
            guard let id = ascii(offset), let size = u32(offset + 4) else { return nil }
            if id == "fmt " {
                byteRate = u32(offset + 8 + 8) ?? 0   // fmt: format, channels, rate, THEN byte rate
            }
            if id == "data" {
                return Layout(dataChunkOffset: offset, declaredDataSize: size, byteRate: byteRate)
            }
            // Chunks are word-aligned.
            offset += 8 + size + (size % 2)
            if offset <= 12 || offset + 8 > header.count { return nil }
        }
        return nil
    }

    /// Rewrite the RIFF and `data` lengths from the file's real size when they
    /// disagree with it. Returns true if the file was changed.
    @discardableResult
    static func repairHeaderIfNeeded(at url: URL) -> Bool {
        guard let handle = FileHandle(forUpdatingAtPath: url.path) else { return false }
        defer { try? handle.close() }
        guard let fileSize = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.intValue,
              let header = try? handle.read(upToCount: 8_192),
              let layout = layout(of: header)
        else { return false }

        let actualPayload = fileSize - layout.payloadOffset
        guard actualPayload > 0, actualPayload != layout.declaredDataSize else { return false }

        func littleEndian(_ value: Int) -> Data {
            let v = UInt32(clamping: value)
            return Data([UInt8(v & 0xFF), UInt8((v >> 8) & 0xFF), UInt8((v >> 16) & 0xFF), UInt8((v >> 24) & 0xFF)])
        }
        do {
            try handle.seek(toOffset: 4)
            try handle.write(contentsOf: littleEndian(fileSize - 8))
            try handle.seek(toOffset: UInt64(layout.dataChunkOffset + 4))
            try handle.write(contentsOf: littleEndian(actualPayload))
            try handle.synchronize()
            return true
        } catch {
            return false
        }
    }

    /// Length of a recorded segment, from its own header. Used where pulling
    /// in an audio framework would be the wrong dependency.
    static func duration(at url: URL) -> TimeInterval? {
        guard let handle = FileHandle(forReadingAtPath: url.path) else { return nil }
        defer { try? handle.close() }
        guard let fileSize = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.intValue,
              let header = try? handle.read(upToCount: 8_192),
              let layout = layout(of: header), layout.byteRate > 0
        else { return nil }
        let payload = min(layout.declaredDataSize, fileSize - layout.payloadOffset)
        guard payload > 0 else { return nil }
        return TimeInterval(payload) / TimeInterval(layout.byteRate)
    }
}

// ── The two records the manifest is made of ──────────────────────────────────

/// One chunk of audio. Uploaded on its own, so a dropped connection or a dead
/// battery costs at most the chunk in flight — never the meeting.
struct CapturedSegment: Codable, Identifiable, Equatable {
    let id: UUID
    /// Position in the recording. Upload order and transcript order are both
    /// this, never arrival order.
    let index: Int
    /// Bare filename inside the recording's directory.
    let filename: String
    let duration: TimeInterval
    var state: SegmentState
    var attempts: Int
    var lastError: String?
    /// Set at the same moment `state` becomes `.sent`, and persisted before
    /// the audio file is removed.
    var transcript: String?

    init(
        id: UUID = UUID(),
        index: Int,
        filename: String,
        duration: TimeInterval,
        state: SegmentState = .pending,
        attempts: Int = 0,
        lastError: String? = nil,
        transcript: String? = nil
    ) {
        self.id = id
        self.index = index
        self.filename = filename
        self.duration = duration
        self.state = state
        self.attempts = attempts
        self.lastError = lastError
        self.transcript = transcript
    }

    /// True while this segment's audio file must still exist on disk. The one
    /// place the retention rule is written down.
    var holdsAudio: Bool { state != .sent }
}

/// One recording: the audio, where it is going, and how far it has got.
struct CapturedRecording: Codable, Identifiable, Equatable {
    let id: UUID
    let kind: RecordingKind
    let startedAt: Date
    /// Chosen BEFORE recording for meetings, so a recording that survives a
    /// crash already knows where it belongs. A quick note may have none yet.
    var projectId: String?
    var projectName: String?
    /// No further segments will be appended (the user stopped, or a relaunch
    /// found a recording whose process is gone).
    var isClosed: Bool
    var segments: [CapturedSegment]
    /// Set once the note has landed on the hub. A recording with a note id has
    /// nothing left to do.
    var savedNoteId: String?
    var lastError: String?

    init(
        id: UUID = UUID(),
        kind: RecordingKind,
        startedAt: Date,
        projectId: String? = nil,
        projectName: String? = nil,
        isClosed: Bool = false,
        segments: [CapturedSegment] = [],
        savedNoteId: String? = nil,
        lastError: String? = nil
    ) {
        self.id = id
        self.kind = kind
        self.startedAt = startedAt
        self.projectId = projectId
        self.projectName = projectName
        self.isClosed = isClosed
        self.segments = segments
        self.savedNoteId = savedNoteId
        self.lastError = lastError
    }
}

// ── The state machine: pure, and the part the regression tests pin ───────────

extension CapturedRecording {
    var ordered: [CapturedSegment] { segments.sorted { $0.index < $1.index } }

    var duration: TimeInterval { segments.reduce(0) { $0 + $1.duration } }

    var sentCount: Int { segments.filter { $0.state == .sent }.count }

    /// Segments whose words the hub has not taken yet. This is the number the
    /// UI must show: it is exactly how much of the meeting is still only on
    /// this phone.
    var waitingCount: Int { segments.filter { $0.state == .pending || $0.state == .uploading }.count }

    /// The next segment to upload — lowest index first, always. Upload order
    /// is transcript order; a segment is never overtaken by a later one.
    var nextToSend: CapturedSegment? {
        segments.filter { $0.state == .pending }.min { $0.index < $1.index }
    }

    /// Nothing left in flight: every segment has either transcribed or been
    /// explicitly given up on.
    var isDrained: Bool { !segments.isEmpty && waitingCount == 0 }

    /// Any real words at all. A recording of only gap markers is not a note.
    var hasWords: Bool {
        segments.contains {
            $0.state == .sent && !($0.transcript ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    /// The note body: transcribed segments in index order, with a gap marker
    /// where the user gave up on one. Recovered and live recordings compose
    /// identically — a recovered transcript must never read differently.
    var transcript: String {
        ordered.compactMap { seg -> String? in
            switch seg.state {
            case .sent:
                let t = (seg.transcript ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                return t.isEmpty ? nil : t
            case .skipped:
                return CaptureText.gapMarker
            case .pending, .uploading:
                return nil
            }
        }
        .joined(separator: " ")
        .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Ready for the note to be written to the hub.
    var isReadyToFile: Bool {
        isClosed && savedNoteId == nil && isDrained && hasWords && projectId != nil
    }

    /// Everything done — the note is filed and no audio is being held back.
    /// Only such a recording may be removed from the queue automatically.
    var isFinished: Bool {
        savedNoteId != nil && !segments.contains { $0.holdsAudio }
    }

    /// The ONLY sanctioned answer to "which audio files may be deleted".
    /// Everything else in the app asks this; nothing else decides.
    var deletableFilenames: [String] {
        segments.filter { !$0.holdsAudio }.map(\.filename)
    }

    // ── Transitions ──────────────────────────────────────────────────────────

    mutating func apply(_ change: (inout CapturedSegment) -> Void, to segmentId: UUID) {
        guard let i = segments.firstIndex(where: { $0.id == segmentId }) else { return }
        change(&segments[i])
    }

    mutating func markUploading(_ segmentId: UUID) {
        apply({ seg in
            guard seg.state == .pending else { return }
            seg.state = .uploading
            seg.attempts += 1
        }, to: segmentId)
    }

    /// The hub took this segment. The transcript is recorded here; the audio
    /// file is deleted by the store only AFTER this state has been persisted.
    mutating func markSent(_ segmentId: UUID, transcript: String) {
        apply({ seg in
            seg.state = .sent
            seg.transcript = transcript
            seg.lastError = nil
        }, to: segmentId)
        lastError = nil
    }

    /// The upload failed. Back to `pending` — never to a terminal state, and
    /// never anywhere that would let the audio be deleted.
    mutating func markFailed(_ segmentId: UUID, error: String) {
        apply({ seg in
            seg.state = .pending
            seg.lastError = error
        }, to: segmentId)
        lastError = error
    }

    /// The user chose to file what transcribed and stop retrying the rest. The
    /// audio stays on disk: giving up on the transcript is not giving up on
    /// the recording.
    mutating func skipRemaining() {
        for i in segments.indices where segments[i].state == .pending || segments[i].state == .uploading {
            segments[i].state = .skipped
        }
    }

    /// A process that died mid-upload leaves `uploading` segments nobody would
    /// retry. Run at launch, before anything reads `nextToSend`.
    mutating func recoverInterruptedUploads() {
        for i in segments.indices where segments[i].state == .uploading {
            segments[i].state = .pending
        }
    }
}

/// What happens to a just-recorded clip after an upload attempt.
///
/// This is the decision the lost meeting turned on, lifted out of the view so
/// it can be stated once and tested. The old code was, in effect,
/// `.deleteNow` unconditionally — a `defer` that could not tell success from
/// failure. There is exactly one condition under which audio may be deleted,
/// and it is written here.
enum ClipDisposition: Equatable {
    case deleteNow
    case retainForRetry
}

enum RetentionPolicy {
    /// `hubConfirmedReceipt` is true only when the hub answered 2xx with a
    /// transcript for this clip. Anything else — network failure, 5xx, 503 "no
    /// dictation model", an app crash — retains.
    static func disposition(hubConfirmedReceipt: Bool) -> ClipDisposition {
        hubConfirmedReceipt ? .deleteNow : .retainForRetry
    }
}

// ── The store: the manifest and the audio, on disk ───────────────────────────

/// The durable side of the queue. `@MainActor` because every caller is UI, and
/// because a single owner is what makes "persist, then delete" an order rather
/// than a race.
@MainActor
final class RecordingStore: ObservableObject {
    static let shared = RecordingStore()

    /// Newest last. Published so "N recordings waiting to send" is never a
    /// number somebody remembered to refresh.
    @Published private(set) var recordings: [CapturedRecording] = []

    let root: URL
    private let manifestURL: URL
    /// True once a manifest has been read or written successfully. The orphan
    /// sweep refuses to run while this is false: an unreadable manifest and an
    /// empty queue look identical from the outside, and mistaking one for the
    /// other would delete every recording on the phone.
    private var manifestTrusted = false

    /// `root` is injectable so tests run against a temp directory instead of
    /// the real Application Support.
    init(root: URL? = nil) {
        let base = root ?? Self.defaultRoot()
        self.root = base
        self.manifestURL = base.appendingPathComponent("manifest.json")
        try? Self.makeDirectory(base)
        load()
    }

    /// Application Support — durable, backed up, and NOT the temporary
    /// directory iOS is free to empty whenever it likes. The `defer`-deleted
    /// clip that started all this lived in the temporary directory; even
    /// without the `defer` it was never safe there.
    static func defaultRoot() -> URL {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        return support.appendingPathComponent("Recordings", isDirectory: true)
    }

    /// Recording must keep working with the phone face-down and locked, so the
    /// directory is created with the protection class that stays readable
    /// after the first unlock. `.complete` would make the recorder fail the
    /// moment the screen went dark — the exact scenario this feature is for.
    private static func makeDirectory(_ url: URL) throws {
        try FileManager.default.createDirectory(
            at: url,
            withIntermediateDirectories: true,
            attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
        )
    }

    // ── Paths ────────────────────────────────────────────────────────────────

    func directory(for recordingId: UUID) -> URL {
        root.appendingPathComponent(recordingId.uuidString, isDirectory: true)
    }

    func audioURL(recordingId: UUID, filename: String) -> URL {
        directory(for: recordingId).appendingPathComponent(filename)
    }

    func audioURL(recordingId: UUID, segment: CapturedSegment) -> URL {
        audioURL(recordingId: recordingId, filename: segment.filename)
    }

    /// Where a clip is written while it is being recorded and uploaded, before
    /// anyone knows whether it will need to be kept.
    ///
    /// It lives inside the durable root rather than in the temporary
    /// directory, so the answer to "what happens if the app dies here" is
    /// "the file is still there at launch" instead of "iOS may have emptied
    /// the directory". `recoverAfterLaunch` adopts anything left behind.
    var stagingDirectory: URL { root.appendingPathComponent(Self.stagingFolder, isDirectory: true) }
    static let stagingFolder = "staging"
    /// Filename prefix of a segment the recorder is still writing. It becomes
    /// `segment-NNNN` once adopted, so the two are told apart on sight — which
    /// is what lets launch recovery find the piece a kill left behind.
    static let inProgressPrefix = "recording-"

    func stageURL(fileExtension: String) -> URL {
        try? Self.makeDirectory(stagingDirectory)
        return stagingDirectory.appendingPathComponent("clip-\(UUID().uuidString).\(fileExtension)")
    }

    /// Keep a clip whose upload never confirmed. It becomes a closed,
    /// project-less recording in the queue: visible, retryable, and deletable
    /// by nobody but the user.
    ///
    /// This is the replacement for the `defer { try? removeItem(at: url) }`
    /// that destroyed a meeting on 2026-08-18. The failure path now KEEPS.
    @discardableResult
    func keepStranded(clipAt url: URL, kind: RecordingKind = .note, startedAt: Date = Date()) -> UUID? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let recording = begin(kind: kind, startedAt: startedAt)
        do {
            _ = try adopt(fileAt: url, into: recording.id, index: 0, duration: 0)
            close(recording.id)
            return recording.id
        } catch {
            // Could not move it — leave the file exactly where it is and drop
            // the empty manifest entry. An un-adopted file can be recovered;
            // a deleted one cannot.
            recordings.removeAll { $0.id == recording.id }
            persist()
            return nil
        }
    }

    /// Dispose of a staged clip according to the retention policy. The only
    /// caller that may pass `hubConfirmedReceipt: true` is one holding a
    /// transcript the hub returned for these exact bytes.
    func retire(clipAt url: URL, hubConfirmedReceipt: Bool, kind: RecordingKind = .note, startedAt: Date = Date()) {
        switch RetentionPolicy.disposition(hubConfirmedReceipt: hubConfirmedReceipt) {
        case .deleteNow:
            try? FileManager.default.removeItem(at: url)
        case .retainForRetry:
            keepStranded(clipAt: url, kind: kind, startedAt: startedAt)
        }
    }

    // ── Reading ──────────────────────────────────────────────────────────────

    func recording(_ id: UUID) -> CapturedRecording? { recordings.first { $0.id == id } }

    /// Recordings with anything still to do — the "N waiting to send" list.
    var unfinished: [CapturedRecording] { recordings.filter { !$0.isFinished } }

    /// Segments of audio that exist only on this phone, across everything.
    var segmentsWaiting: Int { recordings.reduce(0) { $0 + $1.waitingCount } }

    /// Recordings that still hold audio the hub has never seen.
    var recordingsWaiting: Int { recordings.filter { $0.waitingCount > 0 }.count }

    // ── Writing ──────────────────────────────────────────────────────────────

    @discardableResult
    func begin(
        kind: RecordingKind,
        startedAt: Date = Date(),
        projectId: String? = nil,
        projectName: String? = nil
    ) -> CapturedRecording {
        let rec = CapturedRecording(
            kind: kind, startedAt: startedAt,
            projectId: projectId, projectName: projectName
        )
        try? Self.makeDirectory(directory(for: rec.id))
        recordings.append(rec)
        persist()
        return rec
    }

    /// Move a finished audio file into the recording's directory and record it
    /// in the manifest. The move happens BEFORE the manifest is written, so a
    /// crash can leave an orphan file (harmless, swept at launch) but can
    /// never leave a manifest entry pointing at audio that does not exist.
    @discardableResult
    func adopt(
        fileAt source: URL,
        into recordingId: UUID,
        index: Int,
        duration: TimeInterval
    ) throws -> CapturedSegment {
        guard let slot = recordings.firstIndex(where: { $0.id == recordingId }) else {
            throw CocoaError(.fileNoSuchFile)
        }
        let dir = directory(for: recordingId)
        try Self.makeDirectory(dir)
        let filename = String(format: "segment-%04d.%@", index, source.pathExtension.isEmpty ? "m4a" : source.pathExtension)
        let destination = dir.appendingPathComponent(filename)
        if source.standardizedFileURL != destination.standardizedFileURL {
            try? FileManager.default.removeItem(at: destination)
            try FileManager.default.moveItem(at: source, to: destination)
        }
        let segment = CapturedSegment(index: index, filename: filename, duration: duration)
        recordings[slot].segments.append(segment)
        persist()
        return segment
    }

    func setTarget(_ recordingId: UUID, projectId: String, projectName: String) {
        mutate(recordingId) {
            $0.projectId = projectId
            $0.projectName = projectName
        }
    }

    /// The recording is over; no more segments are coming.
    func close(_ recordingId: UUID) {
        mutate(recordingId) { $0.isClosed = true }
    }

    func markUploading(_ recordingId: UUID, segment segmentId: UUID) {
        mutate(recordingId) { $0.markUploading(segmentId) }
    }

    /// The hub confirmed receipt. Transcript first (persisted), audio second —
    /// in that order, so a crash in between costs a stale file, never words.
    func confirmSent(_ recordingId: UUID, segment segmentId: UUID, transcript: String) {
        guard let slot = recordings.firstIndex(where: { $0.id == recordingId }),
              let seg = recordings[slot].segments.first(where: { $0.id == segmentId })
        else { return }
        recordings[slot].markSent(segmentId, transcript: transcript)
        persist()
        // Only now, and only for this exact segment.
        if case .deleteNow = RetentionPolicy.disposition(hubConfirmedReceipt: true) {
            try? FileManager.default.removeItem(at: audioURL(recordingId: recordingId, filename: seg.filename))
        }
    }

    /// The upload failed. Nothing is deleted, ever, on this path — this is the
    /// line the lost meeting died on.
    func markFailed(_ recordingId: UUID, segment segmentId: UUID, error: String) {
        mutate(recordingId) { $0.markFailed(segmentId, error: error) }
    }

    func skipRemaining(_ recordingId: UUID) {
        mutate(recordingId) { $0.skipRemaining() }
    }

    func markNoteSaved(_ recordingId: UUID, noteId: String) {
        mutate(recordingId) {
            $0.savedNoteId = noteId
            $0.lastError = nil
        }
        // Filed AND holding no audio → nothing left to keep.
        if let rec = recording(recordingId), rec.isFinished { remove(recordingId) }
    }

    func noteError(_ recordingId: UUID, _ message: String?) {
        mutate(recordingId) { $0.lastError = message }
    }

    /// Delete a recording and every byte of its audio. USER-INITIATED ONLY —
    /// nothing in the upload path may call this.
    func remove(_ recordingId: UUID) {
        try? FileManager.default.removeItem(at: directory(for: recordingId))
        recordings.removeAll { $0.id == recordingId }
        persist()
    }

    private func mutate(_ recordingId: UUID, _ change: (inout CapturedRecording) -> Void) {
        guard let slot = recordings.firstIndex(where: { $0.id == recordingId }) else { return }
        change(&recordings[slot])
        persist()
    }

    // ── Launch recovery ──────────────────────────────────────────────────────

    /// Run once at launch, before anything uploads.
    ///
    /// Two jobs. Uploads that were in flight when the process died go back to
    /// `pending` so they are retried rather than stranded. And a recording
    /// still marked open belongs to a process that no longer exists — the app
    /// was killed or the phone restarted mid-meeting — so it is closed here
    /// and its captured segments proceed to upload. Whatever was written to
    /// disk before the kill is kept and sent; only the segment being written
    /// at the instant of the kill is at risk.
    func recoverAfterLaunch() {
        // A manifest that would not decode is not an empty queue. Rebuild what
        // can be rebuilt from the directories themselves, so the audio stays
        // referenced (and therefore stays un-swept) instead of being orphaned
        // by a single bad byte.
        if !manifestTrusted { rebuildFromDisk() }
        for i in recordings.indices {
            recordings[i].recoverInterruptedUploads()
            recordings[i].isClosed = true
        }
        persist()
        adoptStrandedSegments()
        adoptStagedClips()
        sweepOrphanDirectories()
    }

    /// Segments a killed recorder wrote but never handed over.
    ///
    /// The file is sitting in the recording's own directory under its
    /// in-progress name, with a RIFF header that was never finalised. Nothing
    /// would ever have looked at it again. Repair the header — without which
    /// the hub rejects the whole piece — and put it in the queue at the end,
    /// where its filename order says it belongs.
    private func adoptStrandedSegments() {
        for slot in recordings.indices {
            let id = recordings[slot].id
            let files = ((try? FileManager.default.contentsOfDirectory(
                at: directory(for: id), includingPropertiesForKeys: nil, options: [.skipsHiddenFiles]
            )) ?? [])
                .filter { $0.lastPathComponent.hasPrefix(Self.inProgressPrefix) }
                .sorted { $0.lastPathComponent < $1.lastPathComponent }
            guard !files.isEmpty else { continue }

            var nextIndex = (recordings[slot].segments.map(\.index).max() ?? -1) + 1
            for file in files {
                WavInspector.repairHeaderIfNeeded(at: file)
                let size = ((try? FileManager.default.attributesOfItem(atPath: file.path))?[.size] as? NSNumber)?.intValue ?? 0
                guard size > 1_024 else {
                    // A stub with no audio in it — the only file this whole
                    // system is allowed to discard without being asked.
                    try? FileManager.default.removeItem(at: file)
                    continue
                }
                let duration = WavInspector.duration(at: file) ?? 0
                if (try? adopt(fileAt: file, into: id, index: nextIndex, duration: duration)) != nil {
                    nextIndex += 1
                }
            }
        }
    }

    /// Reconstruct the queue from what is on disk. Loses the metadata the
    /// manifest carried — which project, how long each piece was — but keeps
    /// the audio and its order, which is what cannot be recreated.
    private func rebuildFromDisk() {
        let entries = (try? FileManager.default.contentsOfDirectory(
            at: root, includingPropertiesForKeys: [.creationDateKey], options: [.skipsHiddenFiles]
        )) ?? []
        var rebuilt: [CapturedRecording] = []
        for dir in entries
        where dir.hasDirectoryPath && dir.lastPathComponent != Self.stagingFolder {
            guard let id = UUID(uuidString: dir.lastPathComponent) else { continue }
            let files = ((try? FileManager.default.contentsOfDirectory(
                at: dir, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles]
            )) ?? []).sorted { $0.lastPathComponent < $1.lastPathComponent }
            guard !files.isEmpty else { continue }
            let created = (try? dir.resourceValues(forKeys: [.creationDateKey]))?.creationDate ?? Date()
            var recording = CapturedRecording(id: id, kind: .meeting, startedAt: created, isClosed: true)
            // Segment filenames are zero-padded by index, so sorting by name
            // restores speech order.
            for (position, file) in files.enumerated() {
                recording.segments.append(
                    CapturedSegment(index: position, filename: file.lastPathComponent, duration: 0)
                )
            }
            rebuilt.append(recording)
        }
        recordings = rebuilt
        manifestTrusted = true
    }

    /// A clip left in staging belongs to a process that died between recording
    /// it and learning whether the hub took it. It is kept, not swept — the
    /// worst case is a duplicate note, and the alternative is the failure this
    /// whole feature exists to prevent.
    private func adoptStagedClips() {
        let files = (try? FileManager.default.contentsOfDirectory(
            at: stagingDirectory,
            includingPropertiesForKeys: [.creationDateKey],
            options: [.skipsHiddenFiles]
        )) ?? []
        for file in files where !file.hasDirectoryPath {
            let created = (try? file.resourceValues(forKeys: [.creationDateKey]))?.creationDate ?? Date()
            keepStranded(clipAt: file, kind: .note, startedAt: created)
        }
    }

    /// Directories with no manifest entry (a crash between the file move and
    /// the manifest write). Audio nothing references cannot be recovered and
    /// nothing will ever retry it.
    private func sweepOrphanDirectories() {
        guard manifestTrusted else { return }
        let known = Set(recordings.map(\.id.uuidString))
        let entries = (try? FileManager.default.contentsOfDirectory(
            at: root, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles]
        )) ?? []
        for url in entries
        where url.hasDirectoryPath
            && url.lastPathComponent != Self.stagingFolder
            && !known.contains(url.lastPathComponent) {
            try? FileManager.default.removeItem(at: url)
        }
    }

    // ── Persistence ──────────────────────────────────────────────────────────

    private func persist() {
        do {
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            let data = try encoder.encode(recordings)
            try data.write(to: manifestURL, options: [.atomic])
            try? FileManager.default.setAttributes(
                [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                ofItemAtPath: manifestURL.path
            )
        } catch {
            // A manifest we cannot write is bad, but the audio is still on
            // disk and the sweep will not delete a directory it cannot prove
            // is orphaned — see `sweepOrphanDirectories`, which only runs
            // against a manifest that loaded.
        }
    }

    private func load() {
        guard FileManager.default.fileExists(atPath: manifestURL.path) else {
            // No manifest yet is a legitimate empty queue on a fresh install.
            manifestTrusted = true
            return
        }
        guard let data = try? Data(contentsOf: manifestURL) else { return }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        guard let decoded = try? decoder.decode([CapturedRecording].self, from: data) else { return }
        recordings = decoded
        manifestTrusted = true
    }
}
