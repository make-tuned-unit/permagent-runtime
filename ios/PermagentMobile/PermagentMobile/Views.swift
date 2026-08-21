// v1 surfaces — supervision from anywhere. Each view is a thin renderer over
// the hub's existing API; no device-local state beyond the pairing token.

import SwiftUI
import AVFoundation

// ── Pairing ──────────────────────────────────────────────────────────────────

struct PairingView: View {
    private enum CameraState {
        case checking, ready, denied, unavailable
    }

    @EnvironmentObject var session: HubSession
    @State private var url = ""
    @State private var errorMessage: String?
    @State private var cameraState: CameraState = .checking
    @State private var scannerID = UUID()
    @State private var isPairing = false

    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
            Spacer(minLength: 28)
            // The wordmark: Manrope ExtraBold with wide tracking over the
            // ribbon — the marketing site's mark, not a terminal readout.
            Text("PERMAGENT")
                .font(.manrope(22, weight: .heavy))
                .tracking(3)
                .foregroundStyle(Brand.ribbon)
            Text("Pair with your hub")
                .font(.chatGreeting)
                .foregroundStyle(ChatSurface.text.opacity(0.9))
            Text("On your Mac, open Settings → Devices and create a pairing link. Scan its QR code here. Both devices must be on your tailnet.")
                .font(.brandCaption)
                .foregroundStyle(ChatSurface.muted)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)

            cameraContent

            HStack(spacing: 12) {
                Rectangle().fill(ChatSurface.border).frame(height: 1)
                Text("OR PASTE THE LINK")
                    .font(.brandLabel).tracking(0.88)
                    .foregroundStyle(ChatSurface.dim)
                    .fixedSize()
                Rectangle().fill(ChatSurface.border).frame(height: 1)
            }
            .padding(.horizontal, 32)

            RaisedCard {
                TextField("http://your-mac.tailnet.ts.net/ui/#claim=…", text: $url)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.jetbrainsMono(13))
                    .foregroundStyle(ChatSurface.text)
                    .tint(ChatSurface.spark)
            }
            .padding(.horizontal, 24)
            if let errorMessage {
                Text(errorMessage)
                    .font(.caption)
                    .foregroundStyle(Brand.danger)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 32)
            }
            SparkCTA(
                title: isPairing ? "Connecting…" : "Connect with pasted link",
                enabled: !(isPairing || url.trimmingCharacters(in: .whitespaces).isEmpty)
            ) {
                Task { await pair(using: url) }
            }
            .padding(.horizontal, 24)
            Spacer(minLength: 28)
            }
            .frame(maxWidth: .infinity)
        }
        .background(ChatSurface.bg.ignoresSafeArea())
        .task { await prepareCamera() }
    }

    @ViewBuilder
    private var cameraContent: some View {
        switch cameraState {
        case .checking:
            ProgressView("Checking camera access…")
                .foregroundStyle(ChatSurface.muted)
                .tint(ChatSurface.spark)
                .frame(height: 210)
        case .ready:
            ZStack(alignment: .bottom) {
                QRScannerView(
                    onCode: { code in
                        url = code
                        Task { await pair(using: code) }
                    },
                    onUnavailable: { cameraState = .unavailable }
                )
                .id(scannerID)
                .frame(height: 230)
                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(ChatSurface.spark, lineWidth: 2)
                    .frame(width: 170, height: 170)
                    .padding(.bottom, 30)
                Text(isPairing ? "Pairing…" : "Point the camera at the QR code")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .background(.black.opacity(0.7))
                    .clipShape(Capsule())
                    .padding(.bottom, 8)
            }
            .padding(.horizontal, 24)
            if errorMessage != nil {
                Button("Scan another code") {
                    errorMessage = nil
                    scannerID = UUID()
                }
                .font(.caption.weight(.semibold))
                .foregroundStyle(ChatSurface.spark)
            }
        case .denied:
            cameraNotice(
                icon: "camera.fill",
                text: "Camera access is off. Open iOS Settings → Permagent → Camera and enable it, or paste the pairing link below."
            )
        case .unavailable:
            cameraNotice(
                icon: "camera.slash.fill",
                text: "No camera is available on this device. Paste the pairing link below instead."
            )
        }
    }

    private func cameraNotice(icon: String, text: String) -> some View {
        RaisedCard {
            VStack(spacing: 10) {
                Image(systemName: icon).font(.title2).foregroundStyle(ChatSurface.muted)
                Text(text)
                    .font(.caption)
                    .foregroundStyle(ChatSurface.muted)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
        .padding(.horizontal, 24)
    }

    private func prepareCamera() async {
        guard AVCaptureDevice.default(for: .video) != nil else {
            cameraState = .unavailable
            return
        }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            cameraState = .ready
        case .notDetermined:
            cameraState = await AVCaptureDevice.requestAccess(for: .video) ? .ready : .denied
        case .denied, .restricted:
            cameraState = .denied
        @unknown default:
            cameraState = .unavailable
        }
    }

    private func pair(using value: String) async {
        guard !isPairing else { return }
        isPairing = true
        errorMessage = nil
        let result = await session.pair(from: value)
        isPairing = false
        guard case .failure(let error) = result else { return }
        switch error {
        case .malformedURL:
            errorMessage = "That isn’t a valid pairing URL. On your Mac, open Settings → Devices and scan or copy a fresh link containing #claim= or #token=."
        case .hubUnreachable(let detail):
            if let detail {
                errorMessage = "Couldn’t reach the hub — \(detail)"
            } else {
                errorMessage = "Couldn’t reach the hub. Check that the hub address matches the machine actually running the daemon, and that both devices are on the tailnet."
            }
        case .claimRejected(let statusCode):
            errorMessage = "The hub rejected this pairing code (HTTP \(statusCode)). It may be expired, already used, or unknown. Create a fresh pairing link in Settings → Devices and scan it."
        case .unexpectedResponse(let statusCode):
            if let statusCode {
                errorMessage = "The hub returned an unexpected or unreadable response (HTTP \(statusCode)). Check that the daemon is healthy, then create a fresh pairing link and try again."
            } else {
                errorMessage = "The hub returned an unreadable response. Check that the address points to the Permagent daemon, then try again."
            }
        }
    }
}

private final class CameraPreviewView: UIView {
    override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
    var previewLayer: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
}

private struct QRScannerView: UIViewRepresentable {
    let onCode: (String) -> Void
    let onUnavailable: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onCode: onCode)
    }

    func makeUIView(context: Context) -> CameraPreviewView {
        let view = CameraPreviewView()
        view.backgroundColor = .black
        guard let camera = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: camera),
              context.coordinator.session.canAddInput(input)
        else {
            onUnavailable()
            return view
        }

        let output = AVCaptureMetadataOutput()
        guard context.coordinator.session.canAddOutput(output) else {
            onUnavailable()
            return view
        }
        context.coordinator.session.addInput(input)
        context.coordinator.session.addOutput(output)
        output.setMetadataObjectsDelegate(context.coordinator, queue: .main)
        output.metadataObjectTypes = [.qr]
        view.previewLayer.session = context.coordinator.session
        view.previewLayer.videoGravity = .resizeAspectFill
        context.coordinator.start()
        return view
    }

    func updateUIView(_ uiView: CameraPreviewView, context: Context) {}

    static func dismantleUIView(_ uiView: CameraPreviewView, coordinator: Coordinator) {
        coordinator.stop()
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate, @unchecked Sendable {
        let session = AVCaptureSession()
        private let queue = DispatchQueue(label: "ai.permagent.qr-scanner")
        private let onCode: (String) -> Void
        private var hasScanned = false

        init(onCode: @escaping (String) -> Void) {
            self.onCode = onCode
        }

        func start() {
            queue.async { [session] in session.startRunning() }
        }

        func stop() {
            queue.async { [session] in
                if session.isRunning { session.stopRunning() }
            }
        }

        func metadataOutput(
            _ output: AVCaptureMetadataOutput,
            didOutput metadataObjects: [AVMetadataObject],
            from connection: AVCaptureConnection
        ) {
            guard !hasScanned,
                  let qr = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
                  qr.type == .qr,
                  let value = qr.stringValue
            else { return }
            hasScanned = true
            stop()
            onCode(value)
        }
    }
}

// ── Decisions (the phone's killer surface — approve/reject from anywhere) ─────

// Mirrors the daemon's OpenDecisionItem (a flattened Decision + goal_title).
// Fields are snake_case (Decision has no camelCase rename); the wrapper is
// { items, summary }. The old scaffold decoded { decisions:[{title}] }, which
// never matched — so this fixes a latent "inbox always empty" bug too.
struct OpenDecision: Decodable, Identifiable {
    let id: String
    let kind: String
    let headline: String?
    let detail: String?
    let goal_title: String?

    /// choice / input decisions need a picker or free text — not a binary
    /// approve/reject. Those are answered on the desktop for now.
    var isBinary: Bool { kind != "choice" && kind != "input" }
}

struct InboxView: View {
    @ObservedObject private var identity = AgentIdentity.shared
    @State private var items: [OpenDecision] = []
    @State private var busy: Set<String> = []
    @State private var errorText: String?
    @State private var resolvedCount = 0

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 14) {
                    if let errorText {
                        Text(errorText).font(.brandCaption).foregroundStyle(Brand.danger)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if items.isEmpty {
                        SparkEmptyState(
                            line: "Nothing needs you right now.",
                            caption: "\(identity.nameCapitalized) surfaces risk gates, reviews, and unblock requests here — approve or send back with a tap."
                        )
                        .padding(.top, 110)
                    }
                    ForEach(items) { d in
                        RaisedCard {
                            VStack(alignment: .leading, spacing: 8) {
                                Text(d.kind.replacingOccurrences(of: "_", with: " ").uppercased())
                                    .font(.brandLabel).tracking(0.88)
                                    .foregroundStyle(ChatSurface.spark)
                                Text(d.headline ?? "Decision")
                                    .font(.brandHeadline)
                                    .foregroundStyle(ChatSurface.text)
                                if let detail = d.detail, !detail.isEmpty {
                                    Text(detail).font(.brandCaption).foregroundStyle(ChatSurface.muted).lineLimit(4)
                                }
                                if let goal = d.goal_title, !goal.isEmpty {
                                    Text("Goal: \(goal)")
                                        .font(.brandLabel)
                                        .foregroundStyle(ChatSurface.dim)
                                }
                                actions(for: d)
                            }
                        }
                    }
                }
                .padding()
            }
            .background(ChatSurface.bg.ignoresSafeArea())
            .toolbar(.hidden, for: .navigationBar)
            .safeAreaInset(edge: .top, spacing: 0) { pageHeader }
            .refreshable { await load() }
            .task { await load() }
            // Tactile: a success tap when you clear a decision.
            .sensoryFeedback(.success, trigger: resolvedCount)
        }
    }

    /// In-page title — the tab hides the system bar in favor of this.
    private var pageHeader: some View {
        HStack {
            Text("Decisions")
                .font(.brandTitle)
                .foregroundStyle(ChatSurface.text)
            Spacer()
        }
        .padding(.horizontal, 18)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .background(ChatSurface.bg)
    }

    @ViewBuilder
    private func actions(for d: OpenDecision) -> some View {
        if d.isBinary {
            HStack(spacing: 10) {
                answerButton(d, verb: "reject", label: "Send back", tint: ChatSurface.text, fill: ChatSurface.control)
                answerButton(d, verb: "approve", label: "Approve", tint: ChatSurface.onSpark, fill: ChatSurface.spark)
            }
            .padding(.top, 2)
        } else {
            Text("Open on your desktop to answer this one.")
                .font(.caption2).foregroundStyle(ChatSurface.dim).padding(.top, 2)
        }
    }

    private func answerButton(_ d: OpenDecision, verb: String, label: String, tint: Color, fill: Color) -> some View {
        Button {
            answer(d.id, verb)
        } label: {
            Text(busy.contains(d.id) ? "…" : label)
                .font(.caption.weight(.semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 9)
                .background(fill)
                .foregroundStyle(tint)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(busy.contains(d.id))
    }

    private func answer(_ id: String, _ verb: String) {
        guard !busy.contains(id) else { return }
        busy.insert(id)
        errorText = nil
        Task {
            struct Body: Encodable { let answer: String }
            struct Resp: Decodable { let effect: String? }
            do {
                _ = try await APIClient.shared.post("/api/decisions/\(id)/answer", body: Body(answer: verb), as: Resp.self)
                resolvedCount += 1
                withAnimation(Motion.spring) { items.removeAll { $0.id == id } }
            } catch {
                errorText = "Couldn't submit that — check the hub, or answer on the desktop."
            }
            busy.remove(id)
        }
    }

    func load() async {
        struct Resp: Decodable { let items: [OpenDecision] }
        if let resp = try? await APIClient.shared.get("/api/decisions", as: Resp.self) {
            items = resp.items
        }
    }
}

/// Compact approve/reject while chatting or talking — same API as the Decisions tab.
struct ChatDecisionStrip: View {
    var tick: Int = 0
    @State private var items: [OpenDecision] = []
    @State private var busy: Set<String> = []

    var body: some View {
        Group {
            if !items.isEmpty {
                VStack(spacing: 8) {
                    ForEach(Array(items.prefix(3))) { d in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(d.headline ?? "Decision")
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.text)
                        if d.isBinary {
                            HStack(spacing: 8) {
                                Button {
                                    answer(d.id, "reject")
                                } label: {
                                    Text(busy.contains(d.id) ? "…" : "Reject")
                                        .font(.caption.weight(.semibold))
                                        .frame(maxWidth: .infinity)
                                        .padding(.vertical, 8)
                                        .background(ChatSurface.control)
                                        .foregroundStyle(ChatSurface.text)
                                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                                }
                                .disabled(busy.contains(d.id))
                                Button {
                                    answer(d.id, "approve")
                                } label: {
                                    Text(busy.contains(d.id) ? "…" : "Approve")
                                        .font(.caption.weight(.semibold))
                                        .frame(maxWidth: .infinity)
                                        .padding(.vertical, 8)
                                        .background(ChatSurface.spark)
                                        .foregroundStyle(ChatSurface.onSpark)
                                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                                }
                                .disabled(busy.contains(d.id))
                            }
                        }
                    }
                    .padding(12)
                    .background(ChatSurface.raised)
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .strokeBorder(ChatSurface.border, lineWidth: 1)
                    )
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 4)
            }
        }
        .task {
            await load()
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 4_000_000_000)
                await load()
            }
        }
        .onChange(of: tick) { _, _ in
            Task { await load() }
        }
    }

    private func answer(_ id: String, _ verb: String) {
        guard !busy.contains(id) else { return }
        busy.insert(id)
        Task {
            struct Body: Encodable { let answer: String }
            struct Resp: Decodable { let effect: String? }
            _ = try? await APIClient.shared.post("/api/decisions/\(id)/answer", body: Body(answer: verb), as: Resp.self)
            await load()
            busy.remove(id)
        }
    }

    private func load() async {
        struct Resp: Decodable { let items: [OpenDecision] }
        if let resp = try? await APIClient.shared.get("/api/decisions", as: Resp.self) {
            items = resp.items.filter { $0.isBinary }
        }
    }
}

// ── Goals ────────────────────────────────────────────────────────────────────

struct ActiveGoal: Decodable, Identifiable {
    let id: String
    let title: String
    let state: String
}

struct GoalsView: View {
    @State private var goals: [ActiveGoal] = []
    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 14) {
                    if goals.isEmpty {
                        SparkEmptyState(
                            line: "Nothing in flight.",
                            caption: "Dispatched goals appear here the moment they start running on the hub."
                        )
                        .padding(.top, 110)
                    }
                    ForEach(goals) { g in
                        RaisedCard {
                            HStack(spacing: 10) {
                                Circle()
                                    .fill(g.state == "in_progress" ? ChatSurface.spark : ChatSurface.dim)
                                    .frame(width: 7, height: 7)
                                Text(g.title).font(.brandHeading).foregroundStyle(ChatSurface.text)
                                Spacer()
                                Text(g.state.replacingOccurrences(of: "_", with: " ").uppercased())
                                    .font(.brandLabel).tracking(0.88)
                                    .foregroundStyle(ChatSurface.dim)
                            }
                        }
                    }
                }
                .padding()
            }
            .background(ChatSurface.bg.ignoresSafeArea())
            .toolbar(.hidden, for: .navigationBar)
            .safeAreaInset(edge: .top, spacing: 0) { pageHeader }
            .refreshable { await load() }
            .task { await load() }
        }
    }

    /// In-page title — the tab hides the system bar in favor of this.
    private var pageHeader: some View {
        HStack {
            Text("In Flight")
                .font(.brandTitle)
                .foregroundStyle(ChatSurface.text)
            Spacer()
        }
        .padding(.horizontal, 18)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .background(ChatSurface.bg)
    }

    func load() async {
        struct Resp: Decodable { let goals: [ActiveGoal] }
        if let resp = try? await APIClient.shared.get("/api/goals/active", as: Resp.self) {
            goals = resp.goals
        }
    }
}

// ── Chat (real: send to the hub's /reply, stream Henry's answer) ─────────────

// ChatBubble and the stream/transcript logic live in ChatStream.swift —
// Foundation-only, compiled directly by the regression tests.

/// Reasoning disclosure — mirrors the desktop chat: the model's thinking in a
/// collapsible block that auto-opens while it's still thinking (no answer yet)
/// and collapses to a one-line summary once the answer starts.
struct ReasoningDisclosure: View {
    let thinking: String
    let hasAnswer: Bool
    @State private var expanded: Bool
    @State private var userToggled = false

    init(thinking: String, hasAnswer: Bool) {
        self.thinking = thinking
        self.hasAnswer = hasAnswer
        _expanded = State(initialValue: !hasAnswer)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Button {
                userToggled = true
                withAnimation(Motion.ease) { expanded.toggle() }
            } label: {
                HStack(spacing: 5) {
                    Text("✦").foregroundStyle(Brand.cyanInk).opacity(hasAnswer ? 0.65 : 1)
                    Text(hasAnswer ? "Reasoning" : "Thinking…")
                        .font(.jetbrainsMono(11))
                        .foregroundStyle(Brand.textDim)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 8, weight: .semibold))
                        .foregroundStyle(Brand.textDim)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                }
            }
            .buttonStyle(.plain)
            if expanded {
                Text(thinking)
                    .font(.system(size: 12.5))
                    .foregroundStyle(Brand.textMuted)
                    .textSelection(.enabled)
                    .padding(.leading, 9)
                    .overlay(alignment: .leading) {
                        RoundedRectangle(cornerRadius: 1).fill(Brand.borderHi).frame(width: 2)
                    }
            }
        }
        .onChange(of: hasAnswer) { _, has in
            if has && !userToggled { withAnimation(Motion.ease) { expanded = false } }
        }
    }
}

struct ChatView: View {
    @ObservedObject private var identity = AgentIdentity.shared
    @Environment(\.scenePhase) private var scenePhase
    @State private var draft = ""
    @State private var messages: [ChatBubble] = []
    @State private var sending = false
    @State private var sentCount = 0
    // Resolved from the hub on appear — NOT minted locally. See MobileSession.
    @State private var sessionId: String?
    @State private var sessionError: String?
    @State private var showHistory = false
    /// True when the turn may still be running ON THE HUB while this device
    /// stopped watching (locked, backgrounded, or the stream dropped). The hub
    /// keeps working either way; this flag is what makes the phone catch up
    /// instead of showing a torn reply.
    @State private var hubTurnLive = false
    /// Grace window so a short lock doesn't kill the stream immediately.
    @State private var bgTask: UIBackgroundTaskIdentifier = .invalid
    /// Dictation into the composer — records here, transcribes on the hub.
    @StateObject private var dictation = DictationRecorder()
    @State private var transcribing = false
    /// Owns the keyboard. Without this there was no way to put it away: it
    /// covered the tab bar, so the user could neither leave chat nor reach the
    /// send button's row — reported 2026-08-05.
    @FocusState private var composerFocused: Bool
    @State private var decisionTick = 0

    /// Start fresh. The old thread is NOT deleted — it stays on the hub and
    /// is one tap away in Conversations, which is why this needs no
    /// confirmation. The next send resolves a new session.
    private func newConversation() {
        MobileSession.endConversation()
        sessionId = nil
        sessionError = nil
        composerFocused = false
        withAnimation(Motion.ease) { messages = [] }
    }

    /// Open a past conversation: adopt its id and render its history. The hub
    /// stays the source of truth — nothing is reconstructed on device.
    private func openSession(_ id: String) async {
        composerFocused = false
        showHistory = false
        do {
            let loaded = try await MobileSession.adopt(id)
            sessionId = id
            sessionError = nil
            withAnimation(Motion.spring) { messages = loaded }
        } catch {
            sessionError = Self.describeChatFailure(error)
        }
    }

    /// The hub session for this chat, resolved once and reused.
    private func resolveSession() async throws -> String {
        if let sessionId { return sessionId }
        let id = try await MobileSession.chatSessionId()
        sessionId = id
        return id
    }

    /// Name the failure. One catch-all string ("Couldn't reach …") sent Jesse
    /// hunting the tailnet while the hub was answering 200 in 16ms and dying
    /// on `Session not found` — the same lesson `PairingFailure` already
    /// taught: a message that describes the wrong layer is worse than no
    /// message, because it is confidently misleading.
    private static func describeChatFailure(_ error: Error) -> String {
        if let api = error as? APIError {
            switch api {
            case .notPaired:
                return "This device is not paired with a hub any more. Re-pair from Settings."
            case .unauthorized:
                return "The hub rejected this device's credentials (401). The pairing may have been revoked — re-pair from Settings."
            case .badStatus(let code) where code == 424:
                return "The hub has no agent configured yet (424). Set a model on the desktop, then try again."
            case .badStatus(let code) where code == 422:
                return "The hub started answering and then reported an error mid-reply. Check the desktop logs — the request reached it fine."
            case .badStatus(let code) where (500...599).contains(code):
                return "The hub errored while answering (HTTP \(code)). It received the message; the failure is on the hub."
            case .badStatus(let code):
                return "The hub answered unexpectedly (HTTP \(code))."
            case .dictationUnavailable:
                return "The hub has no local dictation model configured."
            case .daemon(let message):
                return message
            }
        }
        if let urlError = error as? URLError {
            switch urlError.code {
            case .cannotFindHost, .dnsLookupFailed:
                return "The hub's name could not be resolved (DNS) — is MagicDNS on for this device?"
            case .cannotConnectToHost:
                return "The connection was refused — the hub is reachable but nothing is answering on that port."
            case .notConnectedToInternet, .networkConnectionLost:
                return "This device lost its network connection mid-reply."
            case .timedOut:
                return "The hub did not respond in time — is your Mac awake and on the tailnet?"
            default:
                return "\(urlError.localizedDescription) [URLError \(urlError.errorCode)]"
            }
        }
        return error.localizedDescription
    }


    var body: some View {
        NavigationStack {
            ZStack {
                ChatSurface.bg.ignoresSafeArea()
                VStack(spacing: 0) {
                    header
                    ScrollViewReader { proxy in
                        ScrollView {
                            VStack(alignment: .leading, spacing: 18) {
                                if messages.isEmpty {
                                    emptyState
                                }
                                ForEach(messages) { row($0) }
                                if hubTurnLive {
                                    catchingUpRow
                                }
                            }
                            .padding(.horizontal, 18)
                            .padding(.vertical, 12)
                        }
                        // Drag the transcript down to put the keyboard away —
                        // the native idiom, and the one people try first. A
                        // plain tap on the transcript does the same (reported
                        // 2026-08-06): the keyboard covers the tab bar, so
                        // tap-out must bring navigation back. Simultaneous so
                        // taps inside message rows (links, buttons) still land.
                        .scrollDismissesKeyboard(.interactively)
                        .simultaneousGesture(TapGesture().onEnded { composerFocused = false })
                        .onChange(of: messages.count) { _, _ in
                            if let last = messages.last {
                                withAnimation(Motion.spring) { proxy.scrollTo(last.id, anchor: .bottom) }
                            }
                        }
                    }
                    ChatDecisionStrip(tick: decisionTick)
                    composer
                }
            }
            .toolbar(.hidden, for: .navigationBar)
            // (No keyboard "Done" accessory: tap-out and drag-down both
            // dismiss now, and the floating button read as clutter — removed
            // on Jesse's report 2026-08-06.)
            .sheet(isPresented: $showHistory) {
                ChatHistorySheet(currentSessionId: sessionId) { id in
                    Task { await openSession(id) }
                }
            }
            // Tactile: a light tap when you send.
            .sensoryFeedback(.impact(weight: .light), trigger: sentCount)
            // The hub finishes the turn whether or not this device is
            // watching. Coming back to the foreground, catch up from the
            // hub's stored transcript rather than trusting whatever half of
            // the stream made it here.
            .onChange(of: scenePhase) { _, phase in
                if phase == .active && hubTurnLive {
                    Task { await catchUpFromHub() }
                }
            }
            // Cold launch: put the ongoing conversation back on screen. A
            // reply that finished while the app was closed is simply THERE —
            // and one still running shows the catching-up row until it lands.
            .task { await initialRestore() }
        }
    }

    // ── Chrome ──────────────────────────────────────────────────────────────

    /// The header people already know how to use: hamburger top-left opens
    /// the conversation list; compose top-right starts a new one.
    private var header: some View {
        HStack {
            Button {
                composerFocused = false
                showHistory = true
            } label: {
                Image(systemName: "line.3.horizontal")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(ChatSurface.text)
                    .frame(width: 38, height: 38)
                    .background(ChatSurface.raised, in: Circle())
                    .overlay(Circle().strokeBorder(ChatSurface.border, lineWidth: 1))
            }
            .accessibilityLabel("Past conversations")

            Spacer()

            // No confirmation: starting a new conversation is fully
            // reversible now that past ones are one tap away, and the old
            // thread is never deleted — it stays on the hub.
            Button {
                composerFocused = false
                newConversation()
            } label: {
                Image(systemName: "square.and.pencil")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(ChatSurface.text)
                    .frame(width: 38, height: 38)
                    .background(ChatSurface.raised, in: Circle())
                    .overlay(Circle().strokeBorder(ChatSurface.border, lineWidth: 1))
            }
            .disabled(messages.isEmpty && sessionId == nil)
            .accessibilityLabel("New conversation")
        }
        .padding(.horizontal, 16)
        .padding(.top, 6)
        .padding(.bottom, 2)
    }

    /// Centered spark + a quiet serif greeting, sized to the hour.
    private var emptyState: some View {
        SparkEmptyState(line: ChatGreeting.forHour(Calendar.current.component(.hour, from: Date())))
            .padding(.top, 150)
    }

    /// Shown while a reply is finishing on the hub without a live stream here.
    private var catchingUpRow: some View {
        HStack(spacing: 10) {
            ThinkingDots()
            Text("\(identity.nameCapitalized) is still working on the hub — the reply lands here when it's done.")
                .font(.brandCaption)
                .foregroundStyle(ChatSurface.muted)
        }
        .padding(.top, 2)
    }

    // ── Transcript ──────────────────────────────────────────────────────────

    /// Claude-style rows: the user's words in a soft card on the right; the
    /// assistant's prose set directly on the page in a serif — the reading
    /// surface is the page, not a bubble.
    private func row(_ m: ChatBubble) -> some View {
        Group {
            if m.role == "user" {
                HStack {
                    Spacer(minLength: 56)
                    Text(m.text)
                        .font(.chatUser)
                        .foregroundStyle(ChatSurface.text)
                        .textSelection(.enabled)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                        .background(ChatSurface.raised)
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                }
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    if !m.thinking.isEmpty {
                        ReasoningDisclosure(thinking: m.thinking, hasAnswer: !m.text.isEmpty)
                    }
                    if m.text.isEmpty && m.thinking.isEmpty {
                        ThinkingDots()
                    } else if !m.text.isEmpty {
                        Text(m.text)
                            .font(.chatProse)
                            .lineSpacing(4)
                            .foregroundStyle(ChatSurface.text)
                            .textSelection(.enabled)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .id(m.id)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    // ── Composer ────────────────────────────────────────────────────────────

    /// One rounded card: the field on top, controls beneath. Send lives on
    /// the LEFT (beside the plus), appearing when there is something to send;
    /// dictate and voice hold the far right, always — the thumb geography the
    /// familiar chat apps trained everyone on.
    private var composer: some View {
        VStack(spacing: 10) {
            TextField("Chat with \(identity.nameCapitalized)", text: $draft, axis: .vertical)
                .lineLimit(1...5)
                .font(.system(size: 16))
                .foregroundStyle(ChatSurface.text)
                .tint(ChatSurface.spark)
                .focused($composerFocused)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 10) {
                Menu {
                    Button {
                        newConversation()
                    } label: {
                        Label("New conversation", systemImage: "square.and.pencil")
                    }
                    Button {
                        composerFocused = false
                        showHistory = true
                    } label: {
                        Label("Past conversations", systemImage: "clock.arrow.circlepath")
                    }
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 16, weight: .medium))
                        .foregroundStyle(ChatSurface.text)
                        .frame(width: 34, height: 34)
                        .background(ChatSurface.control, in: Circle())
                }
                Text(identity.nameCapitalized)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(ChatSurface.muted)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .background(ChatSurface.control, in: Capsule())
                Spacer()
                // The trailing control is one slot with two jobs: talk to
                // Henry when there is nothing to send, send when there is.
                // Typing and then reaching past a mic to a second, smaller
                // send button on the FAR side of the bar was the friction —
                // the primary action was never where the thumb already was.
                if hasDraft {
                    Button { send() } label: {
                        Image(systemName: "arrow.up")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(ChatSurface.onSpark)
                            .frame(width: 34, height: 34)
                            .background(ChatSurface.spark, in: Circle())
                    }
                    .disabled(!canSend)
                    .opacity(canSend ? 1 : 0.5)
                    .accessibilityLabel("Send")
                    .transition(.scale.combined(with: .opacity))
                } else {
                    dictateButton
                    Button { AppRoute.shared.talk() } label: {
                        Image(systemName: "waveform")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(ChatSurface.onSpark)
                            .frame(width: 34, height: 34)
                            .background(ChatSurface.spark, in: Circle())
                    }
                    .disabled(dictation.isRecording)
                    .accessibilityLabel("Talk with \(identity.name)")
                    .transition(.scale.combined(with: .opacity))
                }
            }
        }
        .animation(Motion.ease, value: hasDraft)
        .padding(14)
        .background(ChatSurface.raised)
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .strokeBorder(ChatSurface.border, lineWidth: 1)
        )
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
    }

    /// Dictation into the draft: tap to record, tap to stop; the hub's local
    /// Whisper transcribes (same path the Notes composer uses — no cloud STT).
    @ViewBuilder
    private var dictateButton: some View {
        Button { toggleDictation() } label: {
            Group {
                if transcribing {
                    ProgressView()
                        .controlSize(.small)
                        .tint(ChatSurface.muted)
                } else {
                    Image(systemName: dictation.isRecording ? "stop.fill" : "mic")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(dictation.isRecording ? Brand.onDanger : ChatSurface.text)
                }
            }
            .frame(width: 34, height: 34)
            .background(dictation.isRecording ? AnyShapeStyle(Brand.danger) : AnyShapeStyle(ChatSurface.control), in: Circle())
        }
        .disabled(transcribing)
        .accessibilityLabel(dictation.isRecording ? "Stop dictating" : "Dictate")
    }

    private func toggleDictation() {
        if dictation.isRecording {
            dictation.stop()
            return
        }
        Task {
            guard await dictation.requestPermission() == .granted else { return }
            dictation.onFinish = { url in
                guard let url, let wav = try? Data(contentsOf: url) else { return }
                Task { @MainActor in
                    transcribing = true
                    let text = try? await APIClient.shared.transcribe(wav: wav)
                    transcribing = false
                    if let text, !text.isEmpty {
                        draft = draft.isEmpty ? text : draft + " " + text
                    }
                    try? FileManager.default.removeItem(at: url)
                }
            }
            try? dictation.start()
        }
    }

    private var canSend: Bool {
        !sending && hasDraft
    }

    /// Whether there is anything to send — drives WHICH control occupies the
    /// trailing slot. Deliberately not `canSend`: that is false while a send is
    /// in flight, so keying the swap on it would flip the button back to the
    /// microphone mid-send and then back again, which reads as a glitch and
    /// puts a mic under a thumb heading for send.
    private var hasDraft: Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    // ── Sending + surviving the lock screen ─────────────────────────────────

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !sending else { return }
        draft = ""
        sending = true
        sentCount += 1
        withAnimation(Motion.spring) {
            messages.append(ChatBubble(role: "user", text: text))
            messages.append(ChatBubble(role: "assistant", text: ""))
        }
        let idx = messages.count - 1
        // Ask iOS for the background grace window, so a lock or app switch
        // mid-reply doesn't sever the stream instantly. When the window runs
        // out the stream may die — that's what catchUpFromHub() is for.
        beginBackgroundGrace()
        Task {
            var releasedForApproval = false
            var showingApprovalNotice = false
            // The paragraph-break bookkeeping (per channel — a break announced
            // by a thinking-only delta must still separate the next text) is
            // AssistantAccumulator's, regression-tested in ChatStreamTests.
            var acc = AssistantAccumulator()
            do {
                let sid = try await resolveSession()
                for try await delta in APIClient.shared.replyStream(text, sessionId: sid) {
                    if idx < messages.count {
                        if showingApprovalNotice && (!delta.text.isEmpty || !delta.thinking.isEmpty) {
                            acc.reset()
                            showingApprovalNotice = false
                        }
                        acc.apply(delta)
                        messages[idx].text = acc.text
                        messages[idx].thinking = acc.thinking
                        if let approval = delta.awaitingApproval {
                            messages[idx].text = "I asked to use \(approval.toolName) — approve below, or say yes."
                            sending = false
                            releasedForApproval = true
                            showingApprovalNotice = true
                            decisionTick += 1
                        }
                    }
                }
                if idx < messages.count && messages[idx].text.isEmpty {
                    messages[idx].text = "Done — check your desktop."
                }
            } catch {
                // A severed connection is NOT a failed reply: the hub keeps
                // running the turn (its agent has no cancellation tied to this
                // socket). Mark the turn live and catch up from the stored
                // transcript instead of painting a scary error over work that
                // is still happening.
                if ChatConnection.isLoss(error) {
                    hubTurnLive = true
                    if idx < messages.count && messages[idx].text.isEmpty && messages[idx].thinking.isEmpty {
                        withAnimation(Motion.ease) { _ = messages.popLast() }
                    }
                    if scenePhase == .active {
                        await catchUpFromHub()
                    }
                } else if idx < messages.count {
                    messages[idx].text = "⚠️ " + Self.describeChatFailure(error)
                }
            }
            if !releasedForApproval { sending = false }
            endBackgroundGrace()
        }
    }

    /// Cold-launch restore: adopt the persisted conversation (if one exists)
    /// and reconcile against anything that happened while the app was away.
    private func initialRestore() async {
        guard messages.isEmpty, sessionId == nil,
              let id = MobileSession.persistedId else { return }
        sessionId = id
        await catchUpFromHub()
    }

    /// Re-sync this screen from the hub's stored transcript, then — if a turn
    /// is still running — watch the session's event stream for the Finish and
    /// re-sync once more. Content always comes from the transcript (the
    /// hub's truth); the event stream is only the "done yet?" signal.
    private func catchUpFromHub() async {
        guard let sid = sessionId else { hubTurnLive = false; return }
        if let loaded = try? await MobileSession.adopt(sid) {
            withAnimation(Motion.ease) { messages = loaded }
        }
        do {
            for try await event in APIClient.shared.sessionEvents(sessionId: sid) {
                switch event {
                case .activeRequests(let ids):
                    if ids.isEmpty {
                        // Nothing running — the transcript we just loaded is
                        // the whole story.
                        hubTurnLive = false
                        return
                    }
                    hubTurnLive = true
                case .finished:
                    if let loaded = try? await MobileSession.adopt(sid) {
                        withAnimation(Motion.ease) { messages = loaded }
                    }
                    hubTurnLive = false
                    return
                }
            }
        } catch {
            // The watch stream itself failed (network flapped again). Leave
            // hubTurnLive set: the next foreground pass retries.
        }
        // Stream ended without a terminal frame — re-sync and settle.
        if let loaded = try? await MobileSession.adopt(sid) {
            withAnimation(Motion.ease) { messages = loaded }
        }
        hubTurnLive = false
    }

    private func beginBackgroundGrace() {
        endBackgroundGrace()
        bgTask = UIApplication.shared.beginBackgroundTask(withName: "chat-reply") {
            endBackgroundGrace()
        }
    }

    private func endBackgroundGrace() {
        if bgTask != .invalid {
            UIApplication.shared.endBackgroundTask(bgTask)
            bgTask = .invalid
        }
    }
}
