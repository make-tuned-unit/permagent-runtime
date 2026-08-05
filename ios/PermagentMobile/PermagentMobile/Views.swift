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
            Text("PERMAGENT")
                .font(.system(.title2, design: .monospaced).weight(.bold))
                .foregroundStyle(Brand.ribbon)
            Text("Pair with your hub")
                .font(.title3.weight(.semibold))
                .foregroundStyle(Brand.text)
            Text("On your Mac, open Settings → Devices and create a pairing link. Scan its QR code here. Both devices must be on your tailnet.")
                .font(.footnote)
                .foregroundStyle(Brand.textMuted)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)

            cameraContent

            HStack(spacing: 12) {
                Rectangle().fill(Brand.borderHi).frame(height: 1)
                Text("OR PASTE THE LINK")
                    .font(.system(.caption2, design: .monospaced).weight(.semibold))
                    .foregroundStyle(Brand.textDim)
                    .fixedSize()
                Rectangle().fill(Brand.borderHi).frame(height: 1)
            }
            .padding(.horizontal, 32)

            GlassCard {
                TextField("http://your-mac.tailnet.ts.net/ui/#claim=…", text: $url)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(Brand.text)
            }
            .padding(.horizontal, 24)
            if let errorMessage {
                Text(errorMessage)
                    .font(.caption)
                    .foregroundStyle(Brand.danger)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 32)
            }
            Button {
                Task { await pair(using: url) }
            } label: {
                Text(isPairing ? "Connecting…" : "Connect with pasted link")
                    .font(.body.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 14)
                    .background(Brand.ribbon)
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .foregroundStyle(Brand.onAccent)
            }
            .padding(.horizontal, 24)
            .disabled(isPairing || url.trimmingCharacters(in: .whitespaces).isEmpty)
            Spacer(minLength: 28)
            }
        }
        .task { await prepareCamera() }
    }

    @ViewBuilder
    private var cameraContent: some View {
        switch cameraState {
        case .checking:
            ProgressView("Checking camera access…")
                .foregroundStyle(Brand.textMuted)
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
                    .stroke(Brand.cyan, lineWidth: 2)
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
        GlassCard {
            VStack(spacing: 10) {
                Image(systemName: icon).font(.title2).foregroundStyle(Brand.textMuted)
                Text(text)
                    .font(.caption)
                    .foregroundStyle(Brand.textMuted)
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
            List {
                if let errorText {
                    Text(errorText).font(.brandCaption).foregroundStyle(Brand.danger)
                        .listRowBackground(Color.clear).listRowSeparator(.hidden)
                }
                if items.isEmpty {
                    Text("Nothing needs you right now. \(identity.nameCapitalized) surfaces risk gates, reviews, and unblock requests here — approve or send back with a tap.")
                        .font(.brandCaption).foregroundStyle(Brand.textMuted)
                        .listRowBackground(Color.clear).listRowSeparator(.hidden)
                }
                ForEach(items) { d in
                    GlassCard {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(d.kind.replacingOccurrences(of: "_", with: " ").uppercased())
                                .font(.brandLabel)
                                .foregroundStyle(Brand.cyanInk)
                            Text(d.headline ?? "Decision")
                                .font(.brandHeadline)
                                .foregroundStyle(Brand.text)
                            if let detail = d.detail, !detail.isEmpty {
                                Text(detail).font(.brandCaption).foregroundStyle(Brand.textMuted).lineLimit(4)
                            }
                            if let goal = d.goal_title, !goal.isEmpty {
                                Text("Goal: \(goal)")
                                    .font(.brandLabel)
                                    .foregroundStyle(Brand.textDim)
                            }
                            actions(for: d)
                        }
                    }
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Brand.shell)
            .navigationTitle("Decisions")
            .refreshable { await load() }
            .task { await load() }
            // Tactile: a success tap when you clear a decision.
            .sensoryFeedback(.success, trigger: resolvedCount)
        }
    }

    @ViewBuilder
    private func actions(for d: OpenDecision) -> some View {
        if d.isBinary {
            HStack(spacing: 10) {
                answerButton(d, verb: "reject", label: "Send back", tint: Brand.textMuted, fill: Brand.surface)
                answerButton(d, verb: "approve", label: "Approve", tint: Brand.onAccent, fill: Brand.cyan)
            }
            .padding(.top, 2)
        } else {
            Text("Open on your desktop to answer this one.")
                .font(.caption2).foregroundStyle(Brand.textDim).padding(.top, 2)
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
            List(goals) { g in
                HStack(spacing: 10) {
                    Circle()
                        .fill(g.state == "in_progress" ? Brand.cyan : Brand.textDim)
                        .frame(width: 7, height: 7)
                    Text(g.title).font(.subheadline).foregroundStyle(Brand.text)
                    Spacer()
                    Text(g.state).font(.system(.caption2, design: .monospaced)).foregroundStyle(Brand.textMuted)
                }
                .listRowBackground(Color.clear)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Brand.shell)
            .navigationTitle("In Flight")
            .refreshable { await load() }
            .task { await load() }
        }
    }
    func load() async {
        struct Resp: Decodable { let goals: [ActiveGoal] }
        if let resp = try? await APIClient.shared.get("/api/goals/active", as: Resp.self) {
            goals = resp.goals
        }
    }
}

// ── Chat (real: send to the hub's /reply, stream Henry's answer) ─────────────

struct ChatBubble: Identifiable {
    let id = UUID()
    let role: String   // "user" | "assistant"
    var text: String
    var thinking: String = ""
}

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
                        .font(.system(.caption2, design: .monospaced))
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
    @State private var draft = ""
    @State private var messages: [ChatBubble] = []
    @State private var sending = false
    @State private var sentCount = 0
    // Resolved from the hub on appear — NOT minted locally. See MobileSession.
    @State private var sessionId: String?
    @State private var sessionError: String?
    @State private var showVoice = false
    @State private var confirmEnd = false

    /// Drop the thread and the id behind it. The next send resolves a new
    /// session, so this is the only place that has to forget anything.
    private func endConversation() {
        MobileSession.endConversation()
        sessionId = nil
        sessionError = nil
        withAnimation(Motion.ease) { messages = [] }
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
            VStack(spacing: 0) {
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 10) {
                            if messages.isEmpty {
                                Text("Ask \(identity.name) to do something on your hub — open a site in the desktop browser, dispatch a goal, check the Brain. It runs on your Mac; you watch it here.")
                                    .font(.brandCaption)
                                    .foregroundStyle(Brand.textMuted)
                                    .padding(.top, 48)
                                    .padding(.horizontal, 4)
                            }
                            ForEach(messages) { bubble($0) }
                        }
                        .padding()
                    }
                    .onChange(of: messages.count) { _, _ in
                        if let last = messages.last {
                            withAnimation(Motion.spring) { proxy.scrollTo(last.id, anchor: .bottom) }
                        }
                    }
                }
                composer
            }
            .background(Brand.shell)
            .navigationTitle(identity.displayName)
            .toolbar {
                // Ending a conversation had no affordance at all: the thread
                // grew forever and the only way to start clean was to delete
                // the app (reported 2026-08-04). Destructive-styled and
                // confirmed, because it is the one action here that cannot be
                // undone from the phone.
                ToolbarItem(placement: .topBarLeading) {
                    Button(role: .destructive) { confirmEnd = true } label: {
                        Image(systemName: "arrow.counterclockwise")
                            .font(.body.weight(.semibold))
                            .foregroundStyle(Brand.textMuted)
                    }
                    .disabled(messages.isEmpty && sessionId == nil)
                    .accessibilityLabel("End this conversation")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    // Straight in. No await before presenting: VoiceView resolves
                    // the session itself (minting one if this chat has never been
                    // sent to), so voice does not require typing a message first
                    // and a resolution failure shows ON the voice screen instead
                    // of sliding up an empty cover.
                    Button { showVoice = true } label: {
                        Image(systemName: "waveform")
                            .font(.body.weight(.semibold))
                            .foregroundStyle(Brand.cyanInk)
                    }
                    .accessibilityLabel("Talk with \(identity.name)")
                }
            }
            // Voice shares the chat's hub session so spoken turns land in the
            // same conversation. `sessionId` is passed if this chat already
            // resolved one and left nil otherwise — VoiceView resolves and
            // reports its own failures. The previous `if let sessionId` guard
            // is what produced the black screen: when resolution had failed,
            // the cover presented an EMPTY body.
            .fullScreenCover(isPresented: $showVoice) {
                VoiceView(sessionId: sessionId)
            }
            .confirmationDialog(
                "End this conversation?",
                isPresented: $confirmEnd,
                titleVisibility: .visible
            ) {
                Button("End conversation", role: .destructive) { endConversation() }
                Button("Keep talking", role: .cancel) { }
            } message: {
                Text("\(identity.nameCapitalized) starts fresh with no memory of this thread. The old one stays on your hub.")
            }
            // Tactile: a light tap when you send.
            .sensoryFeedback(.impact(weight: .light), trigger: sentCount)
        }
    }

    private func bubble(_ m: ChatBubble) -> some View {
        HStack {
            if m.role == "user" { Spacer(minLength: 44) }
            VStack(alignment: .leading, spacing: 7) {
                if m.role != "user" && !m.thinking.isEmpty {
                    ReasoningDisclosure(thinking: m.thinking, hasAnswer: !m.text.isEmpty)
                }
                if m.role != "user" && m.text.isEmpty && m.thinking.isEmpty {
                    ThinkingDots()
                } else if !m.text.isEmpty {
                    Text(m.text)
                        .font(.brandBody)
                        .foregroundStyle(m.role == "user" ? Brand.onAccent : Brand.text)
                        .textSelection(.enabled)
                }
            }
            .padding(12)
            .background(m.role == "user" ? Brand.cyan : Brand.surface)
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            if m.role != "user" { Spacer(minLength: 44) }
        }
        .id(m.id)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    private var composer: some View {
        HStack(spacing: 8) {
            TextField("Ask \(identity.name)…", text: $draft, axis: .vertical)
                .lineLimit(1...4)
                .padding(12)
                .background(Brand.surface)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .foregroundStyle(Brand.text)
            Button {
                send()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.title2)
                    .foregroundStyle(canSend ? Brand.cyan : Brand.textDim)
            }
            .disabled(!canSend)
        }
        .padding()
    }

    private var canSend: Bool {
        !sending && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

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
        Task {
            do {
                let sid = try await resolveSession()
                for try await delta in APIClient.shared.replyStream(text, sessionId: sid) {
                    if idx < messages.count {
                        messages[idx].text += delta.text
                        messages[idx].thinking += delta.thinking
                    }
                }
                if idx < messages.count && messages[idx].text.isEmpty {
                    messages[idx].text = "Done — check your desktop."
                }
            } catch {
                if idx < messages.count {
                    messages[idx].text = "⚠️ " + Self.describeChatFailure(error)
                }
            }
            sending = false
        }
    }
}
