// MeetingQueueTests — the rules that were not in place when a real meeting was
// lost on 2026-08-18.
//
// The headline is `audioSurvivesAFailedUpload`. On that day the app reported
// "Couldn't reach your hub — is your Mac awake and on the tailnet?" and, six
// lines earlier in the same function, deleted the recording it had just failed
// to send: `defer { try? FileManager.default.removeItem(at: url) }` ran on the
// failure path exactly as it ran on the success path. These tests pin the
// opposite behaviour at both levels — the policy that decides, and the store
// that carries it out — and they fail against that code by construction: it
// had no seam at which "did the hub actually take this?" could be asked.
//
// Everything here is Foundation and a temporary directory. No microphone, no
// hub, no app target.

import XCTest

@MainActor
final class MeetingQueueTests: XCTestCase {

    private var root: URL!

    override func setUp() {
        super.setUp()
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent("meeting-queue-tests-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: root)
        super.tearDown()
    }

    private func makeStore() -> RecordingStore { RecordingStore(root: root) }

    /// A stand-in for a finished segment of audio. Contents are irrelevant;
    /// what matters is whether the file is still there afterwards.
    @discardableResult
    private func writeClip(at url: URL, bytes: Int = 4_096) -> URL {
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true
        )
        FileManager.default.createFile(atPath: url.path, contents: Data(repeating: 0x41, count: bytes))
        return url
    }

    private func stagedClip(_ store: RecordingStore, bytes: Int = 4_096) -> URL {
        writeClip(at: store.stageURL(fileExtension: "m4a"), bytes: bytes)
    }

    private func exists(_ url: URL) -> Bool { FileManager.default.fileExists(atPath: url.path) }

    // ── THE REGRESSION ───────────────────────────────────────────────────────

    /// The lost meeting, as a test. A segment is recorded, the upload fails,
    /// and the audio must still be on the phone afterwards — available to
    /// retry, and gone only if the user says so.
    func testAudioSurvivesAFailedUpload() throws {
        let store = makeStore()
        let recording = store.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        let segment = try store.adopt(
            fileAt: stagedClip(store), into: recording.id, index: 0, duration: 120
        )
        let audio = store.audioURL(recordingId: recording.id, segment: segment)
        XCTAssertTrue(exists(audio), "precondition: the recorded segment is on disk")

        store.markUploading(recording.id, segment: segment.id)
        store.markFailed(recording.id, segment: segment.id, error: "Couldn't reach your hub")

        XCTAssertTrue(
            exists(audio),
            "a failed upload must NEVER delete the recording — this is the 2026-08-18 data loss"
        )
        let after = try XCTUnwrap(store.recording(recording.id))
        XCTAssertEqual(after.segments.first?.state, .pending, "a failure returns the segment to the queue")
        XCTAssertEqual(after.waitingCount, 1)
        XCTAssertEqual(after.nextToSend?.id, segment.id, "and it is the next thing that will be retried")
        XCTAssertTrue(after.deletableFilenames.isEmpty, "nothing about this recording is deletable yet")
    }

    /// The same rule stated where the decision is made, rather than where it
    /// is carried out. The old code was, in effect, `.deleteNow` regardless.
    func testRetentionPolicyDeletesOnlyOnConfirmedReceipt() {
        XCTAssertEqual(RetentionPolicy.disposition(hubConfirmedReceipt: false), .retainForRetry)
        XCTAssertEqual(RetentionPolicy.disposition(hubConfirmedReceipt: true), .deleteNow)
    }

    /// The other half of the contract: once the hub HAS the words, the audio
    /// goes — otherwise the phone fills up with meetings that already landed.
    func testAudioIsDeletedOnlyAfterTheHubConfirms() throws {
        let store = makeStore()
        let recording = store.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        let segment = try store.adopt(
            fileAt: stagedClip(store), into: recording.id, index: 0, duration: 120
        )
        let audio = store.audioURL(recordingId: recording.id, segment: segment)

        store.markUploading(recording.id, segment: segment.id)
        XCTAssertTrue(exists(audio), "an upload in flight is not a confirmation")

        store.confirmSent(recording.id, segment: segment.id, transcript: "we agreed to ship on Friday")

        XCTAssertFalse(exists(audio), "confirmed receipt is the one thing that releases the audio")
        let after = try XCTUnwrap(store.recording(recording.id))
        XCTAssertEqual(after.segments.first?.state, .sent)
        XCTAssertEqual(
            after.segments.first?.transcript, "we agreed to ship on Friday",
            "the words are persisted before the audio is released, never after"
        )
    }

    /// A quick note whose transcription failed. Today's Notes recorder threw
    /// the clip away here; it must now land in the queue instead.
    func testAFailedQuickNoteKeepsItsClipInTheQueue() throws {
        let store = makeStore()
        let clip = stagedClip(store)

        store.retire(clipAt: clip, hubConfirmedReceipt: false)

        XCTAssertFalse(exists(clip), "the clip moved out of staging…")
        let kept = try XCTUnwrap(store.recordings.first)
        XCTAssertEqual(kept.kind, .note)
        XCTAssertTrue(kept.isClosed)
        XCTAssertEqual(kept.waitingCount, 1)
        let segment = try XCTUnwrap(kept.segments.first)
        XCTAssertTrue(
            exists(store.audioURL(recordingId: kept.id, segment: segment)),
            "…and into the queue, still on disk"
        )
    }

    func testAConfirmedQuickNoteReleasesItsClip() {
        let store = makeStore()
        let clip = stagedClip(store)

        store.retire(clipAt: clip, hubConfirmedReceipt: true)

        XCTAssertFalse(exists(clip))
        XCTAssertTrue(store.recordings.isEmpty, "nothing to queue — the hub already has the words")
    }

    // ── Sequencing and reassembly ────────────────────────────────────────────

    /// Upload order is index order. A segment is never overtaken by the one
    /// behind it, whatever order the network completes in.
    func testSegmentsAreSentInIndexOrder() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date())
        recording.segments = [
            CapturedSegment(index: 2, filename: "c", duration: 120),
            CapturedSegment(index: 0, filename: "a", duration: 120),
            CapturedSegment(index: 1, filename: "b", duration: 120),
        ]

        XCTAssertEqual(recording.nextToSend?.filename, "a")
        recording.markSent(recording.nextToSend!.id, transcript: "first")
        XCTAssertEqual(recording.nextToSend?.filename, "b")
        recording.markSent(recording.nextToSend!.id, transcript: "second")
        XCTAssertEqual(recording.nextToSend?.filename, "c")
        recording.markSent(recording.nextToSend!.id, transcript: "third")
        XCTAssertNil(recording.nextToSend)
    }

    /// Reassembly is by index, not by the order transcripts came back. This is
    /// what makes a meeting note read in speech order.
    func testTranscriptIsAssembledInIndexOrderNotArrivalOrder() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date())
        let third = CapturedSegment(index: 2, filename: "c", duration: 120)
        let first = CapturedSegment(index: 0, filename: "a", duration: 120)
        let second = CapturedSegment(index: 1, filename: "b", duration: 120)
        recording.segments = [third, first, second]

        // Deliberately out of order — the middle one lands last.
        recording.markSent(third.id, transcript: "and ship on Friday")
        recording.markSent(first.id, transcript: "we will cut the scope")
        recording.markSent(second.id, transcript: "review it on Thursday")

        XCTAssertEqual(recording.transcript, "we will cut the scope review it on Thursday and ship on Friday")
    }

    /// A segment still in flight contributes nothing yet — a partial
    /// transcript must not silently splice around a hole.
    func testUnsentSegmentsContributeNothingToTheTranscript() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date())
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        let b = CapturedSegment(index: 1, filename: "b", duration: 120)
        recording.segments = [a, b]
        recording.markSent(a.id, transcript: "opening remarks")

        XCTAssertEqual(recording.transcript, "opening remarks")
        XCTAssertFalse(recording.isReadyToFile, "a recording with words still in flight is not finished")
    }

    /// Giving up on a segment marks the hole honestly and KEEPS the audio.
    func testSkippingLeavesAGapMarkerAndKeepsTheAudio() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date())
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        let b = CapturedSegment(index: 1, filename: "b", duration: 120)
        let c = CapturedSegment(index: 2, filename: "c", duration: 120)
        recording.segments = [a, b, c]
        recording.markSent(a.id, transcript: "opening remarks")
        recording.markSent(c.id, transcript: "closing actions")

        recording.skipRemaining()

        XCTAssertEqual(
            recording.transcript,
            "opening remarks \(CaptureText.gapMarker) closing actions"
        )
        XCTAssertEqual(recording.deletableFilenames, ["a", "c"], "the skipped segment's audio is kept")
        XCTAssertTrue(recording.segments.first { $0.filename == "b" }!.holdsAudio)
    }

    // ── Queue state machine ──────────────────────────────────────────────────

    func testAnInFlightUploadIsRequeuedAfterACrash() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date())
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        recording.segments = [a]
        recording.markUploading(a.id)
        XCTAssertNil(recording.nextToSend, "while genuinely in flight it is not re-picked")

        recording.recoverInterruptedUploads()

        XCTAssertEqual(recording.segments.first?.state, .pending)
        XCTAssertEqual(recording.nextToSend?.id, a.id, "a process that died mid-upload leaves work to retry")
    }

    func testRetryingCountsAttemptsWithoutLosingTheSegment() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date())
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        recording.segments = [a]

        for attempt in 1...3 {
            recording.markUploading(a.id)
            recording.markFailed(a.id, error: "hub asleep")
            XCTAssertEqual(recording.segments.first?.attempts, attempt)
            XCTAssertEqual(recording.segments.first?.state, .pending, "a retry never becomes a dead end")
        }
        XCTAssertTrue(recording.segments.first!.holdsAudio, "and never releases the audio")
    }

    func testReadyToFileNeedsEverySegmentInWordsAndAHome() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date())
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        recording.segments = [a]

        XCTAssertFalse(recording.isReadyToFile, "still recording")
        recording.isClosed = true
        XCTAssertFalse(recording.isReadyToFile, "segment not sent")
        recording.markSent(a.id, transcript: "the whole meeting")
        XCTAssertFalse(recording.isReadyToFile, "no project chosen")
        recording.projectId = "p1"
        XCTAssertTrue(recording.isReadyToFile)
    }

    func testARecordingOfSilenceIsNotFiledAsANote() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date(), projectId: "p1", isClosed: true)
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        recording.segments = [a]
        recording.markSent(a.id, transcript: "   ")

        XCTAssertFalse(recording.hasWords)
        XCTAssertFalse(recording.isReadyToFile, "an empty transcript is not a note")
    }

    func testAFiledRecordingHoldingNoAudioIsFinished() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date(), projectId: "p1", isClosed: true)
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        recording.segments = [a]
        recording.markSent(a.id, transcript: "words")
        recording.savedNoteId = "note-1"

        XCTAssertTrue(recording.isFinished)
    }

    func testAFiledRecordingStillHoldingAudioIsNotFinished() {
        var recording = CapturedRecording(kind: .meeting, startedAt: Date(), projectId: "p1", isClosed: true)
        let a = CapturedSegment(index: 0, filename: "a", duration: 120)
        let b = CapturedSegment(index: 1, filename: "b", duration: 120)
        recording.segments = [a, b]
        recording.markSent(a.id, transcript: "words")
        recording.skipRemaining()
        recording.savedNoteId = "note-1"

        XCTAssertFalse(
            recording.isFinished,
            "filing the part that transcribed must not quietly discard the part that did not"
        )
    }

    // ── Durability across relaunch ───────────────────────────────────────────

    /// Kill the app; come back. The recording, its audio, and its transcripts
    /// are all still there, and anything that was mid-flight is queued again.
    func testTheQueueAndItsAudioSurviveRelaunch() throws {
        let first = makeStore()
        let recording = first.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        let a = try first.adopt(fileAt: stagedClip(first), into: recording.id, index: 0, duration: 120)
        let b = try first.adopt(fileAt: stagedClip(first), into: recording.id, index: 1, duration: 120)
        first.confirmSent(recording.id, segment: a.id, transcript: "the first two minutes")
        first.markUploading(recording.id, segment: b.id)   // …and the process dies here

        let relaunched = RecordingStore(root: root)
        relaunched.recoverAfterLaunch()

        let after = try XCTUnwrap(relaunched.recording(recording.id))
        XCTAssertEqual(after.projectName, "Roadmap", "it still knows where it was going")
        XCTAssertEqual(after.transcript, "the first two minutes", "words already won are not re-won")
        XCTAssertTrue(after.isClosed, "a recording whose process is gone is closed, not left open forever")
        XCTAssertEqual(after.nextToSend?.id, b.id, "the interrupted upload is queued again")
        XCTAssertTrue(
            exists(relaunched.audioURL(recordingId: recording.id, segment: b)),
            "and its audio is exactly where it was left"
        )
    }

    /// A clip recorded but never resolved — the app was killed between
    /// recording and transcribing. It is adopted, not swept.
    func testLaunchRecoveryAdoptsAStagedClip() throws {
        let first = makeStore()
        let clip = stagedClip(first)
        XCTAssertTrue(first.recordings.isEmpty)

        let relaunched = RecordingStore(root: root)
        relaunched.recoverAfterLaunch()

        XCTAssertFalse(exists(clip), "it moved out of staging")
        let kept = try XCTUnwrap(relaunched.recordings.first)
        XCTAssertEqual(kept.waitingCount, 1)
        XCTAssertTrue(exists(relaunched.audioURL(recordingId: kept.id, segment: kept.segments[0])))
    }

    /// The orphan sweep exists to clear audio nothing references. It must
    /// never run against a manifest it could not read: an unreadable manifest
    /// and an empty queue look identical, and confusing them would delete
    /// every recording on the phone.
    func testAnUnreadableManifestNeverSweepsAudio() throws {
        let first = makeStore()
        let recording = first.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        let a = try first.adopt(fileAt: stagedClip(first), into: recording.id, index: 0, duration: 120)
        let audio = first.audioURL(recordingId: recording.id, segment: a)

        try Data("{ not json".utf8).write(to: root.appendingPathComponent("manifest.json"))

        let relaunched = RecordingStore(root: root)
        relaunched.recoverAfterLaunch()

        XCTAssertTrue(exists(audio), "a manifest we cannot read is not permission to delete audio")
    }

    /// Deleting is the user's decision and nobody else's — but when they make
    /// it, it is real.
    func testUserDeletionRemovesTheRecordingAndItsAudio() throws {
        let store = makeStore()
        let recording = store.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        let a = try store.adopt(fileAt: stagedClip(store), into: recording.id, index: 0, duration: 120)
        let audio = store.audioURL(recordingId: recording.id, segment: a)

        store.remove(recording.id)

        XCTAssertFalse(exists(audio))
        XCTAssertTrue(store.recordings.isEmpty)
    }

    /// The number the meeting screen shows. It counts audio that exists only
    /// on this phone, which is the only number that matters after a failure.
    func testWaitingCountsAreAboutAudioThatIsStillOnlyHere() throws {
        let store = makeStore()
        let one = store.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        let a = try store.adopt(fileAt: stagedClip(store), into: one.id, index: 0, duration: 120)
        let b = try store.adopt(fileAt: stagedClip(store), into: one.id, index: 1, duration: 120)
        let two = store.begin(kind: .note)
        _ = try store.adopt(fileAt: stagedClip(store), into: two.id, index: 0, duration: 8)

        XCTAssertEqual(store.segmentsWaiting, 3)
        XCTAssertEqual(store.recordingsWaiting, 2)

        store.confirmSent(one.id, segment: a.id, transcript: "one")
        store.confirmSent(one.id, segment: b.id, transcript: "two")

        XCTAssertEqual(store.segmentsWaiting, 1)
        XCTAssertEqual(store.recordingsWaiting, 1)
    }

    // ── A 90-minute meeting with the hub unreachable throughout ──────────────

    /// The scenario the feature is judged on: 45 two-minute segments recorded
    /// while the Mac is asleep, every upload failing, then the hub comes back.
    func testNinetyMinutesWithNoHubLosesNothingAndDrainsInOrder() throws {
        let store = makeStore()
        let recording = store.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        var segments: [CapturedSegment] = []
        for index in 0..<45 {
            segments.append(try store.adopt(
                fileAt: stagedClip(store), into: recording.id, index: index, duration: 120
            ))
        }
        store.close(recording.id)

        // Every attempt fails, repeatedly, for the whole meeting.
        for _ in 0..<3 {
            for segment in segments {
                store.markUploading(recording.id, segment: segment.id)
                store.markFailed(recording.id, segment: segment.id, error: "Your hub isn't answering")
            }
        }

        let stranded = try XCTUnwrap(store.recording(recording.id))
        XCTAssertEqual(stranded.waitingCount, 45, "nothing was dropped")
        XCTAssertEqual(stranded.duration, 5_400, "ninety minutes of it")
        for segment in segments {
            XCTAssertTrue(exists(store.audioURL(recordingId: recording.id, segment: segment)))
        }

        // The Mac wakes up. Everything drains, in order.
        var order: [Int] = []
        while let next = store.recording(recording.id)?.nextToSend {
            order.append(next.index)
            store.confirmSent(recording.id, segment: next.id, transcript: "part \(next.index)")
        }
        XCTAssertEqual(order, Array(0..<45), "drained in speech order, not arrival order")

        let drained = try XCTUnwrap(store.recording(recording.id))
        XCTAssertTrue(drained.isReadyToFile)
        XCTAssertTrue(drained.transcript.hasPrefix("part 0 part 1 "))
        for segment in segments {
            XCTAssertFalse(
                exists(store.audioURL(recordingId: recording.id, segment: segment)),
                "and only now is the audio released"
            )
        }
    }


    // ── WAV fixtures ─────────────────────────────────────────────────────────

    /// A real 16 kHz mono 16-bit RIFF/WAVE file — the exact shape the meeting
    /// recorder writes. `finalized: false` reproduces what a recorder KILLED
    /// mid-segment leaves behind: every sample on disk, and the two length
    /// fields still saying nothing was written.
    private func wavBytes(seconds: Double, finalized: Bool) -> Data {
        let rate = 16_000, bytesPerFrame = 2
        let payload = Int(Double(rate * bytesPerFrame) * seconds)
        func le32(_ v: Int) -> Data {
            let u = UInt32(v)
            return Data([UInt8(u & 0xFF), UInt8((u >> 8) & 0xFF), UInt8((u >> 16) & 0xFF), UInt8((u >> 24) & 0xFF)])
        }
        func le16(_ v: Int) -> Data {
            let u = UInt16(v)
            return Data([UInt8(u & 0xFF), UInt8((u >> 8) & 0xFF)])
        }
        var d = Data("RIFF".utf8)
        d.append(le32(finalized ? 36 + payload : 36))
        d.append(Data("WAVE".utf8))
        d.append(Data("fmt ".utf8))
        d.append(le32(16))
        d.append(le16(1))                        // PCM
        d.append(le16(1))                        // mono
        d.append(le32(rate))
        d.append(le32(rate * bytesPerFrame))     // byte rate
        d.append(le16(bytesPerFrame))            // block align
        d.append(le16(16))                       // bits per sample
        d.append(Data("data".utf8))
        d.append(le32(finalized ? payload : 0))
        d.append(Data(repeating: 0x01, count: payload))
        return d
    }

    // ── The unfinished segment a kill leaves behind ──────────────────────────

    /// Measured against the hub's own decoder on 2026-08-19: a WAV whose
    /// lengths were never finalised is rejected outright — "wav: missing data
    /// chunk" — and the whole two minutes is lost even though every sample is
    /// on disk. Rewriting the two fields from the real file length is the
    /// difference between losing that piece and keeping it.
    func testAnUnfinalisedSegmentIsRepairedRatherThanLost() throws {
        let url = root.appendingPathComponent("killed.wav")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try wavBytes(seconds: 30, finalized: false).write(to: url)

        XCTAssertNil(WavInspector.duration(at: url), "as written, the header claims no audio at all")

        XCTAssertTrue(WavInspector.repairHeaderIfNeeded(at: url))

        XCTAssertEqual(try XCTUnwrap(WavInspector.duration(at: url)), 30, accuracy: 0.01)
        let repaired = try Data(contentsOf: url)
        let layout = try XCTUnwrap(WavInspector.layout(of: repaired))
        XCTAssertEqual(layout.declaredDataSize, repaired.count - layout.payloadOffset)
    }

    func testAFinalisedSegmentIsLeftAlone() throws {
        let url = root.appendingPathComponent("clean.wav")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try wavBytes(seconds: 12, finalized: true).write(to: url)

        XCTAssertFalse(WavInspector.repairHeaderIfNeeded(at: url), "nothing to fix, nothing touched")
        XCTAssertEqual(try XCTUnwrap(WavInspector.duration(at: url)), 12, accuracy: 0.01)
    }

    /// The whole "app killed mid-meeting" story, end to end: the segments that
    /// completed are queued, and so is the one that was being written.
    func testLaunchRecoveryRescuesTheSegmentBeingWrittenWhenTheAppDied() throws {
        let first = makeStore()
        let recording = first.begin(kind: .meeting, projectId: "p1", projectName: "Roadmap")
        _ = try first.adopt(fileAt: stagedClip(first), into: recording.id, index: 0, duration: 120)
        // …and the recorder was part-way through the next piece when the phone
        // died, so it never reached the manifest.
        let inProgress = first.directory(for: recording.id)
            .appendingPathComponent("\(RecordingStore.inProgressPrefix)0001.wav")
        try wavBytes(seconds: 47, finalized: false).write(to: inProgress)

        let relaunched = RecordingStore(root: root)
        relaunched.recoverAfterLaunch()

        let after = try XCTUnwrap(relaunched.recording(recording.id))
        XCTAssertEqual(after.segments.count, 2, "the unfinished piece was rescued, not swept")
        let rescued = try XCTUnwrap(after.ordered.last)
        XCTAssertEqual(rescued.index, 1, "and it sits after the piece before it")
        XCTAssertEqual(rescued.duration, 47, accuracy: 0.1, "its real length, read back from the repaired header")
        XCTAssertTrue(exists(relaunched.audioURL(recordingId: recording.id, segment: rescued)))
        XCTAssertFalse(exists(inProgress))
    }

    // ── Small honest details ─────────────────────────────────────────────────

    func testElapsedReadsAsAMeetingLength() {
        XCTAssertEqual(CaptureText.elapsed(0), "0:00")
        XCTAssertEqual(CaptureText.elapsed(65), "1:05")
        XCTAssertEqual(CaptureText.elapsed(600), "10:00")
        XCTAssertEqual(CaptureText.elapsed(5_400), "1:30:00", "the length that used to be impossible")
        XCTAssertEqual(CaptureText.elapsed(-5), "0:00")
    }
}
