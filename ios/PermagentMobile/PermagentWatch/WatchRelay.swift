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
    @Published var chatThinking = false
    @Published var chatBusy = false

    @Published var noteTranscript = ""
    @Published var noteBusy = false
    @Published var noteSaved: String?
    @Published var ambiguousProjects: [WatchProject] = []
    @Published var resolvedProject: WatchProject?
    @Published var projects: [WatchProject] = []

    private var pendingNoteId: String?
    private var queuedRecordings: [(url: URL, id: String, kind: String)] = []

    func start() {
        guard WCSession.isSupported() else {
            notice = "WatchConnectivity is unavailable."
            return
        }
        let session = WCSession.default
        session.delegate = self
        session.activate()
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
        notice = nil
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
        } else {
            noteBusy = true
            noteTranscript = ""
            noteSaved = nil
        }
        let session = WCSession.default
        if session.activationState == .activated {
            session.transferFile(url, metadata: ["id": id, "kind": kind])
        } else {
            queuedRecordings.append((url, id, kind))
            notice = "iPhone unreachable — queued until it is back."
            chatBusy = false
            chatThinking = false
            noteBusy = false
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
        Task { @MainActor in
            phoneReachable = reachable
            ping()
            flushQueue()
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
            session.sendMessageData(data, replyHandler: { [weak self] reply in
                Task { @MainActor in self?.apply(reply) }
            }, errorHandler: { [weak self] _ in
                Task { @MainActor in
                    self?.notice = "Hold your iPhone nearby."
                    self?.chatBusy = false
                    self?.noteBusy = false
                }
            })
        } else {
            session.transferUserInfo(["request": data])
            notice = "iPhone sleeping — queued."
        }
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
        case "chatDelta":
            if let text = reply.text { chatText = text }
            chatThinking = reply.thinking ?? false
            if reply.done == true || reply.ok == false {
                chatBusy = false
                chatThinking = false
                if let err = reply.error { notice = err }
            }
        case "transcript":
            noteBusy = false
            if let err = reply.error {
                notice = err
            } else if let text = reply.text {
                noteTranscript = text
                if let project = resolvedProject {
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
        for item in queuedRecordings {
            session.transferFile(item.url, metadata: ["id": item.id, "kind": item.kind])
        }
        queuedRecordings.removeAll()
    }
}
