// The drain — moving recorded audio off the phone, in order, without ever
// deleting anything the hub has not taken.
//
// Recording and uploading are deliberately separate concerns. The recorder's
// job ends when a finished file is in the store; this file's job is to keep
// trying until the hub has the words, however many hours or reconnections
// that takes. Nothing here can lose audio: every failure path leaves the
// segment `pending` with its file untouched, and the ONLY call that removes a
// file is `RecordingStore.confirmSent`, reached only when the hub answered
// 2xx with a transcript for that exact segment.
//
// Ordering is by segment index, always. Upload order is transcript order, so
// a slow segment is never overtaken by the one behind it and the note reads in
// speech order rather than arrival order.

import Foundation

@MainActor
final class MeetingUploader: ObservableObject {
    static let shared = MeetingUploader()

    /// A pass is running. Not "something is wrong" — just "hands off".
    @Published private(set) var isDraining = false
    /// The last thing that went wrong, in the user's words. Cleared by a
    /// successful segment.
    @Published private(set) var lastError: String?
    /// Segments uploaded in the current app run — proof of life for the UI.
    @Published private(set) var uploadedThisRun = 0

    private let store: RecordingStore
    private var pass: Task<Void, Never>?

    /// `nil` rather than `.shared` as the default: a default-argument
    /// expression is written outside the initialiser it belongs to, so whether
    /// it may read main-actor state is a question the two compilers this ships
    /// under answer differently. Resolving it in the body — which is
    /// unambiguously main-actor isolated — asks no such question, and the
    /// injection point tests need is unchanged.
    init(store: RecordingStore? = nil) {
        self.store = store ?? RecordingStore.shared
    }

    /// Ask for a drain. Cheap and idempotent — safe to call after every
    /// segment, on foreground, on a retry timer, and at launch.
    func requestDrain() {
        guard pass == nil else { return }
        pass = Task { [weak self] in
            await self?.drain()
            self?.pass = nil
        }
    }

    /// One pass over everything unfinished, oldest recording first.
    func drain() async {
        guard !isDraining else { return }
        isDraining = true
        defer { isDraining = false }

        let queue = store.recordings
            .filter { !$0.isFinished }
            .sorted { $0.startedAt < $1.startedAt }

        for recording in queue {
            if Task.isCancelled { return }
            switch await send(recordingId: recording.id) {
            case .hubUnreachable:
                // Nothing else will get through either. Stop the pass and
                // leave every byte where it is.
                return
            case .rejected, .done:
                break
            }
            await fileIfReady(recordingId: recording.id)
        }
    }

    private enum PassOutcome { case done, rejected, hubUnreachable }

    /// Upload this recording's outstanding segments, lowest index first.
    private func send(recordingId: UUID) async -> PassOutcome {
        while let current = store.recording(recordingId), let segment = current.nextToSend {
            if Task.isCancelled { return .done }
            let url = store.audioURL(recordingId: recordingId, segment: segment)

            guard let audio = try? Data(contentsOf: url, options: [.mappedIfSafe]) else {
                // The manifest says there is audio and there is not. Say so
                // rather than retrying forever against a file that is gone.
                store.markFailed(
                    recordingId, segment: segment.id,
                    error: "This segment's audio file is missing from this phone."
                )
                return .rejected
            }

            store.markUploading(recordingId, segment: segment.id)
            do {
                let text = try await APIClient.shared.transcribeSegment(
                    audio, filename: segment.filename, mimeType: "audio/wav"
                )
                // Persisted here — and only here is the audio deleted.
                store.confirmSent(recordingId, segment: segment.id, transcript: text)
                uploadedThisRun += 1
                lastError = nil
            } catch {
                let message = Self.describe(error)
                store.markFailed(recordingId, segment: segment.id, error: message)
                lastError = message
                return Self.isTransportFailure(error) ? .hubUnreachable : .rejected
            }
        }
        return .done
    }

    /// Every segment is in; write the note.
    private func fileIfReady(recordingId: UUID) async {
        guard let recording = store.recording(recordingId), recording.isReadyToFile,
              let projectId = recording.projectId
        else { return }

        let title = recording.kind == .meeting
            ? CaptureText.meetingTitle(startedAt: recording.startedAt)
            : CaptureText.noteTitle(startedAt: recording.startedAt)
        // `kind: "meeting"` is what triggers the hub's write-up pass, which is
        // the whole payoff: organised notes plus follow-ups on the board.
        let kind: String? = recording.kind == .meeting ? "meeting" : nil

        do {
            let note = try await APIClient.shared.createNote(
                projectId: projectId, title: title, body: recording.transcript, kind: kind
            )
            store.markNoteSaved(recordingId, noteId: note.id)
        } catch {
            // The transcript is already durable in the manifest; a failed note
            // save costs a retry, never words.
            store.noteError(recordingId, "Couldn't file the note yet — \(Self.describe(error)) The transcript is safe on this phone.")
        }
    }

    /// File the part that transcribed and stop retrying the rest.
    ///
    /// The desktop recorder's honesty on a partial meeting ("the transcribed
    /// part survived — save it as the meeting note"), made an action. The
    /// skipped segments' AUDIO is kept: giving up on a transcript is not
    /// giving up on the recording, and the user can retry or delete it later.
    func fileWhatTranscribed(recordingId: UUID) async {
        store.skipRemaining(recordingId)
        await fileIfReady(recordingId: recordingId)
    }

    /// Assign a project to a recording that has none — the path a quick note
    /// stranded by a failed upload takes to become a note.
    func setTargetAndDrain(recordingId: UUID, projectId: String, projectName: String) {
        store.setTarget(recordingId, projectId: projectId, projectName: projectName)
        requestDrain()
    }

    // ── Error vocabulary ─────────────────────────────────────────────────────

    /// A failure to REACH the hub, as opposed to the hub answering with a
    /// complaint. The difference decides whether the rest of the queue is
    /// worth attempting in this pass.
    static func isTransportFailure(_ error: Error) -> Bool {
        if error is URLError { return true }
        if case APIError.badStatus(0) = error { return true }
        if case APIError.notPaired = error { return true }
        return false
    }

    static func describe(_ error: Error) -> String {
        switch error {
        case APIError.notPaired:
            return "This phone isn't paired with a hub yet."
        case APIError.unauthorized:
            return "The hub rejected this phone's pairing — pair again."
        case APIError.dictationUnavailable:
            return "Your hub has no local transcription model set up yet — open the desktop app once."
        case APIError.daemon(let detail):
            return "Your hub couldn't transcribe this segment: \(detail)"
        case APIError.badStatus(let code):
            return "The hub answered \(code)."
        case let urlError as URLError:
            switch urlError.code {
            case .notConnectedToInternet, .networkConnectionLost:
                return "No connection to the hub right now."
            case .cannotFindHost, .dnsLookupFailed:
                return "Couldn't find your hub on the tailnet."
            case .cannotConnectToHost:
                return "Your hub isn't answering — is your Mac awake?"
            case .timedOut:
                return "Your hub took too long to answer."
            default:
                return urlError.localizedDescription
            }
        default:
            return error.localizedDescription
        }
    }
}
