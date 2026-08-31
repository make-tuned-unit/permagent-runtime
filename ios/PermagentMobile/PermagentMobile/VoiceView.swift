// VoiceView — live conversation with the configured agent over the hub's /voice WebSocket.
//
// Wire protocol (crates/goose-server/src/routes/voice.rs):
//   connect  ws(s)://<hub>/voice?session_id=<id>&token=<bearer>&client=ios_voice
//            (query-param auth — the WS upgrade can't carry the Bearer header)
//   server → {"type":"ready"} once STT+TTS providers are loaded
//   client → {"type":"start","sample_rate":16000}
//            binary frames: raw Float32 LE mono PCM @ 16 kHz
//            {"type":"stop"}
//   server → {"type":"transcript","text":…}
//            {"type":"reply_start"}
//            binary frames: Float32 LE mono PCM @ 24 kHz (queued, played in order)
//            {"type":"clipboard","text":…}  copy on this device as soon as the
//            tool runs (often mid-turn, before confirmation audio). Write the
//            pasteboard immediately — the user may already be switching to
//            Notes, and iOS drops background writes.
//            {"type":"reply_text","text":…}
//            {"type":"navigate",…}       (desktop speak-then-act; ignored here)
//            {"type":"reply_end","sample_rate":24000}
//            {"type":"error","message":…}
//   Speaker print (N3 — gate, not a better ear):
//   server → {"type":"voice_print","enrolled":true|false}  after ready
//   client → {"type":"enroll_start"} then the usual start/pcm/stop per sentence
//            {"type":"enroll_done"} | {"type":"enroll_skip"} | {"type":"enroll_clear"}
//   server → {"type":"enroll_status","have":1,"need":3,"prompt":"…"}
//            {"type":"enrolled"} | {"type":"enroll_retry","reason":"…"}
//            {"type":"enroll_cleared"}
//   Rejected speech is idle, same as empty STT — no toast. Watch has no /voice
//   socket; desktop Command Center uses the same hub print and fails open.
//
// Turn-taking mirrors ui/command-center/src/hooks/useVoice.ts: push-to-talk or
// hands-free VAD (VoiceVAD.swift — thresholds retuned for iOS's quieter
// voiceChat-processed mic levels; see that file's header) and barge-in by
// reconnecting a FRESH socket — closing the old one sets the daemon handler's
// cancellation flag, so it stops synthesizing the reply the user talked over.

import SwiftUI
import AVFoundation
import UIKit

// ── Mic pipe: input-format buffers → 16 kHz mono Float32 frames ──────────────
// Lives on the audio tap thread only (serial), so the converter needs no lock.

private final class MicPipe: @unchecked Sendable {
    private let converter: AVAudioConverter
    private let outFormat: AVAudioFormat
    /// The format the converter was built for. A tap buffer in any OTHER format
    /// must be dropped, never fed in — see `convert`.
    private let inFormat: AVAudioFormat

    init?(from inputFormat: AVAudioFormat) {
        guard inputFormat.sampleRate > 0, inputFormat.channelCount > 0 else { return nil }
        guard let out = AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false
        ), let conv = AVAudioConverter(from: inputFormat, to: out) else { return nil }
        converter = conv
        outFormat = out
        inFormat = inputFormat
    }

    /// Convert one tap buffer; returns (f32le bytes @16 kHz, RMS, voice-like).
    func convert(_ buffer: AVAudioPCMBuffer) -> (Data, Float, Bool) {
        // A route change mid-session (AirPods connecting, a call arriving, the
        // speaker engaging) re-formats the input node while the tap keeps
        // delivering. Feeding an AVAudioConverter a buffer that does not match
        // the format it was constructed for raises an ObjC exception, which
        // Swift cannot catch — the app just dies. Dropping the buffer costs one
        // frame; the engine's restart path re-makes the pipe.
        guard buffer.format.sampleRate == inFormat.sampleRate,
              buffer.format.channelCount == inFormat.channelCount else {
            return (Data(), 0, true)
        }
        // `AVAudioFrameCount` is UInt32, and converting a NaN or infinite Double
        // to an integer type is a FATAL Swift trap, not a wrong number — the
        // same trap VoiceOrbView documents for `Int(NaN)`. A zero sample rate
        // makes `ratio` infinite, and a zero rate is exactly what a
        // deactivated or mid-reconfiguration session reports.
        let sourceRate = buffer.format.sampleRate
        guard sourceRate > 0 else { return (Data(), 0, true) }
        let scaled = Double(buffer.frameLength) * (outFormat.sampleRate / sourceRate)
        guard scaled.isFinite, scaled >= 0, scaled < Double(UInt32.max - 16) else {
            return (Data(), 0, true)
        }
        let capacity = AVAudioFrameCount(scaled) + 16
        guard let out = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: capacity) else {
            return (Data(), 0, true)
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
            return (Data(), 0, true)
        }
        let n = Int(out.frameLength)
        var sum: Float = 0
        for i in 0..<n { sum += ch[0][i] * ch[0][i] }
        let rms = (sum / Float(n)).squareRoot()
        let bins = VoiceSpectrum.byteBins(samples: ch[0], count: n)
        let voiceLike = VoiceSpectrum.looksLikeVoice(bins)
        return (Data(bytes: ch[0], count: n * MemoryLayout<Float>.size), rms, voiceLike)
    }
}

/// Why audio setup gave up. Each case exists because the corresponding
/// AVAudioEngine call would otherwise raise an uncatchable ObjC exception —
/// naming them turns an instant crash into a message on the orb screen, and
/// tells the next bug report exactly which format was bad.
enum AudioSetupError: LocalizedError {
    case unusableInputFormat(Double, UInt32)
    case converterUnavailable(Double)
    case unusableOutputFormat(Double)

    var errorDescription: String? {
        switch self {
        case let .unusableInputFormat(rate, channels):
            return "the microphone reported an unusable format (\(Int(rate)) Hz, \(channels) ch). Another app may hold the mic — close it and reopen voice."
        case let .converterUnavailable(rate):
            return "couldn't convert \(Int(rate)) Hz mic audio to 16 kHz."
        case let .unusableOutputFormat(rate):
            return "the audio output reported an unusable format (\(Int(rate)) Hz)."
        }
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
    /// Last paste-ready body from a `clipboard` frame — retappable if the
    /// automatic pasteboard write raced a switch to Notes.
    @Published private(set) var lastClipboard: String?
    /// Transient server-side notice (clipboard / real faults). Empty STT is idle, not this.
    @Published private(set) var notice: String?
    /// Word placed on the Orb for a listen-once pronunciation. Never spoken.
    @Published private(set) var teachWord: String?
    /// Hub learned speaker print is on disk.
    @Published private(set) var printEnrolled = false
    @Published private(set) var identityModelAvailable = false
    @Published private(set) var identityModelDownloading = false
    /// Collecting the three enrollment sentences. Stop must not look like a chat turn.
    @Published private(set) var enrolling = false
    /// Next sentence to put on the orb. Nil when not enrolling or after the third take.
    @Published private(set) var enrollPrompt: String?
    @Published private(set) var enrollHave = 0
    @Published private(set) var enrollNeed = VoiceEnroll.need
    /// Live audio level 0…1 (mic while listening, TTS pulse while speaking).
    @Published private(set) var level: Float = 0
    /// ON by default: tapping the voice icon should drop you into a
    /// conversation with the agent already listening, not into a screen you
    /// have to hold a button on to be heard. Push-to-talk is still one tap
    /// away. Turning it off mid-turn ends that turn rather than stranding it.
    @Published var handsFree = true { didSet { if !handsFree && state == .listening { endTurn() } } }

    /// Hands-free turn-taking. The state machine (thresholds, silence window,
    /// 60 s turn cap) lives in VoiceVAD.swift, pure and unit-tested — this
    /// engine only feeds it frames and executes the actions it returns.
    private var vad = VoiceVAD()
    /// A route change mid-turn parks the new preset here; it applies at the
    /// next non-listening frame so an open turn's clocks are never clobbered.
    private var pendingVADConfig: VoiceVAD.Config?
    private var routeObserver: NSObjectProtocol?
    private var routeApplyTask: Task<Void, Never>?
    /// True while taps are installed — `removeTap` throws if they are not.
    private var graphBuilt = false
    /// Voice processing is ON for headphones (AEC works) and OFF for the
    /// loudspeaker (AEC cancels the near-end mic — "Listening" heard nothing).
    private var captureUsesVoiceProcessing = false
    /// Raw playback RMS while the agent is speaking — gates barge-in on speaker
    /// so his own voice from the same speaker is not treated as an interrupt.
    private var playbackRms: Float = 0
    private var identityQuietGate = VoiceIdentityQuietGate()

    private var sessionId = ""
    private var active = false
    /// Set synchronously at the top of `start` — see the comment there.
    private var starting = false
    private var connectEpoch = 0
    private var wsTask: URLSessionWebSocketTask?

    private var engine: AVAudioEngine?
    private var playerNode: AVAudioPlayerNode?
    private let playbackFormat = AVAudioFormat(
        standardFormatWithSampleRate: 24_000, channels: 1
    )!
    private var pendingBuffers = 0
    private var replyEnded = false

    // ── Lifecycle ────────────────────────────────────────────────────────────

    /// Bring the conversation up. `provided` is the caller's already-resolved
    /// hub session; `nil` means resolve one here.
    ///
    /// Resolution lives INSIDE the engine on purpose. It used to happen at the
    /// tap site, which discarded the error with `try?` and presented the sheet
    /// regardless — so a hub that could not mint a session slid up a black
    /// screen with no orb and no explanation (reported 2026-08-04). Every way
    /// this can fail now lands in `.failed`, which the orb screen already
    /// renders. It also means the sheet opens INSTANTLY: no round trip runs
    /// before presentation, and no message has to be typed first to create the
    /// session — `chatSessionId()` mints one if there is none.
    func start(sessionId provided: String?) async {
        // `starting` closes a TOCTOU that `active` alone cannot: `active` is not
        // set until after the session round trip and the permission prompt, so
        // between the guard and the assignment there are two suspension points
        // during which a second `start` would sail past. Two starts means two
        // `installTap` calls on input bus 0, and AVAudioEngine raises
        // `NSInternalInconsistencyException` on the second — an ObjC exception
        // Swift cannot catch, i.e. an instant crash. Set synchronously, before
        // the first `await`, so the window does not exist.
        guard !active, !starting else { return }
        starting = true
        defer { starting = false }
        state = .connecting

        // Startup used to be strictly serial — session round trip, THEN the
        // permission prompt, THEN the audio engine, THEN the socket — so the
        // orb sat on "connecting" for the SUM of a network call and CoreAudio
        // bring-up. The two are independent: the mic does not need a session
        // id, and the session does not need a microphone. Run them together
        // and the wait becomes the slower of the two rather than the total.
        async let sessionTask: String? = {
            if let provided { return provided }
            return try? await MobileSession.chatSessionId()
        }()
        async let micGranted = AVAudioApplication.requestRecordPermission()

        // Audio first once permission lands: it is the piece that must be
        // live before the user speaks.
        guard await micGranted else {
            state = .failed("Microphone access is off for Permagent — enable it in Settings to talk with \(AgentIdentity.shared.name).")
            return
        }
        do {
            try startAudio()
        } catch {
            // Carry the reason through. The old blanket message sent Jesse
            // hunting a missing on-device voice model that does not exist —
            // STT/TTS both run on the hub, over the /voice socket.
            state = .failed("Couldn't start audio — \(error.localizedDescription)")
            return
        }

        guard let resolved = await sessionTask else {
            state = .failed("Couldn't open a conversation on your hub.")
            stop()
            return
        }
        self.sessionId = resolved
        active = true
        await connect()
    }

    func stop() {
        active = false
        connectEpoch += 1
        if let routeObserver {
            NotificationCenter.default.removeObserver(routeObserver)
            self.routeObserver = nil
        }
        wsTask?.cancel(with: .goingAway, reason: nil)
        wsTask = nil
        routeApplyTask?.cancel()
        routeApplyTask = nil
        teardownGraph()
        pendingBuffers = 0
        playbackRms = 0
        level = 0
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        state = .idle
        enrolling = false
        enrollPrompt = nil
        enrollHave = 0
        teachWord = nil
    }

    // ── Audio graph ──────────────────────────────────────────────────────────

    /// Session once, graph as needed. The 2026-08-21 rebuild called
    /// `startAudio()` (setCategory + new engine) on every route notification.
    /// That tore down a working tap, often while the input was still 0 Hz,
    /// swallowed the throw, and left the orb on LISTENING with no mic —
    /// speakerphone and headphones both, after weeks of the old path working.
    private func startAudio() throws {
        try configureSession()
        try buildGraph()
    }

    private func configureSession() throws {
        let session = AVAudioSession.sharedInstance()
        // `.default` is the path that worked for weeks. `.voiceChat` ducks TTS;
        // `.videoChat` adds session-level AEC that muted the near-end mic and
        // dropped HFP (tonight's hub log: three sockets, zero recordings).
        let bluetoothOption: AVAudioSession.CategoryOptions
        if #available(iOS 26.0, *) {
            bluetoothOption = .allowBluetoothHFP
        } else {
            bluetoothOption = .allowBluetooth
        }
        try session.setCategory(
            .playAndRecord, mode: VoiceAudioRoute.sessionMode, options: [.defaultToSpeaker, bluetoothOption]
        )
        try session.setActive(true)
        applySpeakerOverride()
        if session.isInputGainSettable {
            try? session.setInputGain(1.0)
        }
        applyVADConfigForCurrentRoute()
        if routeObserver == nil {
            routeObserver = NotificationCenter.default.addObserver(
                forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main
            ) { [weak self] _ in
                Task { @MainActor in self?.scheduleRouteApply() }
            }
        }
    }

    private func buildGraph() throws {
        let engine = AVAudioEngine()

        // ── Validate EVERY format before AVAudioEngine sees it ──────────────
        //
        // `connect(_:to:format:)` and `installTap` do not return errors on a bad
        // format — they raise an ObjC exception, and Swift cannot catch an
        // ObjC exception. The `do { try startAudio() } catch` around this call
        // therefore never fires; the process just dies. That is the shape of
        // the crash: the orb paints, `.task` runs this, and the app is gone a
        // frame later — on device only, because the simulator's audio stack
        // barely engages and always hands back a plausible format.
        //
        // So the order matters. Read the INPUT first: on real hardware it is
        // the one that comes back 0 Hz / 0 channels while the session is still
        // settling a route, and every call below would inherit that. Turning
        // each case into a thrown Swift error means the failure lands on the
        // orb screen with a reason instead of killing the app.
        let input = engine.inputNode
        // Headphones: voice processing AEC is safe (playback is in the ears).
        // Speakerphone: the same AEC treats the user's voice as echo and the
        // mic goes silent. Rebuild on route change so this tracks plug/unplug.
        let wantVP = VoiceAudioRoute.policy(
            outputPortTypes: AVAudioSession.sharedInstance().currentRoute.outputs.map(\.portType.rawValue)
        ).voiceProcessing
        try? input.setVoiceProcessingEnabled(wantVP)
        captureUsesVoiceProcessing = wantVP
        if wantVP {
            input.voiceProcessingOtherAudioDuckingConfiguration =
                .init(enableAdvancedDucking: false, duckingLevel: .min)
        }
        let inFormat = input.inputFormat(forBus: 0)
        guard inFormat.sampleRate > 0, inFormat.channelCount > 0 else {
            throw AudioSetupError.unusableInputFormat(inFormat.sampleRate,
                                                      inFormat.channelCount)
        }
        captureSampleRate = inFormat.sampleRate
        captureChannels = inFormat.channelCount
        guard let pipe = MicPipe(from: inFormat) else {
            throw AudioSetupError.converterUnavailable(inFormat.sampleRate)
        }
        // Touching `mainMixerNode` is itself what wires the mixer to the output
        // node, at the HARDWARE format — so it has to be sound before anything
        // is connected to it.
        let mixerFormat = engine.mainMixerNode.outputFormat(forBus: 0)
        guard mixerFormat.sampleRate > 0, mixerFormat.channelCount > 0 else {
            throw AudioSetupError.unusableOutputFormat(mixerFormat.sampleRate)
        }

        let player = AVAudioPlayerNode()
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: playbackFormat)
        // 1.0 is unity; anything above is a phone-speaker push on top of the
        // hub's -1 dBFS master. 1.15 ≈ +1.2 dB — audible on a handset, short
        // of clipping the mastered peaks.
        engine.mainMixerNode.outputVolume = 1.15
        // ~85 ms per callback at 48 kHz — close to the web hook's ~128 ms VAD
        // window, so the copied thresholds behave comparably.
        // `@Sendable` is LOAD-BEARING, not decoration.
        //
        // `AVAudioNodeTapBlock` is not marked @Sendable in the SDK, so a closure
        // written here inherits the isolation of its enclosing context — and
        // `startAudio()` is a method on a @MainActor class. Under Swift 6 the
        // compiler then plants a runtime executor check in the block, CoreAudio
        // invokes it from its realtime thread, and `dispatch_assert_queue` traps
        // with `brk #0x1`. That is an instant process kill: no catchable error,
        // no `.failed` state, and no crash report — the app simply vanishes the
        // moment the first audio buffer arrives, which is exactly the reported
        // "orb for a split second, then gone" (2026-08-04, confirmed under lldb:
        // _dispatch_assert_queue_fail ← _swift_task_checkIsolatedSwift ← this
        // closure ← AVAudioNodeTap::TapMessage::RealtimeMessenger_Perform).
        //
        // Marking it @Sendable opts the closure OUT of inherited isolation, so
        // no check is planted. Everything it touches is already safe to use off
        // the main actor: `pipe` is @unchecked Sendable and confined to this
        // thread, and the only main-actor work hops explicitly via Task.
        input.installTap(onBus: 0, bufferSize: 4096, format: inFormat) { @Sendable [weak self] buffer, _ in
            guard let self else { return }
            let (data, rms, voiceLike) = pipe.convert(buffer)
            guard !data.isEmpty else {
                Task { @MainActor in self.noteDroppedTap() }
                return
            }
            Task { @MainActor in self.handleMicFrame(data, rms: rms, voiceLike: voiceLike) }
        }
        // Playback tap — the orb's TRUTH while the agent speaks. The level used
        // to be pulsed once per delivered TTS chunk, but the daemon synthesizes
        // far faster than real time, so every chunk of a 30s answer lands in
        // the first ~10s: the orb animated to the DELIVERY schedule, not the
        // voice. Tapping the player node reads the audio actually being
        // rendered, so the orb moves with his syllables.
        //
        // @Sendable is load-bearing here for exactly the reason spelled out
        // above the input tap — this block runs on CoreAudio's realtime thread.
        player.installTap(onBus: 0, bufferSize: 1024, format: playbackFormat) {
            @Sendable [weak self] buffer, _ in
            guard let self, let ch = buffer.floatChannelData else { return }
            let n = Int(buffer.frameLength)
            guard n > 0 else { return }
            var sum: Float = 0
            let stride = max(1, n / 256)
            var count = 0
            var i = 0
            while i < n {
                sum += ch[0][i] * ch[0][i]
                count += 1
                i += stride
            }
            let rms = (sum / Float(max(1, count))).squareRoot()
            guard rms.isFinite else { return }
            // Hub peak-normalizes TTS to -1 dBFS, so a 2× visual gain maps
            // typical speech RMS onto the orb without slamming the ceiling.
            let lvl = min(1, rms * 2)
            Task { @MainActor in self.handlePlaybackLevel(rawRms: rms, orbLevel: lvl) }
        }
        engine.prepare()
        try engine.start()
        self.engine = engine
        self.playerNode = player
        graphBuilt = true
    }

    private func teardownGraph() {
        guard graphBuilt else { return }
        graphBuilt = false
        playerNode?.stop()
        engine?.inputNode.removeTap(onBus: 0)
        playerNode?.removeTap(onBus: 0)
        engine?.stop()
        engine = nil
        playerNode = nil
        captureUsesVoiceProcessing = false
    }

    /// Headphones / BT / CarPlay currently own the output.
    static func usingExternalOutput() -> Bool {
        VoiceAudioRoute.isExternalOutput(
            AVAudioSession.sharedInstance().currentRoute.outputs.map(\.portType.rawValue)
        )
    }

    /// Speaker only when nothing is plugged in. Forcing `.speaker` while
    /// AirPods are connected steals playback and can drop the headset mic.
    private func applySpeakerOverride() {
        let session = AVAudioSession.sharedInstance()
        switch VoiceAudioRoute.policy(
            outputPortTypes: session.currentRoute.outputs.map(\.portType.rawValue)
        ).outputOverride {
        case .speaker:
            try? session.overrideOutputAudioPort(.speaker)
        case .none:
            try? session.overrideOutputAudioPort(.none)
        }
    }

    private func scheduleRouteApply() {
        routeApplyTask?.cancel()
        routeApplyTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            // `graphBuilt` is enough: the first setCategory fires a route
            // change before `active` is set, and skipping that apply is how
            // a 0 Hz settle used to miss the working tap.
            guard graphBuilt || active else { return }
            applyCurrentRoute()
        }
    }

    /// Plug/unplug: retarget speaker vs headset, swap VAD, rebuild the graph
    /// when the input format or AEC setting has to change (MicPipe dies
    /// otherwise — it drops mismatched buffers and the orb never hears you).
    /// Does NOT call `setCategory` again — that retriggers this path and
    /// was tonight's capture death spiral.
    private func applyCurrentRoute() {
        applySpeakerOverride()
        applyVADConfigForCurrentRoute()
        guard !rebuildingGraph else { return }
        if !graphBuilt {
            try? buildGraph()
            return
        }
        guard let engine else { return }
        let wantVP = VoiceAudioRoute.policy(
            outputPortTypes: AVAudioSession.sharedInstance().currentRoute.outputs.map(\.portType.rawValue)
        ).voiceProcessing
        let fmt = engine.inputNode.inputFormat(forBus: 0)
        if fmt.sampleRate <= 0 || fmt.channelCount == 0 {
            scheduleRouteApply()
            return
        }
        if VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: captureUsesVoiceProcessing,
            wantVoiceProcessing: wantVP,
            captureSampleRate: captureSampleRate,
            captureChannels: captureChannels,
            liveSampleRate: fmt.sampleRate,
            liveChannels: fmt.channelCount
        ) {
            try? rebuildGraphPreservingPlayback()
        }
    }

    private var rebuildingGraph = false
    private var captureSampleRate: Double = 0
    private var captureChannels: AVAudioChannelCount = 0
    /// Consecutive empty tap buffers (format mismatch). Used to surface a
    /// dead mic instead of sitting on LISTENING forever.
    private var droppedTapStreak = 0

    private func rebuildGraphPreservingPlayback() throws {
        guard !rebuildingGraph else { return }
        rebuildingGraph = true
        defer { rebuildingGraph = false }
        teardownGraph()
        try buildGraph()
    }

    private func noteDroppedTap() {
        droppedTapStreak += 1
        if droppedTapStreak == 25, state == .ready || state == .listening {
            notice = "Microphone went quiet — recovering the route."
            scheduleRouteApply()
        }
    }

    /// Publish a playback level for the orb. Ignored unless the agent is
    /// actually speaking, so a trailing tap callback can't light the orb after
    /// a barge-in.
    private func handlePlaybackLevel(rawRms: Float, orbLevel: Float) {
        playbackRms = rawRms
        guard state == .speaking else { return }
        level = orbLevel
    }

    /// Rolling pre-roll of the most recent mic frames, kept while NOT
    /// streaming. Speech is only detected once it crosses the VAD's onset
    /// threshold — by which point the first syllable is already spoken and,
    /// before this buffer, thrown away ("it misses the first words I say").
    /// When a turn opens these frames are flushed ahead of the live audio, so
    /// the hub transcribes the word from its actual beginning.
    private var preRoll: [Data] = []
    /// ~85 ms per frame; 6 frames ≈ half a second of lead-in, which covers a
    /// word begun before onset without adding meaningful latency.
    private static let preRollFrames = 6

    /// Swap the VAD to the preset for whatever is currently capturing. Applied
    /// immediately unless a turn is open — then it parks until the turn ends,
    /// so the endpoint clocks are never reset mid-utterance.
    private func applyVADConfigForCurrentRoute() {
        let ports = AVAudioSession.sharedInstance().currentRoute.inputs.map(\.portType.rawValue)
        let cfg = VoiceVAD.configForRoute(
            inputPortTypes: ports,
            speakerphone: !Self.usingExternalOutput()
        )
        if state == .listening {
            pendingVADConfig = cfg
        } else {
            vad = VoiceVAD(config: cfg)
            pendingVADConfig = nil
        }
    }

    private func handleMicFrame(_ data: Data, rms: Float, voiceLike: Bool) {
        droppedTapStreak = 0
        if let cfg = pendingVADConfig, state != .listening {
            vad = VoiceVAD(config: cfg)
            pendingVADConfig = nil
        }
        if state == .listening || state == .ready {
            level = min(1, rms * 12)
        }
        if state == .listening {
            wsTask?.send(.data(data)) { _ in }
        } else if state == .ready {
            preRoll.append(data)
            if preRoll.count > Self.preRollFrames { preRoll.removeFirst() }
        }
        if state != .listening && state != .speaking && state != .ready {
            if level > 0.001 { level = max(0, level * 0.9) }
        }
        if identityQuietGate.observe(rms: rms) {
            // Keep the pre-roll empty too: once quiet arrives, the next turn
            // must not begin with the rejected background speaker's tail.
            preRoll.removeAll(keepingCapacity: true)
            return
        }
        if VoiceEnroll.shouldDriveVAD(
            handsFree: handsFree,
            enrolling: enrolling,
            isListening: state == .listening
        ) {
            vadStep(rms: rms, voiceLike: voiceLike)
        }
    }

    // ── VAD (hands-free): the state machine itself is VoiceVAD, unit-tested ──

    private func vadStep(rms: Float, voiceLike: Bool) {
        let phase: VoiceVAD.Phase
        switch state {
        case .ready: phase = .ready
        case .listening: phase = .listening
        case .thinking: phase = .thinking
        case .speaking: phase = .speaking
        default: phase = .inactive
        }
        switch vad.step(rms: rms, phase: phase, now: Date().timeIntervalSince1970, voiceLike: voiceLike) {
        case .beginTurn: enterListening()  // NOT beginTurn(): the VAD stamped its own clocks
        case .endTurn: endTurn()
        case .interrupt:
            // Don't barge-in the teach prompt. Kitchen speakerphone hears
            // ASK_FIRST and would reconnect, dropping the word off the orb
            // (2026-08-27 11:14:29 teach → 11:14:34 close 1001).
            if teachWord != nil { break }
            // Speakerphone without AEC: his TTS comes out the same speaker the
            // mic hears. Ignore barge while playback is actually coming out.
            if VoiceAudioRoute.ignoreBargeIn(
                speakerphone: !Self.usingExternalOutput(),
                playbackRms: playbackRms,
                micRms: rms
            ) { break }
            interrupt()
        case .none: break
        }
    }

    // ── Turns ────────────────────────────────────────────────────────────────

    func recopyClipboard() {
        guard let body = lastClipboard, !body.isEmpty else { return }
        if VoiceClipboard.write(body) {
            notice = "Copied — paste into Notes"
        } else {
            notice = "Couldn't copy — stay in Permagent and try again"
        }
    }

    /// Begin a turn from OUTSIDE the VAD (the push-to-talk button). Stamps the
    /// VAD's turn clocks so the max-turn cap measures from now — a turn begun
    /// here used to inherit the previous hands-free turn's epoch, and the cap
    /// could end it on the first frame after hands-free came back on.
    func beginTurn() {
        guard state == .ready, wsTask != nil else { return }
        vad.noteTurnBegan(at: Date().timeIntervalSince1970)
        enterListening()
    }

    /// Send `start` and switch to `.listening`. The VAD's clocks are already
    /// stamped, by whichever path got here.
    private func enterListening() {
        guard state == .ready, wsTask != nil else { return }
        transcript = ""
        reply = ""
        notice = nil
        sendText(#"{"type":"start","sample_rate":16000}"#)
        // Flush the lead-in BEFORE any live frame, so the hub hears the word
        // from its beginning rather than from the moment it got loud enough
        // to notice. Ordering matters: these are older than everything the
        // tap will deliver next.
        let lead = preRoll
        preRoll.removeAll(keepingCapacity: true)
        for frame in lead {
            wsTask?.send(.data(frame)) { _ in }
        }
        state = .listening
    }

    func endTurn() {
        guard state == .listening else { return }
        vad.noteTurnEnded()
        // The stop send used to discard its error: a failed send left the
        // daemon recording forever and this client in .thinking with no exit.
        wsTask?.send(.string(#"{"type":"stop"}"#)) { [weak self] error in
            guard error != nil else { return }
            Task { @MainActor [weak self] in
                guard let self, self.state == .thinking else { return }
                self.notice = "Connection hiccup — try that again."
                self.state = .ready
            }
        }
        level = 0
        state = .thinking
        armThinkingWatchdog()
    }

    /// `.thinking` has no natural exit if the daemon parks (tool approval
    /// waits indefinitely) or an early server return skips reply_end — the
    /// orb sat in THINKING forever with the mic dead. Epoch-guarded: any real
    /// transition (first audio chunk, reply_end, error, disconnect) advances
    /// state, and the stale watchdog fires harmlessly against the guard.
    private var thinkingEpoch = 0
    private func armThinkingWatchdog() {
        thinkingEpoch += 1
        let epoch = thinkingEpoch
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 30_000_000_000)
            guard let self, self.thinkingEpoch == epoch, self.state == .thinking else { return }
            self.notice = "Still working — if I asked for approval, tap Approve below or say yes."
            self.state = .ready
        }
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
            URLQueryItem(name: "client", value: "ios_voice"),
        ]
        guard let url = comps.url else {
            state = .failed("Bad hub URL — re-pair with your hub.")
            active = false
            return
        }
        let epoch = connectEpoch
        let task = URLSession.shared.webSocketTask(with: url)
        task.maximumMessageSize = VoiceTransport.maximumIncomingMessageBytes
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
        let word: String?
        let have: Int?
        let need: Int?
        let prompt: String?
        let enrolled: Bool?
        let available: Bool?
        let downloading: Bool?
        let reason: String?
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
            case "clipboard":
                if let body = msg.text, !body.isEmpty {
                    lastClipboard = body
                    if VoiceClipboard.write(body) {
                        notice = "Copied — paste into Notes"
                    } else {
                        notice = "Couldn't copy — stay in Permagent and try again"
                    }
                }
            case "teach":
                if let word = msg.word, !word.isEmpty {
                    teachWord = word
                    // Reconnect / replay: the word is already parked — open
                    // the mic. First-time teach arrives before ASK_FIRST audio;
                    // stay put so the prompt can play, then finishSpeaking listens.
                    if state == .ready {
                        enterListening()
                    }
                }
            case "taught":
                teachWord = nil
            case "voice_print":
                printEnrolled = msg.enrolled ?? false
                identityModelAvailable = msg.available ?? false
                identityModelDownloading = msg.downloading ?? false
                if !printEnrolled && !enrolling {
                    enrollPrompt = nil
                }
            case "enroll_status":
                let have = msg.have ?? 0
                let need = msg.need ?? VoiceEnroll.need
                enrollHave = have
                enrollNeed = need
                enrollPrompt = msg.prompt
                enrolling = true
                if have >= need {
                    enrolling = false
                    enrollPrompt = nil
                    sendText(#"{"type":"enroll_done"}"#)
                } else if state == .ready || state == .thinking {
                    // The hub has confirmed the learned model is loaded and
                    // enrollment mode is active. Only now open the mic; doing
                    // it optimistically could send the setup sentence through
                    // STT as a normal agent turn while the model downloads.
                    //
                    // A successful take's enroll_status arrives while this
                    // client is still `.thinking` — waiting for the trailing
                    // Idle to open the next take costs a round trip, so this
                    // frame is the primary trigger and `idle` is the fallback.
                    state = .ready
                    beginEnrollmentTake()
                }
            case "enrolled":
                enrolling = false
                enrollPrompt = nil
                printEnrolled = true
                enrollHave = enrollNeed
                notice = "Voice saved — I'll ignore other talkers"
            case "enroll_retry":
                notice = msg.reason ?? "Say the sentence on the orb again"
            case "enroll_cleared":
                enrolling = false
                enrollPrompt = nil
                printEnrolled = false
                enrollHave = 0
            case "speaker_rejected":
                // The server's learned identity check rejected this capture.
                // Require actual quiet before hands-free re-arms or the same
                // ambient talker immediately opens another recording.
                vad.noteTurnEnded()
                identityQuietGate.lock()
                preRoll.removeAll(keepingCapacity: true)
                level = 0
                notice = nil
                if state == .thinking || state == .listening || state == .speaking {
                    state = .ready
                }
            case "reply_end":
                replyEnded = true
                if pendingBuffers == 0, state == .speaking || state == .thinking {
                    finishSpeaking()
                }
            case "idle":
                // Empty / too-short capture — back to ready, no toast.
                // Rejected speaker-print is the same path. Enrollment idle
                // opens the next sentence; pronunciation teach still wins.
                if enrolling, enrollPrompt != nil {
                    // A successful take sends enroll_status followed by the
                    // prior take's Idle. enroll_status may already have opened
                    // the next take, so never reset a live recording or send a
                    // duplicate Start here. A retry has no status frame; in
                    // that case Thinking -> Ready is the signal to re-open.
                    if state == .thinking {
                        state = .ready
                        beginEnrollmentTake()
                    }
                } else if teachWord != nil {
                    notice = nil
                    state = .ready
                    enterListening()
                } else if state == .thinking || state == .listening || state == .speaking {
                    notice = nil
                    state = .ready
                }
            case "error":
                if VoiceIdle.isTransientEmptyTurn(msg.message) {
                    notice = nil
                    if state == .thinking || state == .listening || state == .speaking {
                        state = .ready
                    }
                    break
                }
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
        // `floatChannelData` is nil whenever the buffer is not float32
        // non-interleaved — a route change (AirPods connecting, a call
        // arriving) can hand back a different format mid-session. Force
        // unwrapping it crashes the app in the middle of the agent speaking;
        // dropping one buffer is the correct trade.
        guard let channels = buf.floatChannelData else { return }
        data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            memcpy(channels[0], base, n * MemoryLayout<Float>.size)
        }
        // No per-chunk level pulse here: chunks arrive far ahead of playback,
        // so they are the wrong clock for the orb. The player tap installed in
        // startAudio() drives `level` from the audio actually being heard.
        pendingBuffers += 1
        state = .speaking
        // Same inherited-isolation trap as the mic tap above: this completion
        // runs on an audio thread, and without @Sendable it would carry a
        // main-actor check that traps the moment a TTS buffer finishes.
        player.scheduleBuffer(buf) { @Sendable [weak self] in
            guard let self else { return }
            Task { @MainActor in self.bufferDrained() }
        }
        if !player.isPlaying { player.play() }
    }

    private func bufferDrained() {
        pendingBuffers = max(0, pendingBuffers - 1)
        if pendingBuffers == 0 {
            level = 0
            playbackRms = 0
            if replyEnded && state == .speaking {
                finishSpeaking()
            }
        }
    }

    /// After his line finishes: if a word is on the Orb, open the mic.
    private func finishSpeaking() {
        state = .ready
        if teachWord != nil {
            enterListening()
        } else if enrolling, enrollPrompt != nil {
            beginEnrollmentTake()
        }
    }

    /// Open the next enrollment take. The hub's status frames — not the VAD's
    /// onset detector — decide when a take begins here, so the turn clocks
    /// must be stamped on the way in exactly as push-to-talk stamps them.
    /// Without that, the max-turn cap measures from a stale (on the setup
    /// screen, never-set) epoch and endpoints the take on its first frame.
    private func beginEnrollmentTake() {
        guard enrolling, enrollPrompt != nil, state == .ready else { return }
        VoiceEnroll.openTake(&vad, now: Date().timeIntervalSince1970)
        enterListening()
    }

    func beginEnroll() {
        guard teachWord == nil, wsTask != nil else { return }
        // enroll_start on the hub drops any in-flight recording so a leftover
        // Stop cannot STT the kitchen into a chat turn.
        if state == .listening {
            vad.noteTurnEnded()
            level = 0
            state = .ready
        }
        enrolling = true
        enrollHave = 0
        enrollNeed = VoiceEnroll.need
        enrollPrompt = VoiceEnroll.prompt(have: 0)
        notice = nil
        sendText(#"{"type":"enroll_start"}"#)
    }

    func skipEnroll() {
        enrolling = false
        enrollPrompt = nil
        enrollHave = 0
        if state == .listening {
            vad.noteTurnEnded()
            level = 0
            state = .ready
        }
        sendText(#"{"type":"enroll_skip"}"#)
    }

    func clearEnroll() {
        enrolling = false
        enrollPrompt = nil
        enrollHave = 0
        if state == .listening {
            vad.noteTurnEnded()
            level = 0
            state = .ready
        }
        sendText(#"{"type":"enroll_clear"}"#)
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
    /// An already-resolved hub session, when the caller has one. `nil` means
    /// "resolve it yourself" — see `VoiceEngine.start(sessionId:)`.
    var sessionId: String? = nil
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

                // The particle sphere, shared with the desktop conversation
                // view (VoiceOrbView ← VoiceOrb.tsx). Replaces the old
                // breathing-rings BlobOrb, which made the same product read
                // as two different apps mid-conversation.
                VoiceOrbView(
                    level: Double(engine.level),
                    speaking: engine.state == .speaking,
                    thinking: engine.state == .thinking,
                    listening: engine.state == .listening
                        || (engine.state == .ready && engine.handsFree)
                        || engine.teachWord != nil,
                    teachWord: engine.teachWord
                )
                    .onTapGesture {
                        if engine.teachWord != nil { return }
                        engine.interrupt()
                    }
                    .accessibilityLabel(orbAccessibility)
                    .accessibilityAddTraits(.isButton)

                Text(statusLine)
                    .font(.brandLabel)
                    .foregroundStyle(statusColor)
                    .padding(.top, 26)
                    .animation(Motion.ease, value: engine.state)

                conversationText
                    .padding(.top, 14)

                ChatDecisionStrip()
                    .padding(.top, 12)

                Spacer()
                controls
            }
            .padding(.horizontal, 24)
        }
        .task { await engine.start(sessionId: sessionId) }
        .onDisappear { engine.stop() }
    }

    // ── Chrome ───────────────────────────────────────────────────────────────

    private var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("VOICE").font(.brandLabel).foregroundStyle(Brand.cyanInk)
                Text(AgentIdentity.shared.displayName).font(.brandTitle).foregroundStyle(Brand.text)
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
            if let word = engine.teachWord {
                Text(word)
                    .font(.brandTitle)
                    .foregroundStyle(Brand.text)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .background(Brand.surfaceHi, in: Capsule())
                    .accessibilityLabel("Say \(word)")
            }
            if !engine.transcript.isEmpty {
                Text(engine.transcript)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.textMuted)
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
            }
            if !engine.reply.isEmpty {
                ScrollView {
                    Text(engine.reply)
                        .font(.brandBody)
                        .foregroundStyle(Brand.text)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: .infinity)
                }
                .frame(maxHeight: 220)
            }
            if let notice = engine.notice {
                Text(notice)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.warning)
                    .multilineTextAlignment(.center)
            }
            if engine.lastClipboard != nil {
                Button("Copy again") {
                    engine.recopyClipboard()
                }
                .font(.caption.weight(.semibold))
                .foregroundStyle(Brand.cyanInk)
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
                    Text(engine.handsFree ? "Hands-free on — \(AgentIdentity.shared.nameCapitalized) is listening" : "Go hands-free")
                        .font(.caption.weight(.semibold))
                }
                .foregroundStyle(engine.handsFree ? Brand.onAccent : Brand.textMuted)
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
                .foregroundStyle(pressing ? Brand.onAccent : Brand.cyan)
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
        if engine.teachWord != nil {
            switch engine.state {
            case .speaking: return "PLACING A WORD ON THE ORB"
            case .listening, .ready: return "SAY THE WORD ON THE ORB"
            default: break
            }
        }
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
        if let word = engine.teachWord {
            return "Say \(word). \(AgentIdentity.shared.nameCapitalized) is listening for the pronunciation."
        }
        switch engine.state {
        case .speaking: return "\(AgentIdentity.shared.nameCapitalized) is speaking. Tap to interrupt."
        case .thinking: return "\(AgentIdentity.shared.nameCapitalized) is thinking. Tap to cancel."
        default: return AgentIdentity.shared.displayName
        }
    }
}
