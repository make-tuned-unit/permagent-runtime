import Foundation
import WatchConnectivity

@MainActor
final class WatchRelay: NSObject, ObservableObject, WCSessionDelegate {
    static let shared = WatchRelay()

    @Published var paired = false
    @Published var phoneReachable = false
    @Published var agentName = "your agent"
    @Published var notice: String?

    @Published var chatText = ""
    /// What the user said this turn (typed, or transcribed by the phone).
    @Published var chatUserText = ""
    @Published var chatThinking = false
    @Published var chatBusy = false

    @Published var noteTranscript = ""
    @Published var noteBusy = false
    @Published var noteSaved: String?
    @Published var ambiguousProjects: [WatchProject] = []
    @Published var resolvedProject: WatchProject?
    @Published var projects: [WatchProject] = []

    private var pendingNoteId: String?
    private struct QueuedRecording: Codable, Equatable {
        let path: String
        let id: String
        let kind: String
    }

    private static let pendingRecordingsKey = "watchRelay.pendingRecordings.v1"
    private var queuedRecordings: [QueuedRecording] = []
    private var activeTransferIds: Set<String> = []
    private var responseWatchdog: Task<Void, Never>?
    /// Request ids whose final chatDelta already landed. The phone sends the
    /// final reply over two routes (live message + durable userInfo) and
    /// stale progress deltas can trail in behind it; none may undo the result.
    private var finishedChatIds: Set<String> = []

    func start() {
        guard WCSession.isSupported() else {
            notice = "WatchConnectivity is unavailable."
            return
        }
        let session = WCSession.default
        session.delegate = self
        session.activate()
        loadPendingRecordings()
        flushQueue()
    }

    func ping() {
        send(WatchRequest(op: .ping, id: UUID().uuidString, text: nil, projectId: nil))
    }

    func chat(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        chatBusy = true
        chatThinking = true
        chatText = ""
        chatUserText = trimmed
        notice = nil
        armResponseWatchdog(kind: "chat")
        send(WatchRequest(op: .chat, id: UUID().uuidString, text: trimmed, projectId: nil))
    }

    func sendRecording(_ url: URL, kind: String) {
        let id = UUID().uuidString
        pendingNoteId = id
        notice = nil
        if kind == "chat" {
            chatBusy = true
            chatThinking = true
            chatText = ""
            chatUserText = ""
        } else {
            noteBusy = true
            noteTranscript = ""
            noteSaved = nil
        }
        queuedRecordings.append(QueuedRecording(path: url.path, id: id, kind: kind))
        persistPendingRecordings()
        armResponseWatchdog(kind: kind)
        flushQueue()
        if WCSession.default.activationState != .activated {
            notice = "Waking your iPhone — the recording is safely queued."
        }
    }

    func saveNote(to project: WatchProject) {
        resolvedProject = project
        saveNote()
    }

    /// After a save, keep the project and clear the clip so the next listen
    /// starts immediately. The user already chose once this visit.
    func prepareNextNote() {
        noteTranscript = ""
        noteSaved = nil
        notice = nil
    }

    func listProjects() {
        send(WatchRequest(op: .listProjects, id: UUID().uuidString, text: nil, projectId: nil))
    }

    func resolveProject(_ spoken: String) {
        send(WatchRequest(op: .resolveProject, id: UUID().uuidString, text: spoken, projectId: nil))
    }

    func saveNote() {
        guard let project = resolvedProject,
              !noteTranscript.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        noteBusy = true
        send(WatchRequest(op: .saveNote, id: UUID().uuidString,
                          text: noteTranscript, projectId: project.id))
    }

    // MARK: WCSessionDelegate

    nonisolated func session(_ session: WCSession,
                             activationDidCompleteWith activationState: WCSessionActivationState,
                             error: Error?) {
        // Copy Sendable bits here. WCSession is not Sendable; hopping the
        // object itself to MainActor is a Swift 6 data-race error.
        let reachable = session.isReachable
        let voice = WatchVoiceProvisioning.extract(session.receivedApplicationContext)
        Task { @MainActor in
            phoneReachable = reachable
            WatchVoiceProvisioning.shared.apply(credentialData: voice.data, clear: voice.clear)
            ping()
            flushQueue()
        }
    }

    /// The phone's standalone-voice credential (WatchVoiceProvisioning).
    nonisolated func session(_ session: WCSession,
                             didReceiveApplicationContext applicationContext: [String: Any]) {
        let voice = WatchVoiceProvisioning.extract(applicationContext)
        Task { @MainActor in
            WatchVoiceProvisioning.shared.apply(credentialData: voice.data, clear: voice.clear)
        }
    }

    nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        let reachable = session.isReachable
        Task { @MainActor in
            phoneReachable = reachable
            if reachable { flushQueue() }
        }
    }

    nonisolated func session(_ session: WCSession,
                             didFinish fileTransfer: WCSessionFileTransfer,
                             error: Error?) {
        let id = fileTransfer.file.metadata?["id"] as? String
        let kind = fileTransfer.file.metadata?["kind"] as? String
        let path = fileTransfer.file.fileURL.path
        let message = error?.localizedDescription
        Task { @MainActor in
            transferFinished(id: id, kind: kind, path: path, error: message)
        }
    }

    nonisolated func session(_ session: WCSession,
                             didReceiveMessageData messageData: Data) {
        Task { @MainActor in apply(messageData) }
    }

    nonisolated func session(_ session: WCSession,
                             didReceiveMessageData messageData: Data,
                             replyHandler: @escaping (Data) -> Void) {
        // Reply from this isolation: the handler is not Sendable, so it
        // cannot ride the MainActor hop. The ack payload is empty.
        replyHandler(Data())
        Task { @MainActor in apply(messageData) }
    }

    nonisolated func session(_ session: WCSession,
                             didReceiveUserInfo userInfo: [String: Any] = [:]) {
        if let data = userInfo["payload"] as? Data {
            Task { @MainActor in apply(data) }
        }
    }

    // MARK: Private

    private func send(_ request: WatchRequest) {
        guard let data = try? JSONEncoder().encode(request) else { return }
        let session = WCSession.default
        guard session.activationState == .activated else {
            notice = "Open Permagent on iPhone."
            chatBusy = false
            noteBusy = false
            return
        }
        if session.isReachable {
            Self.sendMessageData(session, data, relay: self)
        } else {
            session.transferUserInfo(["request": data])
            notice = "iPhone sleeping — queued."
        }
    }

    /// WCSession invokes these handlers on its own background queue. Closures
    /// formed inside a @MainActor method inherit main-actor isolation under
    /// Swift 6 and trip a libdispatch assertion, so build them here in a
    /// nonisolated context and hop to the main actor explicitly.
    private nonisolated static func sendMessageData(_ session: WCSession, _ data: Data,
                                                    relay: WatchRelay) {
        session.sendMessageData(data, replyHandler: { @Sendable [weak relay] reply in
            Task { @MainActor in relay?.apply(reply) }
        }, errorHandler: { @Sendable [weak relay] _ in
            Task { @MainActor in relay?.sendFailed() }
        })
    }

    private func sendFailed() {
        notice = "Hold your iPhone nearby."
        chatBusy = false
        chatThinking = false
        noteBusy = false
        responseWatchdog?.cancel()
    }

    private func apply(_ data: Data) {
        guard let reply = try? JSONDecoder().decode(WatchResponse.self, from: data) else { return }
        if let name = reply.agentName, !name.isEmpty { agentName = name }
        if let paired = reply.paired { self.paired = paired }
        if let reachable = reply.reachable { phoneReachable = reachable }

        switch reply.op {
        case WatchOp.ping.rawValue:
            notice = reply.error
        case WatchOp.chat.rawValue:
            if !reply.ok {
                notice = reply.error
                chatBusy = false
                chatThinking = false
            }
        case "chatTranscript":
            if !finishedChatIds.contains(reply.id), let text = reply.text { chatUserText = text }
        case "chatDelta":
            let final = reply.done == true || reply.ok == false
            // Duplicate or late progress for a turn that is over (or was given
            // up on) must not flip "Thinking…" back on or replace the reply.
            if finishedChatIds.contains(reply.id) { return }
            if !final && !chatBusy { return }
            if let text = reply.text { chatText = text }
            chatThinking = reply.thinking ?? false
            if !final { armResponseWatchdog(kind: "chat") }
            if final {
                finishedChatIds.insert(reply.id)
                responseWatchdog?.cancel()
                chatBusy = false
                chatThinking = false
                if let err = reply.error { notice = err }
            }
        case "transcript":
            responseWatchdog?.cancel()
            noteBusy = false
            if let err = reply.error {
                notice = err
            } else if let text = reply.text {
                noteTranscript = text
                if resolvedProject != nil {
                    saveNote()
                } else if projects.isEmpty {
                    listProjects()
                }
            }
        case WatchOp.listProjects.rawValue:
            if let list = reply.projects {
                projects = list
                notice = list.isEmpty ? "No projects on the hub." : nil
            } else if let err = reply.error {
                notice = err
            }
        case WatchOp.resolveProject.rawValue:
            if let projects = reply.projects, projects.count == 1 {
                resolvedProject = projects[0]
                ambiguousProjects = []
                notice = nil
            } else if let projects = reply.projects, projects.count > 1 {
                ambiguousProjects = projects
                resolvedProject = nil
                notice = reply.error
            } else {
                resolvedProject = nil
                notice = reply.error ?? "No match."
            }
        case WatchOp.saveNote.rawValue:
            noteBusy = false
            if reply.ok {
                noteSaved = reply.text ?? "Saved."
                notice = nil
            } else {
                notice = reply.error
            }
        default:
            if let err = reply.error { notice = err }
        }
    }

    private func flushQueue() {
        let session = WCSession.default
        guard session.activationState == .activated else { return }
        let priorCount = queuedRecordings.count
        queuedRecordings.removeAll { !FileManager.default.fileExists(atPath: $0.path) }
        if queuedRecordings.count != priorCount { persistPendingRecordings() }
        for item in queuedRecordings {
            guard !activeTransferIds.contains(item.id) else { continue }
            let url = URL(fileURLWithPath: item.path)
            activeTransferIds.insert(item.id)
            var metadata: [String: Any] = ["id": item.id, "kind": item.kind]
            #if DEBUG
            // Audio from the PERMAGENT_INJECT_WAV test hook is never passed off as a real recording.
            if url.lastPathComponent.hasPrefix("watch-injected-") {
                metadata["injected"] = true
                metadata["source"] = "injected"
                // transferFile never arrives between paired simulators, so the
                // injected clip goes as chunked sendMessageData (65 KB cap).
                if let data = try? Data(contentsOf: url) {
                    Self.sendInjected(session, data, metadata: metadata)
                    transferFinished(id: item.id, kind: item.kind, path: item.path, error: nil)
                    continue
                }
            }
            #endif
            session.transferFile(url, metadata: metadata)
        }
    }

    #if DEBUG
    /// Frame: "PGINJ1" + UInt32 BE header length + JSON header + audio chunk.
    private nonisolated static func sendInjected(_ session: WCSession, _ data: Data,
                                                 metadata: [String: Any]) {
        let chunkSize = 40_000
        let total = max(1, (data.count + chunkSize - 1) / chunkSize)
        for index in 0..<total {
            var header = metadata
            header["index"] = index
            header["total"] = total
            guard let json = try? JSONSerialization.data(withJSONObject: header) else { return }
            var frame = Data("PGINJ1".utf8)
            var len = UInt32(json.count).bigEndian
            frame.append(Data(bytes: &len, count: 4))
            frame.append(json)
            frame.append(data.subdata(in: (index * chunkSize)..<min(data.count, (index + 1) * chunkSize)))
            session.sendMessageData(frame, replyHandler: nil, errorHandler: { @Sendable error in
                NSLog("permagent-inject: chunk send failed %@", error.localizedDescription)
            })
        }
        NSLog("permagent-inject: sent INJECTED audio as %d message chunk(s), %d bytes", total, data.count)
    }
    #endif

    private func transferFinished(id: String?, kind: String?, path: String, error: String?) {
        let resolvedId = id ?? queuedRecordings.first(where: { $0.path == path })?.id
        guard let resolvedId else { return }
        activeTransferIds.remove(resolvedId)
        if let error {
            notice = "The recording could not reach iPhone: \(error)"
            if kind == "chat" {
                chatBusy = false
                chatThinking = false
            } else {
                noteBusy = false
            }
            return
        }
        if let item = queuedRecordings.first(where: { $0.id == resolvedId }) {
            try? FileManager.default.removeItem(atPath: item.path)
        }
        queuedRecordings.removeAll { $0.id == resolvedId }
        persistPendingRecordings()
    }

    private func loadPendingRecordings() {
        guard queuedRecordings.isEmpty,
              let data = UserDefaults.standard.data(forKey: Self.pendingRecordingsKey),
              let stored = try? JSONDecoder().decode([QueuedRecording].self, from: data) else {
            return
        }
        queuedRecordings = stored.filter { FileManager.default.fileExists(atPath: $0.path) }
        persistPendingRecordings()
    }

    private func persistPendingRecordings() {
        if queuedRecordings.isEmpty {
            UserDefaults.standard.removeObject(forKey: Self.pendingRecordingsKey)
        } else if let data = try? JSONEncoder().encode(queuedRecordings) {
            UserDefaults.standard.set(data, forKey: Self.pendingRecordingsKey)
        }
    }

    private func armResponseWatchdog(kind: String) {
        responseWatchdog?.cancel()
        responseWatchdog = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(kind == "chat" ? 40 : 90))
            guard !Task.isCancelled, let self else { return }
            if kind == "chat", self.chatBusy {
                self.chatBusy = false
                self.chatThinking = false
                // Shown in the reply slot so the orb never sits on "Thinking…".
                // A reply that lands later still replaces this (see apply).
                self.chatText = "Reply is on your iPhone."
            } else if kind != "chat", self.noteBusy {
                self.noteBusy = false
                self.notice = "The iPhone did not finish transcribing. Your recording is saved for retry."
            }
        }
    }
}
