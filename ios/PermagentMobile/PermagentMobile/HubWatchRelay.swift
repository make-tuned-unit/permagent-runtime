// WatchConnectivity relay — the iPhone is the watch's hop to the hub.
//
// watchOS cannot run Tailscale, so the watch never talks to the daemon
// itself. Every chat turn and every dictated note lands here, uses the
// same pairing token and the same chat session as the phone (so the Mac,
// the phone, and the watch are one conversation), and the result is
// pushed back over WatchConnectivity.

import Foundation
import UIKit
import WatchConnectivity

@MainActor
final class HubWatchRelay: NSObject, WCSessionDelegate {
    static let shared = HubWatchRelay()
    private var processingFiles: Set<URL> = []

    func start() {
        guard WCSession.isSupported() else { return }
        let session = WCSession.default
        session.delegate = self
        session.activate()
        Task { await drainPendingFiles() }
    }

    func pushStatus() {
        Task {
            let name = AgentIdentity.shared.displayName
            let paired = await APIClient.shared.isPaired
            let payload = WatchResponse(
                id: UUID().uuidString,
                ok: true,
                op: WatchOp.ping.rawValue,
                text: nil,
                agentName: name,
                paired: paired,
                reachable: true,
                thinking: nil,
                done: true,
                projects: nil,
                error: paired ? nil : "Open Permagent on iPhone and pair with your hub."
            )
            send(payload)
        }
    }

    // MARK: WCSessionDelegate

    nonisolated func session(_ session: WCSession,
                             activationDidCompleteWith activationState: WCSessionActivationState,
                             error: Error?) {
        Task { @MainActor in pushStatus() }
    }

    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

    nonisolated func sessionDidDeactivate(_ session: WCSession) {
        session.activate()
    }

    nonisolated func sessionWatchStateDidChange(_ session: WCSession) {
        Task { @MainActor in pushStatus() }
    }

    nonisolated func session(_ session: WCSession,
                             didReceiveMessageData messageData: Data,
                             replyHandler: @escaping (Data) -> Void) {
        let reply = UncheckedReply(replyHandler)
        Task { @MainActor in
            reply(Self.encode(await handle(messageData)))
        }
    }

    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data) {
        Task { @MainActor in
            _ = await handle(messageData)
        }
    }

    nonisolated func session(_ session: WCSession, didReceive file: WCSessionFile) {
        // The incoming file is deleted when this method returns — copy it
        // first, then hop to the actor to transcribe.
        let requestId = (file.metadata?["id"] as? String) ?? UUID().uuidString
        let kind = (file.metadata?["kind"] as? String) ?? "note"
        do {
            let dest = try Self.stage(file: file.fileURL, requestId: requestId, kind: kind)
            Task { @MainActor in
                await processStagedFile(at: dest, requestId: requestId, kind: kind)
            }
        } catch {
            Task { @MainActor in
                send(WatchResponse.fail(
                    requestId,
                    op: kind == "chat" ? "chatDelta" : "transcript",
                    "The iPhone could not save the incoming recording."
                ))
            }
        }
    }

    nonisolated func session(_ session: WCSession,
                             didReceiveUserInfo userInfo: [String: Any] = [:]) {
        if let data = userInfo["request"] as? Data {
            Task { @MainActor in
                let reply = await handle(data)
                send(reply)
            }
        }
    }

    // MARK: Handle

    private func handle(_ data: Data) async -> WatchResponse {
        guard let req = try? JSONDecoder().decode(WatchRequest.self, from: data) else {
            return WatchResponse.fail("", op: "error", "Malformed watch request.")
        }
        switch req.op {
        case .ping:
            let paired = await APIClient.shared.isPaired
            return WatchResponse(
                id: req.id, ok: true, op: req.op.rawValue,
                text: nil, agentName: AgentIdentity.shared.displayName,
                paired: paired, reachable: true, thinking: nil, done: true,
                projects: nil,
                error: paired ? nil : "Open Permagent on iPhone and pair with your hub."
            )
        case .chat:
            guard let text = req.text?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !text.isEmpty else {
                return WatchResponse.fail(req.id, op: req.op.rawValue, "Nothing to send.")
            }
            Task { await streamChat(id: req.id, text: text) }
            return WatchResponse.ack(req.id, op: req.op.rawValue)
        case .listProjects:
            do {
                let projects = try await APIClient.shared.projects().map {
                    WatchProject(id: $0.id, name: $0.name, slug: $0.slug)
                }
                return WatchResponse(
                    id: req.id, ok: true, op: req.op.rawValue,
                    text: nil, agentName: nil, paired: true, reachable: true,
                    thinking: nil, done: true, projects: projects, error: nil
                )
            } catch {
                return WatchResponse.fail(req.id, op: req.op.rawValue, "Couldn't list projects.")
            }
        case .resolveProject:
            guard let spoken = req.text else {
                return WatchResponse.fail(req.id, op: req.op.rawValue, "Say a project name.")
            }
            do {
                let projects = try await APIClient.shared.projects().map {
                    WatchProject(id: $0.id, name: $0.name, slug: $0.slug)
                }
                switch ProjectMatcher.match(spoken: spoken, among: projects) {
                case .none:
                    return WatchResponse.fail(req.id, op: req.op.rawValue,
                                              "No project matched “\(spoken)”.")
                case .one(let project):
                    return WatchResponse(
                        id: req.id, ok: true, op: req.op.rawValue,
                        text: project.name, agentName: nil, paired: true, reachable: true,
                        thinking: nil, done: true, projects: [project], error: nil
                    )
                case .many(let matches):
                    return WatchResponse(
                        id: req.id, ok: false, op: req.op.rawValue,
                        text: nil, agentName: nil, paired: true, reachable: true,
                        thinking: nil, done: true, projects: matches,
                        error: "Several projects match. Say the full name."
                    )
                }
            } catch {
                return WatchResponse.fail(req.id, op: req.op.rawValue, "Couldn't reach the hub.")
            }
        case .saveNote:
            guard let projectId = req.projectId,
                  let body = req.text?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !body.isEmpty else {
                return WatchResponse.fail(req.id, op: req.op.rawValue, "Missing note or project.")
            }
            do {
                _ = try await APIClient.shared.createNote(projectId: projectId, title: nil, body: body)
                let name = (try? await APIClient.shared.projects())?
                    .first(where: { $0.id == projectId })?.name ?? "the project"
                return WatchResponse(
                    id: req.id, ok: true, op: req.op.rawValue,
                    text: "Saved to \(name).", agentName: nil, paired: true, reachable: true,
                    thinking: nil, done: true, projects: nil, error: nil
                )
            } catch {
                return WatchResponse.fail(req.id, op: req.op.rawValue, "Couldn't save the note.")
            }
        }
    }

    private func streamChat(id: String, text: String) async {
        do {
            let sid = try await MobileSession.chatSessionId()
            var acc = AssistantAccumulator()
            for try await delta in APIClient.shared.replyStream(text, sessionId: sid) {
                acc.apply(delta)
                var payload = WatchResponse.ack(id, op: "chatDelta")
                payload.text = acc.text.isEmpty ? acc.thinking : acc.text
                payload.thinking = acc.text.isEmpty && !acc.thinking.isEmpty
                send(payload)
            }
            var done = WatchResponse.ack(id, op: "chatDelta")
            done.done = true
            done.text = acc.text.isEmpty ? "Done — check your desktop." : acc.text
            send(done)
        } catch {
            send(WatchResponse.fail(id, op: "chatDelta",
                                    ChatConnection.isLoss(error)
                                    ? "Connection dropped; the hub is still working."
                                    : "Couldn't reach the hub."))
        }
    }

    private func transcribeFile(at url: URL, requestId: String, kind: String) async {
        let data = await Task.detached(priority: .userInitiated) {
            try? Data(contentsOf: url)
        }.value
        guard let data else {
            send(WatchResponse.fail(requestId, op: kind == "chat" ? "chatDelta" : "transcript",
                                    "The recording never arrived."))
            return
        }
        do {
            let text = try await APIClient.shared.transcribe(wav: data)
            if kind == "chat" {
                let clipped = text.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !clipped.isEmpty else {
                    send(WatchResponse.fail(requestId, op: "chatDelta", "I didn't catch that."))
                    return
                }
                await streamChat(id: requestId, text: clipped)
                return
            }
            var payload = WatchResponse.ack(requestId, op: "transcript")
            payload.text = text
            payload.done = true
            send(payload)
        } catch APIError.dictationUnavailable {
            send(WatchResponse.fail(requestId, op: kind == "chat" ? "chatDelta" : "transcript",
                                    "No local dictation model on the hub."))
        } catch {
            send(WatchResponse.fail(requestId, op: kind == "chat" ? "chatDelta" : "transcript",
                                    "Transcription failed."))
        }
    }

    private func send(_ payload: WatchResponse) {
        let data = Self.encode(payload)
        let session = WCSession.default
        guard session.activationState == .activated else { return }
        if session.isReachable {
            session.sendMessageData(data, replyHandler: nil) { _ in
                WCSession.default.transferUserInfo(["payload": data])
            }
        } else {
            session.transferUserInfo(["payload": data])
        }
    }

    private static func encode(_ payload: WatchResponse) -> Data {
        (try? JSONEncoder().encode(payload)) ?? Data()
    }

    private func processStagedFile(at url: URL, requestId: String, kind: String) async {
        guard processingFiles.insert(url).inserted else { return }
        let lease = PhoneBackgroundLease(name: "Permagent watch voice \(requestId)")
        defer {
            processingFiles.remove(url)
            try? FileManager.default.removeItem(at: url)
            lease.end()
        }
        await transcribeFile(at: url, requestId: requestId, kind: kind)
    }

    private func drainPendingFiles() async {
        let inbox = Self.inboxDirectory()
        guard let files = try? FileManager.default.contentsOfDirectory(
            at: inbox,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        ) else { return }
        for file in files where file.pathExtension == "wav" {
            let stem = file.deletingPathExtension().lastPathComponent
            let parts = stem.split(separator: "~", maxSplits: 1).map(String.init)
            guard parts.count == 2 else { continue }
            await processStagedFile(at: file, requestId: parts[0], kind: parts[1])
        }
    }

    nonisolated private static func stage(file source: URL,
                                          requestId: String,
                                          kind: String) throws -> URL {
        let safeId = requestId.filter { $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }
        let safeKind = kind == "chat" ? "chat" : "note"
        let inbox = inboxDirectory()
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        let dest = inbox.appendingPathComponent("\(safeId)~\(safeKind).wav")
        try? FileManager.default.removeItem(at: dest)
        try FileManager.default.copyItem(at: source, to: dest)
        return dest
    }

    nonisolated private static func inboxDirectory() -> URL {
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent("WatchRelayInbox", isDirectory: true)
    }
}

@MainActor
private final class PhoneBackgroundLease {
    private var identifier: UIBackgroundTaskIdentifier = .invalid

    init(name: String) {
        identifier = UIApplication.shared.beginBackgroundTask(withName: name) { [weak self] in
            self?.end()
        }
    }

    func end() {
        guard identifier != .invalid else { return }
        UIApplication.shared.endBackgroundTask(identifier)
        identifier = .invalid
    }

    deinit {
        if identifier != .invalid {
            UIApplication.shared.endBackgroundTask(identifier)
        }
    }
}

/// WatchConnectivity reply handlers are not Sendable. The session already
/// invokes them across queues; this only makes the hop the compiler can see.
private struct UncheckedReply: @unchecked Sendable {
    let handler: (Data) -> Void
    init(_ handler: @escaping (Data) -> Void) { self.handler = handler }
    func callAsFunction(_ data: Data) { handler(data) }
}
