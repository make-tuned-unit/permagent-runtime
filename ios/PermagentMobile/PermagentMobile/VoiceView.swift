// VoiceView — live voice conversation with Henry over the hub's /voice WebSocket.
//
// Wire protocol (crates/goose-server/src/routes/voice.rs):
//   connect  ws(s)://<hub>/voice?session_id=<id>&token=<bearer>   (query-param auth —
//            the WS upgrade can't carry the Bearer header; validate_stream_token)
//   server → {"type":"ready"} once STT+TTS providers are loaded
//   client → {"type":"start","sample_rate":16000}
//            binary frames: raw Float32 LE mono PCM @ 16 kHz
//            {"type":"stop"}
//   server → {"type":"transcript","text":…}
//            {"type":"reply_start"}
//            binary frames: Float32 LE mono PCM @ 24 kHz (queued, played in order)
//            {"type":"reply_text","text":…}
//            {"type":"navigate",…}       (desktop speak-then-act; ignored here)
//            {"type":"reply_end","sample_rate":24000}
//            {"type":"error","message":…}
//
// Turn-taking mirrors ui/command-center/src/hooks/useVoice.ts: push-to-talk or
// hands-free VAD (RMS thresholds copied from the web hook) and barge-in by
// reconnecting a FRESH socket — closing the old one sets the daemon handler's
// cancellation flag, so it stops synthesizing the reply the user talked over.

import SwiftUI
import AVFoundation

// ── Mic pipe: input-format buffers → 16 kHz mono Float32 frames ──────────────
// Lives on the audio tap thread only (serial), so the converter needs no lock.

private final class MicPipe: @unchecked Sendable {
    private let converter: AVAudioConverter
    private let outFormat: AVAudioFormat

    init?(from inputFormat: AVAudioFormat) {
        guard let out = AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false
        ), let conv = AVAudioConverter(from: inputFormat, to: out) else { return nil }
        converter = conv
        outFormat = out
    }

    /// Convert one tap buffer; returns (f32le bytes @16 kHz, RMS of the frame).
    func convert(_ buffer: AVAudioPCMBuffer) -> (Data, Float) {
        let ratio = outFormat.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 16
        guard let out = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: capacity) else {
            return (Data(), 0)
        }
        // One-shot feed box: the input block is @Sendable in the SDK, so the
        // single source buffer rides in an @unchecked Sendable box (the block
        // only ever runs synchronously inside convert()).
        final class Feed: @unchecked Sendable {
            var buffer: AVAudioPCMBuffer?
            init(_ b: AVAudioPCMBuffer) { buffer = b }
        }
        let feed = Feed(buffer)
        let status = converter.convert(to: out, error: nil) { _, outStatus in
            guard let b = feed.buffer else { outStatus.pointee = .noDataNow; return nil }
            feed.buffer = nil
            outStatus.pointee = .haveData
            return b
        }
        guard status != .error, let ch = out.floatChannelData, out.frameLength > 0 else {
            return (Data(), 0)
        }
        let n = Int(out.frameLength)
        var sum: Float = 0
        for i in 0..<n { sum += ch[0][i] * ch[0][i] }
        let rms = (sum / Float(n)).squareRoot()
        return (Data(bytes: ch[0], count: n * MemoryLayout<Float>.size), rms)
    }
}

// ── Engine: audio session + WS + turn state machine ──────────────────────────

@MainActor
final class VoiceEngine: ObservableObject {
    enum ConvState: Equatable {
        case idle
        case connecting
        case ready        // socket open, server said ready — waiting for a turn
        case listening    // capturing + streaming mic frames
        case thinking     // stop sent, awaiting STT + reply
        case speaking     // TTS audio queued/playing
        case failed(String)  // fatal (no pairing / mic denied / audio init)
    }

    @Published private(set) var state: ConvState = .idle
    @Published private(set) var transcript = ""
    @Published private(set) var reply = ""
    /// Transient server-side notice ("No speech detected…") — clears on recovery.
    @Published private(set) var notice: String?
    /// Live audio level 0…1 (mic while listening, TTS pulse while speaking).
    @Published private(set) var level: Float = 0
    @Published var handsFree = false { didSet { if !handsFree && state == .listening { endTurn() } } }

    // VAD thresholds — mirror useVoice.ts exactly.
    private static let vadOnset: Float = 0.015
    private static let vadKeepalive: Float = 0.010
    private static let vadBarge: Float = 0.05
    private static let vadSilenceMs: Double = 900
    private static let vadMaxTurnMs: Double = 45_000

    private var sessionId = ""
    private var active = false
    private var connectEpoch = 0
    private var wsTask: URLSessionWebSocketTask?

    private var engine: AVAudioEngine?
    private var playerNode: AVAudioPlayerNode?
    private let playbackFormat = AVAudioFormat(
        standardFormatWithSampleRate: 24_000, channels: 1
    )!
    private var pendingBuffers = 0
    private var replyEnded = false

    private var vadLastVoice: TimeInterval = 0
    private var vadHeardSpeech = false
    private var vadBargeStreak = 0
    private var vadTurnStart: TimeInterval = 0

    // ── Lifecycle ────────────────────────────────────────────────────────────

    func start(sessionId: String) async {
        guard !active else { return }
        self.sessionId = sessionId
        active = true

        guard await AVAudioApplication.requestRecordPermission() else {
            state = .failed("Microphone access is off for Permagent — enable it in Settings to talk with Henry.")
            active = false
            return
        }
        do {
            try startAudio()
        } catch {
            state = .failed("Couldn't start the microphone — close other audio apps and try again.")
            active = false
            return
        }
        await connect()
    }

    func stop() {
        active = false
        connectEpoch += 1
        wsTask?.cancel(with: .goingAway, reason: nil)
        wsTask = nil
        playerNode?.stop()
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        engine = nil
        playerNode = nil
        pendingBuffers = 0
        level = 0
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        state = .idle
    }

    // ── Audio graph ──────────────────────────────────────────────────────────

    private func startAudio() throws {
        let session = AVAudioSession.sharedInstance()
        // voiceChat mode = echo cancellation, so hands-free VAD doesn't trip on
        // Henry's own TTS (same reason the web hook requests echoCancellation).
        try session.setCategory(
            .playAndRecord, mode: .voiceChat, options: [.defaultToSpeaker, .allowBluetooth]
        )
        try session.setActive(true)

        let engine = AVAudioEngine()
        let player = AVAudioPlayerNode()
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: playbackFormat)

        let input = engine.inputNode
        let inFormat = input.inputFormat(forBus: 0)
        guard let pipe = MicPipe(from: inFormat) else {
            throw APIError.badStatus(0)
        }
        // ~85 ms per callback at 48 kHz — close to the web hook's ~128 ms VAD
        // window, so the copied thresholds behave comparably.
        input.installTap(onBus: 0, bufferSize: 4096, format: inFormat) { [weak self] buffer, _ in
            guard let self else { return }
            let (data, rms) = pipe.convert(buffer)
            guard !data.isEmpty else { return }
            Task { @MainActor in self.handleMicFrame(data, rms: rms) }
        }
        engine.prepare()
        try engine.start()
        self.engine = engine
        self.playerNode = player
    }

    private func handleMicFrame(_ data: Data, rms: Float) {
        if state == .listening {
            level = min(1, rms * 8)
            wsTask?.send(.data(data)) { _ in }
        } else {
            level = max(0, level * 0.9)
        }
        if handsFree { vadStep(rms: rms) }
    }

    // ── VAD (hands-free): thresholds and transitions mirror useVoice.ts ─────

    private func vadStep(rms: Float) {
        let now = Date().timeIntervalSince1970
        switch state {
        case .ready:
            vadBargeStreak = 0
            if rms > Self.vadOnset {
                vadHeardSpeech = true
                vadLastVoice = now
                vadTurnStart = now
                beginTurn()
            }
        case .listening:
            if rms > Self.vadKeepalive {
                vadHeardSpeech = true
                vadLastVoice = now
            }
            let silentMs = (now - vadLastVoice) * 1000
            let turnMs = (now - vadTurnStart) * 1000
            if (vadHeardSpeech && silentMs > Self.vadSilenceMs) || turnMs > Self.vadMaxTurnMs {
                vadHeardSpeech = false
                endTurn()
            }
        case .speaking, .thinking:
            // Barge-in demands a sustained loud signal (2 consecutive buffers
            // over the high bar) so residual TTS bleed can't cut Henry off —
            // and, like the web hook, only fires while actually speaking.
            if rms > Self.vadBarge {
                vadBargeStreak += 1
                if vadBargeStreak >= 2 && state == .speaking {
                    vadBargeStreak = 0
                    vadHeardSpeech = false
                    interrupt()
                }
            } else {
                vadBargeStreak = 0
            }
        default:
            break
        }
    }

    // ── Turns ────────────────────────────────────────────────────────────────

    func beginTurn() {
        guard state == .ready, wsTask != nil else { return }
        transcript = ""
        reply = ""
        notice = nil
        sendText(#"{"type":"start","sample_rate":16000}"#)
        state = .listening
    }

    func endTurn() {
        guard state == .listening else { return }
        sendText(#"{"type":"stop"}"#)
        level = 0
        state = .thinking
    }

    /// Barge-in: silence is instant (playback torn down first), then the turn is
    /// cancelled daemon-side by reconnecting a fresh socket — the close sets the
    /// handler's cancellation flag so it stops synthesizing further sentences.
    func interrupt() {
        guard state == .speaking || state == .thinking else { return }
        playerNode?.stop()
        pendingBuffers = 0
        replyEnded = false
        level = 0
        reconnect()
    }

    private func reconnect() {
        connectEpoch += 1
        wsTask?.cancel(with: .goingAway, reason: nil)
        wsTask = nil
        state = .connecting
        Task { await connect() }
    }

    // ── WebSocket ────────────────────────────────────────────────────────────

    private func connect() async {
        state = .connecting
        guard let config = await APIClient.shared.currentConfig(),
              var comps = URLComponents(url: config.baseURL, resolvingAgainstBaseURL: false)
        else {
            state = .failed("Not paired with a hub yet — pair from Settings → Devices on your Mac first.")
            active = false
            return
        }
        comps.scheme = comps.scheme == "https" ? "wss" : "ws"
        comps.path = "/voice"
        comps.queryItems = [
            URLQueryItem(name: "session_id", value: sessionId),
            URLQueryItem(name: "token", value: config.token),
        ]
        guard let url = comps.url else {
            state = .failed("Bad hub URL — re-pair with your hub.")
            active = false
            return
        }
        let epoch = connectEpoch
        let task = URLSession.shared.webSocketTask(with: url)
        wsTask = task
        replyEnded = false
        task.resume()
        receiveLoop(task, epoch: epoch)
    }

    private nonisolated func receiveLoop(_ task: URLSessionWebSocketTask, epoch: Int) {
        task.receive { [weak self] result in
            guard let self else { return }
            Task { @MainActor in
                guard epoch == self.connectEpoch, task === self.wsTask else { return }
                switch result {
                case .success(let message):
                    self.handleMessage(message)
                    self.receiveLoop(task, epoch: epoch)
                case .failure:
                    self.handleDisconnect()
                }
            }
        }
    }

    private func handleDisconnect() {
        wsTask = nil
        playerNode?.stop()
        pendingBuffers = 0
        guard active else { return }
        state = .connecting
        notice = "Reconnecting to your hub…"
        let epoch = connectEpoch
        Task {
            try? await Task.sleep(for: .seconds(2))
            guard self.active, epoch == self.connectEpoch else { return }
            self.notice = nil
            await self.connect()
        }
    }

    private struct ServerMsg: Decodable {
        let type: String
        let text: String?
        let message: String?
        let sample_rate: Int?
    }

    private func handleMessage(_ message: URLSessionWebSocketTask.Message) {
        switch message {
        case .string(let text):
            guard let data = text.data(using: .utf8),
                  let msg = try? JSONDecoder().decode(ServerMsg.self, from: data) else { return }
            switch msg.type {
            case "ready":
                state = .ready
            case "transcript":
                notice = nil
                transcript = msg.text ?? ""
            case "reply_start":
                if state == .thinking || state == .ready { state = .thinking }
            case "reply_text":
                reply = msg.text ?? ""
            case "reply_end":
                replyEnded = true
                if pendingBuffers == 0, state == .speaking || state == .thinking {
                    state = .ready
                }
            case "error":
                notice = msg.message ?? "Voice error"
                // Transient recovery on a live socket, like the web hook: return
                // to ready after a beat so a too-short press can't wedge a turn.
                if state == .thinking || state == .listening || state == .speaking {
                    let epoch = connectEpoch
                    Task {
                        try? await Task.sleep(for: .seconds(2))
                        guard epoch == self.connectEpoch, self.wsTask != nil else { return }
                        if self.state == .thinking || self.state == .listening {
                            self.notice = nil
                            self.state = .ready
                        }
                    }
                }
            default:
                break // "navigate" is desktop-only speak-then-act; nothing to do here
            }
        case .data(let data):
            playChunk(data)
        @unknown default:
            break
        }
    }

    private func sendText(_ text: String) {
        wsTask?.send(.string(text)) { _ in }
    }

    // ── Playback: schedule 24 kHz Float32 chunks in arrival order ────────────

    private func playChunk(_ data: Data) {
        let n = data.count / MemoryLayout<Float>.size
        guard n > 0, let player = playerNode,
              let buf = AVAudioPCMBuffer(pcmFormat: playbackFormat, frameCapacity: AVAudioFrameCount(n))
        else { return }
        buf.frameLength = AVAudioFrameCount(n)
        data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            memcpy(buf.floatChannelData![0], base, n * MemoryLayout<Float>.size)
        }
        // A pulse per sentence for the orb.
        var sum: Float = 0
        let ch = buf.floatChannelData![0]
        let stride = max(1, n / 512)
        var count = 0
        var i = 0
        while i < n { sum += ch[i] * ch[i]; count += 1; i += stride }
        level = min(1, (sum / Float(max(1, count))).squareRoot() * 6)

        pendingBuffers += 1
        state = .speaking
        player.scheduleBuffer(buf) { [weak self] in
            guard let self else { return }
            Task { @MainActor in self.bufferDrained() }
        }
        if !player.isPlaying { player.play() }
    }

    private func bufferDrained() {
        pendingBuffers = max(0, pendingBuffers - 1)
        if pendingBuffers == 0 {
            level = 0
            if replyEnded && state == .speaking { state = .ready }
        }
    }
}

// ── The organic blob orb (deliberately NOT a perfect circle) ─────────────────

private struct BlobShape: Shape {
    var phase: Double

    var animatableData: Double {
        get { phase }
        set { phase = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let c = CGPoint(x: rect.midX, y: rect.midY)
        let base = min(rect.width, rect.height) / 2 * 0.86
        let n = 96
        var points: [CGPoint] = []
        points.reserveCapacity(n)
        for i in 0..<n {
            let a = Double(i) / Double(n) * 2 * .pi
            // Irregular capsule: three incommensurate lobes drifting at
            // different speeds — asymmetric at every instant.
            let wobble = 0.055 * sin(3 * a + phase)
                + 0.042 * sin(5 * a - phase * 1.37)
                + 0.028 * sin(7 * a + phase * 0.71)
            let r = base * (1 + wobble)
            points.append(CGPoint(x: c.x + r * cos(a), y: c.y + r * sin(a)))
        }
        var p = Path()
        p.move(to: points[0])
        p.addLines(points)
        p.closeSubpath()
        return p
    }
}

private struct BlobOrb: View {
    let state: VoiceEngine.ConvState
    let level: Float
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var drift: Double {
        switch state {
        case .listening: return 1.6
        case .thinking: return 2.4
        case .speaking: return 2.0
        default: return 0.55   // calm when idle/ready
        }
    }

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: reduceMotion)) { ctx in
            let t = ctx.date.timeIntervalSinceReferenceDate
            let breathe = reduceMotion ? 0 : 0.02 * sin(t * 1.4)
            BlobShape(phase: reduceMotion ? 0 : t * drift)
                .fill(
                    LinearGradient(
                        colors: [Brand.cyan, Color(hex: 0x6366F1), Brand.violet],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .overlay(
                    BlobShape(phase: reduceMotion ? 0 : t * drift)
                        .stroke(Color.white.opacity(0.18), lineWidth: 1)
                )
                .scaleEffect(1 + CGFloat(level) * 0.12 + breathe)
                .shadow(color: Brand.cyanGlow, radius: 36)
                .animation(.linear(duration: 0.1), value: level)
        }
        .frame(width: 190, height: 190)
    }
}

// ── The screen ───────────────────────────────────────────────────────────────

struct VoiceView: View {
    let sessionId: String
    @Environment(\.dismiss) private var dismiss
    @StateObject private var engine = VoiceEngine()
    @State private var pressing = false

    var body: some View {
        ZStack {
            Brand.deepVoid.ignoresSafeArea()
            Brand.shell.ignoresSafeArea()

            VStack(spacing: 0) {
                header
                Spacer()

                BlobOrb(state: engine.state, level: engine.level)
                    .onTapGesture { engine.interrupt() }
                    .accessibilityLabel(orbAccessibility)
                    .accessibilityAddTraits(.isButton)

                Text(statusLine)
                    .font(.brandLabel)
                    .foregroundStyle(statusColor)
                    .padding(.top, 26)
                    .animation(Motion.ease, value: engine.state)

                conversationText
                    .padding(.top, 14)

                Spacer()
                controls
            }
            .padding(.horizontal, 24)
        }
        .task { await engine.start(sessionId: sessionId) }
        .onDisappear { engine.stop() }
        .preferredColorScheme(.dark)
    }

    // ── Chrome ───────────────────────────────────────────────────────────────

    private var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("VOICE").font(.brandLabel).foregroundStyle(Brand.cyan)
                Text("Henry").font(.brandTitle).foregroundStyle(Brand.text)
            }
            Spacer()
            Button {
                engine.stop()
                dismiss()
            } label: {
                Image(systemName: "xmark")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Brand.text)
                    .frame(width: 38, height: 38)
            }
            .glassChrome(in: Circle(), interactive: true)
            .accessibilityLabel("End voice conversation")
        }
        .padding(.top, 16)
    }

    private var conversationText: some View {
        VStack(spacing: 10) {
            if !engine.transcript.isEmpty {
                Text(engine.transcript)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.textMuted)
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
            }
            if !engine.reply.isEmpty {
                Text(engine.reply)
                    .font(.brandBody)
                    .foregroundStyle(Brand.text)
                    .multilineTextAlignment(.center)
                    .lineLimit(6)
            }
            if let notice = engine.notice {
                Text(notice)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.warning)
                    .multilineTextAlignment(.center)
            }
            if case .failed(let why) = engine.state {
                Text(why)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.danger)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(minHeight: 90, alignment: .top)
        .padding(.horizontal, 8)
        .animation(Motion.ease, value: engine.transcript)
        .animation(Motion.ease, value: engine.reply)
    }

    private var controls: some View {
        VStack(spacing: 18) {
            if !engine.handsFree {
                pushToTalkButton
            }
            Button {
                engine.handsFree.toggle()
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: engine.handsFree ? "waveform.badge.mic" : "hand.raised.fill")
                        .font(.caption.weight(.semibold))
                    Text(engine.handsFree ? "Hands-free on — Henry is listening" : "Go hands-free")
                        .font(.caption.weight(.semibold))
                }
                .foregroundStyle(engine.handsFree ? Brand.deepVoid : Brand.textMuted)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .background(engine.handsFree ? AnyShapeStyle(Brand.cyan) : AnyShapeStyle(Color.clear))
                .clipShape(Capsule())
            }
            .glassChrome(in: Capsule(), interactive: true)
            .disabled(!interactable)
            .animation(Motion.ease, value: engine.handsFree)
        }
        .padding(.bottom, 34)
    }

    private var pushToTalkButton: some View {
        ZStack {
            Circle()
                .fill(pressing ? AnyShapeStyle(Brand.ribbon) : AnyShapeStyle(Brand.surface))
                .frame(width: 84, height: 84)
                .overlay(Circle().strokeBorder(Brand.borderHi, lineWidth: 1))
                .shadow(color: Brand.cyanGlow, radius: pressing ? 24 : 10)
            Image(systemName: "mic.fill")
                .font(.system(size: 28, weight: .semibold))
                .foregroundStyle(pressing ? Brand.deepVoid : Brand.cyan)
        }
        .scaleEffect(pressing ? 1.08 : 1)
        .animation(Motion.spring, value: pressing)
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in
                    guard !pressing else { return }
                    pressing = true
                    engine.beginTurn()
                }
                .onEnded { _ in
                    pressing = false
                    engine.endTurn()
                }
        )
        .opacity(interactable ? 1 : 0.4)
        .allowsHitTesting(interactable)
        .accessibilityLabel("Hold to talk")
    }

    // ── Copy ─────────────────────────────────────────────────────────────────

    private var interactable: Bool {
        switch engine.state {
        case .idle, .connecting, .failed: return false
        default: return true
        }
    }

    private var statusLine: String {
        switch engine.state {
        case .idle: return "STARTING…"
        case .connecting: return "CONNECTING…"
        case .ready: return engine.handsFree ? "LISTENING FOR YOU" : "HOLD TO TALK"
        case .listening: return "LISTENING…"
        case .thinking: return "THINKING… TAP THE ORB TO CANCEL"
        case .speaking: return "SPEAKING — TAP THE ORB TO INTERRUPT"
        case .failed: return "VOICE UNAVAILABLE"
        }
    }

    private var statusColor: Color {
        switch engine.state {
        case .failed: return Brand.danger
        case .listening: return Brand.cyan
        case .speaking: return Brand.cyan
        default: return Brand.textMuted
        }
    }

    private var orbAccessibility: String {
        switch engine.state {
        case .speaking: return "Henry is speaking. Tap to interrupt."
        case .thinking: return "Henry is thinking. Tap to cancel."
        default: return "Henry"
        }
    }
}
